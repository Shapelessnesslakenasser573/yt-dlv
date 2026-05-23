//! A generic catch-all extractor (yt-dlp's `Generic`): handle direct media
//! URLs and scrape `og:video` / `<video>`/`<source>` from arbitrary pages.
//! Registered last, so site-specific extractors win.

use async_trait::async_trait;
use regex::Regex;
use ytdlv_core::{Error, Extraction, Format, InfoDict, Protocol, Result};

use crate::{ExtractContext, Extractor};

const VIDEO_EXTS: &[&str] = &["mp4", "m4v", "webm", "mkv", "mov", "flv", "ts"];
const AUDIO_EXTS: &[&str] = &["mp3", "m4a", "aac", "ogg", "oga", "opus", "wav", "flac"];

pub struct GenericExtractor;

impl GenericExtractor {
    pub fn new() -> Self {
        Self
    }
}

impl Default for GenericExtractor {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Extractor for GenericExtractor {
    fn key(&self) -> &'static str {
        "generic"
    }

    fn matches(&self, url: &str) -> bool {
        url.starts_with("http://") || url.starts_with("https://")
    }

    async fn extract(&self, url: &str, ctx: &ExtractContext<'_>) -> Result<Extraction> {
        tracing::info!("generic: probing {url}");
        let resp = ctx
            .http
            .raw()
            .get(url)
            .send()
            .await
            .map_err(|e| Error::Extraction(format!("request failed: {e}")))?
            .error_for_status()
            .map_err(|e| Error::Extraction(e.to_string()))?;

        let final_url = resp.url().to_string();
        let ctype = resp
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_lowercase();
        let clen = resp.content_length();

        // Direct media: the URL itself is the file.
        if ctype.starts_with("video/")
            || ctype.starts_with("audio/")
            || media_ext(&final_url).is_some()
        {
            let fmt = direct_format("0", &final_url, &ctype, clen);
            let info = build_info(&final_url, page_title_from_url(&final_url), vec![fmt]);
            return Ok(Extraction::Video(Box::new(info)));
        }

        // Otherwise treat as an HTML page and scrape for media.
        let body = resp
            .text()
            .await
            .map_err(|e| Error::Extraction(format!("reading page: {e}")))?;
        let media = scrape_media_urls(&body, &final_url);
        if media.is_empty() {
            return Err(Error::Extraction(format!(
                "no downloadable media found at {final_url}"
            )));
        }
        let title = scrape_title(&body).unwrap_or_else(|| page_title_from_url(&final_url));
        let formats = media
            .into_iter()
            .enumerate()
            .map(|(i, u)| direct_format(&i.to_string(), &u, "", None))
            .collect();
        Ok(Extraction::Video(Box::new(build_info(
            &final_url, title, formats,
        ))))
    }
}

fn build_info(url: &str, title: String, formats: Vec<Format>) -> InfoDict {
    InfoDict {
        id: derive_id(url),
        title,
        formats,
        webpage_url: Some(url.to_string()),
        extractor: Some("generic".into()),
        extractor_key: Some("Generic".into()),
        ..Default::default()
    }
}

fn direct_format(id: &str, url: &str, ctype: &str, filesize: Option<u64>) -> Format {
    let ext = guess_ext(url, ctype);
    let is_audio = AUDIO_EXTS.contains(&ext.as_str()) || ctype.starts_with("audio/");
    let protocol = match ext.as_str() {
        "m3u8" => Protocol::M3u8Native,
        "mpd" => Protocol::HttpDashSegments,
        _ => Protocol::Https,
    };
    Format {
        format_id: id.to_string(),
        url: url.to_string(),
        ext,
        protocol,
        // Codecs are unknown; mark "unknown" (not "none") so the format is still
        // selectable. Video files are assumed muxed; audio files audio-only.
        vcodec: Some(if is_audio { "none" } else { "unknown" }.into()),
        acodec: Some("unknown".into()),
        filesize,
        ..Default::default()
    }
}

fn guess_ext(url: &str, ctype: &str) -> String {
    if let Some(e) = media_ext(url) {
        return e;
    }
    match ctype.split(';').next().unwrap_or("").trim() {
        "video/mp4" => "mp4",
        "video/webm" => "webm",
        "video/x-matroska" => "mkv",
        "video/quicktime" => "mov",
        "audio/mpeg" => "mp3",
        "audio/mp4" | "audio/x-m4a" => "m4a",
        "audio/ogg" => "ogg",
        "audio/wav" | "audio/x-wav" => "wav",
        "application/vnd.apple.mpegurl" | "application/x-mpegurl" => "m3u8",
        "application/dash+xml" => "mpd",
        _ => "mp4",
    }
    .to_string()
}

/// The media file extension of a URL's path, if it's a known media type.
fn media_ext(url: &str) -> Option<String> {
    let path = url.split(['?', '#']).next().unwrap_or(url);
    let ext = path.rsplit('.').next()?.to_lowercase();
    if VIDEO_EXTS.contains(&ext.as_str())
        || AUDIO_EXTS.contains(&ext.as_str())
        || ext == "m3u8"
        || ext == "mpd"
    {
        Some(ext)
    } else {
        None
    }
}

fn derive_id(url: &str) -> String {
    let path = url.split(['?', '#']).next().unwrap_or(url);
    let last = path.rsplit('/').next().unwrap_or(path);
    let stem = last.rsplit_once('.').map(|(s, _)| s).unwrap_or(last);
    if stem.is_empty() {
        "generic".to_string()
    } else {
        stem.to_string()
    }
}

fn page_title_from_url(url: &str) -> String {
    derive_id(url)
}

fn scrape_title(html: &str) -> Option<String> {
    if let Some(c) =
        Regex::new(r#"<meta[^>]+property=["']og:title["'][^>]+content=["']([^"']+)["']"#)
            .unwrap()
            .captures(html)
    {
        return Some(c[1].to_string());
    }
    Regex::new(r"(?is)<title>\s*(.*?)\s*</title>")
        .unwrap()
        .captures(html)
        .map(|c| c[1].trim().to_string())
}

/// Scrape candidate media URLs from a page: og:video, twitter player streams,
/// `<video src>` / `<source src>`, and bare media links.
fn scrape_media_urls(html: &str, base: &str) -> Vec<String> {
    let mut out = Vec::new();
    let patterns = [
        r#"<meta[^>]+property=["']og:video(?::url|:secure_url)?["'][^>]+content=["']([^"']+)["']"#,
        r#"<meta[^>]+name=["']twitter:player:stream["'][^>]+content=["']([^"']+)["']"#,
        r#"<(?:video|source|audio)[^>]+src=["']([^"']+)["']"#,
    ];
    for p in patterns {
        for c in Regex::new(p).unwrap().captures_iter(html) {
            if let Some(m) = c.get(1) {
                push_unique(&mut out, resolve_url(base, m.as_str()));
            }
        }
    }
    // Bare media links as a last resort.
    for c in Regex::new(r#"https?://[^\s"'<>]+\.(?:mp4|webm|m3u8|mp3|m4a)(?:\?[^\s"'<>]*)?"#)
        .unwrap()
        .find_iter(html)
    {
        push_unique(&mut out, c.as_str().to_string());
    }
    out
}

fn push_unique(v: &mut Vec<String>, s: String) {
    if !s.is_empty() && !v.contains(&s) {
        v.push(s);
    }
}

fn resolve_url(base: &str, link: &str) -> String {
    if link.starts_with("http://") || link.starts_with("https://") {
        return link.to_string();
    }
    match url::Url::parse(base).and_then(|b| b.join(link)) {
        Ok(u) => u.to_string(),
        Err(_) => link.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_media_extensions() {
        assert_eq!(media_ext("https://h/v.mp4?x=1").as_deref(), Some("mp4"));
        assert_eq!(media_ext("https://h/a.m3u8").as_deref(), Some("m3u8"));
        assert_eq!(media_ext("https://h/page.html"), None);
    }

    #[test]
    fn direct_video_is_selectable_muxed() {
        let f = direct_format("0", "https://h/v.mp4", "video/mp4", Some(100));
        assert!(f.is_muxed(), "generic video should be treated as muxed");
        assert_eq!(f.ext, "mp4");
    }

    #[test]
    fn direct_audio_is_audio_only() {
        let f = direct_format("0", "https://h/a.mp3", "audio/mpeg", None);
        assert!(f.is_audio_only());
        assert_eq!(f.ext, "mp3");
    }

    #[test]
    fn scrapes_og_video_and_resolves_relative() {
        let html = r#"<meta property="og:video" content="/clips/x.mp4"><title>Hi</title>"#;
        let urls = scrape_media_urls(html, "https://example.com/page");
        assert_eq!(urls, vec!["https://example.com/clips/x.mp4"]);
        assert_eq!(scrape_title(html).as_deref(), Some("Hi"));
    }
}
