//! A thin async HTTP client shared by extractors (page/API fetches) and the
//! downloader (ranged byte streaming). Wraps `reqwest` with sensible defaults:
//! a browser-like user agent, cookie jar, gzip/brotli, and HTTP/2.

use std::sync::Arc;
use std::time::Duration;

use reqwest::header::{HeaderMap, HeaderName, HeaderValue};
use serde::de::DeserializeOwned;
use serde::Serialize;

/// A reasonably current desktop Chrome UA. Many sites (YouTube included) tailor
/// responses to the UA, so this is part of the contract, not cosmetic.
pub const DEFAULT_USER_AGENT: &str =
    "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 \
     (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36";

#[derive(Clone)]
pub struct HttpClient {
    inner: reqwest::Client,
}

impl HttpClient {
    pub fn new() -> anyhow::Result<Self> {
        Self::builder().build()
    }

    pub fn builder() -> HttpClientBuilder {
        HttpClientBuilder::default()
    }

    pub fn raw(&self) -> &reqwest::Client {
        &self.inner
    }

    /// GET a URL and return the body as text.
    pub async fn get_text(&self, url: &str) -> reqwest::Result<String> {
        self.inner.get(url).send().await?.error_for_status()?.text().await
    }

    /// GET a URL with extra headers and return the body as text.
    pub async fn get_text_with(
        &self,
        url: &str,
        headers: HeaderMap,
    ) -> reqwest::Result<String> {
        self.inner
            .get(url)
            .headers(headers)
            .send()
            .await?
            .error_for_status()?
            .text()
            .await
    }

    /// POST JSON and deserialize the JSON response.
    pub async fn post_json<B: Serialize, R: DeserializeOwned>(
        &self,
        url: &str,
        body: &B,
        headers: HeaderMap,
    ) -> reqwest::Result<R> {
        self.inner
            .post(url)
            .headers(headers)
            .json(body)
            .send()
            .await?
            .error_for_status()?
            .json::<R>()
            .await
    }

    /// Issue a ranged GET, returning the streaming response for the downloader.
    pub async fn get_range(
        &self,
        url: &str,
        start: u64,
        end: Option<u64>,
        headers: &HeaderMap,
    ) -> reqwest::Result<reqwest::Response> {
        let range = match end {
            Some(e) => format!("bytes={start}-{e}"),
            None => format!("bytes={start}-"),
        };
        self.inner
            .get(url)
            .headers(headers.clone())
            .header(reqwest::header::RANGE, range)
            .send()
            .await?
            .error_for_status()
    }

    /// HEAD-like probe via a 0-0 range request to learn total size.
    pub async fn content_length(&self, url: &str, headers: &HeaderMap) -> Option<u64> {
        let resp = self.inner.get(url).headers(headers.clone()).send().await.ok()?;
        resp.content_length()
    }
}

#[derive(Default)]
pub struct HttpClientBuilder {
    user_agent: Option<String>,
    proxy: Option<String>,
    default_headers: Vec<(String, String)>,
    cookie_jar: Option<Arc<reqwest::cookie::Jar>>,
}

impl HttpClientBuilder {
    pub fn user_agent(mut self, ua: impl Into<String>) -> Self {
        self.user_agent = Some(ua.into());
        self
    }

    pub fn proxy(mut self, proxy: Option<String>) -> Self {
        self.proxy = proxy;
        self
    }

    pub fn header(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        self.default_headers.push((name.into(), value.into()));
        self
    }

    /// Pre-load a cookie jar (e.g. from `--cookies`); received cookies are also
    /// stored in it for the session.
    pub fn cookie_jar(mut self, jar: Arc<reqwest::cookie::Jar>) -> Self {
        self.cookie_jar = Some(jar);
        self
    }

    pub fn build(self) -> anyhow::Result<HttpClient> {
        let mut headers = HeaderMap::new();
        for (k, v) in &self.default_headers {
            let name = HeaderName::from_bytes(k.as_bytes())?;
            headers.insert(name, HeaderValue::from_str(v)?);
        }

        let mut builder = reqwest::Client::builder()
            .user_agent(self.user_agent.unwrap_or_else(|| DEFAULT_USER_AGENT.to_string()))
            .default_headers(headers)
            .timeout(Duration::from_secs(60))
            .connect_timeout(Duration::from_secs(20));

        // A provided jar both supplies preloaded cookies and stores new ones;
        // otherwise use reqwest's default in-memory store.
        match self.cookie_jar {
            Some(jar) => builder = builder.cookie_provider(jar),
            None => builder = builder.cookie_store(true),
        }

        // Proxy precedence:
        //   Some(non-empty) -> route all traffic through it (http/https/socks5).
        //   Some("")        -> explicitly disable, ignoring HTTP(S)_PROXY env.
        //   None            -> reqwest default (honours HTTP(S)_PROXY env).
        match self.proxy.as_deref() {
            Some("") => builder = builder.no_proxy(),
            Some(proxy) => {
                builder = builder.proxy(
                    reqwest::Proxy::all(proxy)
                        .map_err(|e| anyhow::anyhow!("invalid proxy '{proxy}': {e}"))?,
                )
            }
            None => {}
        }

        Ok(HttpClient { inner: builder.build()? })
    }
}

/// Shared handle type used throughout the pipeline.
pub type SharedHttp = Arc<HttpClient>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_with_http_proxy() {
        assert!(HttpClient::builder().proxy(Some("http://127.0.0.1:8080".into())).build().is_ok());
    }

    #[test]
    fn builds_with_socks5_proxy() {
        // Fails to build unless reqwest's `socks` feature is enabled.
        assert!(HttpClient::builder().proxy(Some("socks5://127.0.0.1:1080".into())).build().is_ok());
    }

    #[test]
    fn empty_proxy_disables_and_builds() {
        assert!(HttpClient::builder().proxy(Some(String::new())).build().is_ok());
    }

    #[test]
    fn invalid_proxy_errors() {
        let err = HttpClient::builder().proxy(Some("http://bad host with spaces".into())).build();
        assert!(err.is_err());
    }
}
