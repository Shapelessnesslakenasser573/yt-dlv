//! Netscape `cookies.txt` parsing — the same format yt-dlp's `--cookies` reads.
//! Parsed cookies are loaded into a `reqwest` cookie jar so authenticated
//! requests (which sidestep much of YouTube's bot-flagging) just work.

use std::path::Path;
use std::sync::Arc;

use anyhow::{Context, Result};
use reqwest::cookie::Jar;

/// One entry from a Netscape-format cookie jar.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NetscapeCookie {
    pub domain: String,
    pub include_subdomains: bool,
    pub path: String,
    pub secure: bool,
    /// Unix expiry; `None` for session cookies (field value `0`).
    pub expires: Option<i64>,
    pub name: String,
    pub value: String,
}

/// Parse the Netscape cookie-file format: tab-separated
/// `domain  includeSubdomains  path  secure  expiry  name  value`.
/// Comment lines (`#`) are ignored, except the `#HttpOnly_` prefix.
pub fn parse_netscape(content: &str) -> Vec<NetscapeCookie> {
    let mut out = Vec::new();
    for raw in content.lines() {
        let line = raw.trim_end_matches(['\r', '\n']);
        if line.trim().is_empty() {
            continue;
        }
        let line = if let Some(rest) = line.strip_prefix("#HttpOnly_") {
            rest
        } else if line.starts_with('#') {
            continue;
        } else {
            line
        };

        // Spec is tab-separated; fall back to whitespace for hand-edited files.
        let mut fields: Vec<&str> = line.split('\t').collect();
        if fields.len() < 7 {
            fields = line.split_whitespace().collect();
        }
        if fields.len() < 7 {
            continue;
        }

        out.push(NetscapeCookie {
            domain: fields[0].to_string(),
            include_subdomains: fields[1].eq_ignore_ascii_case("true"),
            path: fields[2].to_string(),
            secure: fields[3].eq_ignore_ascii_case("true"),
            expires: fields[4].parse::<i64>().ok().filter(|&e| e != 0),
            name: fields[5].to_string(),
            value: fields[6].to_string(),
        });
    }
    out
}

/// Build a `reqwest` cookie jar from Netscape cookie-file content.
pub fn jar_from_netscape(content: &str) -> Arc<Jar> {
    let jar = Jar::default();
    for c in parse_netscape(content) {
        let host = c.domain.trim_start_matches('.');
        let scheme = if c.secure { "https" } else { "http" };
        let Ok(url) = format!("{scheme}://{host}{}", c.path).parse::<reqwest::Url>() else {
            continue;
        };
        let mut cookie = format!("{}={}; Domain={}; Path={}", c.name, c.value, c.domain, c.path);
        if c.secure {
            cookie.push_str("; Secure");
        }
        jar.add_cookie_str(&cookie, &url);
    }
    Arc::new(jar)
}

/// Load a Netscape cookie file into a jar.
pub fn load_cookie_file(path: &Path) -> Result<Arc<Jar>> {
    let content = std::fs::read_to_string(path)
        .with_context(|| format!("reading cookies file {}", path.display()))?;
    Ok(jar_from_netscape(&content))
}

#[cfg(test)]
mod tests {
    use super::*;
    use reqwest::cookie::CookieStore;

    const SAMPLE: &str = "\
# Netscape HTTP Cookie File
# This is a comment
.youtube.com\tTRUE\t/\tTRUE\t1999999999\tSID\tabc123
#HttpOnly_.youtube.com\tTRUE\t/\tTRUE\t0\tHSID\tsess_only
www.youtube.com\tFALSE\t/\tFALSE\t1999999999\tPREF\tf1=40000000
";

    #[test]
    fn parses_entries_including_httponly_and_session() {
        let cookies = parse_netscape(SAMPLE);
        assert_eq!(cookies.len(), 3);

        assert_eq!(cookies[0].name, "SID");
        assert_eq!(cookies[0].value, "abc123");
        assert!(cookies[0].include_subdomains);
        assert!(cookies[0].secure);
        assert_eq!(cookies[0].expires, Some(1999999999));

        // #HttpOnly_ prefix is stripped and the line is kept.
        assert_eq!(cookies[1].name, "HSID");
        assert_eq!(cookies[1].expires, None); // expiry 0 => session
    }

    #[test]
    fn comments_and_blanks_are_skipped() {
        assert!(parse_netscape("# just a comment\n\n   \n").is_empty());
    }

    #[test]
    fn jar_serves_loaded_cookies_for_matching_url() {
        let jar = jar_from_netscape(SAMPLE);
        let header = jar
            .cookies(&"https://www.youtube.com/watch".parse().unwrap())
            .expect("expected cookies for youtube.com");
        let s = header.to_str().unwrap();
        assert!(s.contains("SID=abc123"), "got: {s}");
        assert!(s.contains("PREF=f1=40000000"), "got: {s}");
    }
}
