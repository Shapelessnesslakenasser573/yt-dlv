//! The YouTube extractor: the marquee target of the rewrite.
//!
//! Flow: parse the video id → fetch the player `base.js` and build a
//! [`PlayerSolver`] (signature/n transforms + `sts`) → query one or more
//! InnerTube clients → parse `streamingData` into formats with fully-resolved
//! URLs → assemble an [`InfoDict`].

pub mod clients;
pub mod player;
pub mod signature;

use async_trait::async_trait;
use regex::Regex;
use serde_json::Value;
use ytdlv_core::{Error, Extraction, InfoDict, Result, Thumbnail};

use crate::{ExtractContext, Extractor};
use signature::PlayerSolver;

pub struct YoutubeExtractor;

impl YoutubeExtractor {
    pub fn new() -> Self {
        Self
    }
}

impl Default for YoutubeExtractor {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Extractor for YoutubeExtractor {
    fn key(&self) -> &'static str {
        "Youtube"
    }

    fn matches(&self, url: &str) -> bool {
        is_youtube_host(url) && parse_video_id(url).is_some()
    }

    async fn extract(&self, url: &str, ctx: &ExtractContext<'_>) -> Result<Extraction> {
        let video_id = parse_video_id(url)
            .ok_or_else(|| Error::UnsupportedUrl(url.to_string()))?;
        tracing::info!("youtube: extracting video {video_id}");

        // Build the JS solver from base.js (needed for web/tv clients + sts).
        let solver = match fetch_base_js(ctx).await {
            Ok(js) => {
                let s = PlayerSolver::from_base_js(&js);
                tracing::info!(
                    "player solver: sig={} nsig={} sts={:?}",
                    s.has_sig(),
                    s.has_nsig(),
                    s.signature_timestamp
                );
                Some(s)
            }
            Err(e) => {
                tracing::warn!("could not load base.js ({e}); JS-player clients will be skipped");
                None
            }
        };
        let sts = solver.as_ref().and_then(|s| s.signature_timestamp);

        let order: Vec<String> = if ctx.options.player_clients.is_empty() {
            clients::DEFAULT_ORDER.iter().map(|s| s.to_string()).collect()
        } else {
            ctx.options.player_clients.clone()
        };

        let mut formats = Vec::new();
        let mut details: Option<player::VideoDetails> = None;
        let mut first_resp: Option<Value> = None;
        let mut last_error: Option<String> = None;

        for key in &order {
            let Some(client) = clients::by_key(key) else {
                tracing::warn!("unknown player client '{key}', skipping");
                continue;
            };
            // JS-player clients are useless without a solver.
            if client.requires_player && solver.is_none() {
                continue;
            }
            tracing::info!("querying InnerTube client '{}'", client.key);

            let resp = match player::call_player(ctx.http, &client, &video_id, sts).await {
                Ok(r) => r,
                Err(e) => {
                    tracing::warn!("client '{}' failed: {e}", client.key);
                    last_error = Some(e.to_string());
                    continue;
                }
            };
            if let Err(e) = player::check_playability(&resp) {
                tracing::warn!("client '{}': {e}", client.key);
                last_error = Some(e.to_string());
                continue;
            }

            // The solver is only meaningfully used for clients needing it, but
            // direct-URL clients may still carry an `n` param worth fixing.
            let empty = PlayerSolver::from_base_js("");
            let used_solver = solver.as_ref().unwrap_or(&empty);
            let parsed = player::parse_formats(&resp, used_solver, ctx.js);
            tracing::info!("client '{}' yielded {} formats", client.key, parsed.len());

            for f in parsed {
                if !formats.iter().any(|e: &ytdlv_core::Format| e.format_id == f.format_id) {
                    formats.push(f);
                }
            }
            if details.is_none() {
                details = Some(player::parse_video_details(&resp));
                first_resp = Some(resp);
            }
        }

        if formats.is_empty() {
            return Err(Error::Extraction(format!(
                "no formats extracted for {video_id}{}",
                last_error.map(|e| format!(" (last error: {e})")).unwrap_or_default()
            )));
        }

        let details = details.unwrap_or_default();
        let resp = first_resp.unwrap_or(Value::Null);

        let info = InfoDict {
            id: if details.id.is_empty() { video_id.clone() } else { details.id },
            title: if details.title.is_empty() {
                video_id.clone()
            } else {
                details.title
            },
            formats,
            thumbnails: parse_thumbnails(&resp, &video_id),
            description: details.description,
            duration: details.duration,
            uploader: details.author.clone(),
            channel: details.author,
            channel_id: details.channel_id.clone(),
            channel_url: details
                .channel_id
                .map(|c| format!("https://www.youtube.com/channel/{c}")),
            view_count: details.view_count,
            upload_date: parse_upload_date(&resp),
            webpage_url: Some(format!("https://www.youtube.com/watch?v={video_id}")),
            extractor: Some("youtube".into()),
            extractor_key: Some("Youtube".into()),
            is_live: details.is_live,
            ..Default::default()
        };

        Ok(Extraction::Video(Box::new(info)))
    }
}

/// Fetch the watch page and download the referenced player `base.js`.
async fn fetch_base_js(ctx: &ExtractContext<'_>) -> anyhow::Result<String> {
    // The embed page reliably advertises the player JS URL and is lighter than
    // a full watch page.
    let html = ctx
        .http
        .get_text("https://www.youtube.com/embed/dQw4w9WgXcQ")
        .await?;
    let js_url = find_player_js_url(&html)
        .ok_or_else(|| anyhow::anyhow!("could not find jsUrl in player page"))?;
    tracing::debug!("player base.js: {js_url}");
    Ok(ctx.http.get_text(&js_url).await?)
}

fn find_player_js_url(html: &str) -> Option<String> {
    let re = Regex::new(r#""(?:jsUrl|PLAYER_JS_URL)"\s*:\s*"([^"]+base\.js)""#).unwrap();
    let path = re
        .captures(html)
        .and_then(|c| c.get(1))
        .map(|m| m.as_str().replace("\\/", "/"))
        .or_else(|| {
            // Fallback: any /s/player/.../base.js reference.
            Regex::new(r"/s/player/[0-9a-fA-F]{8,}/[\w./_-]+/base\.js")
                .unwrap()
                .find(html)
                .map(|m| m.as_str().to_string())
        })?;
    if path.starts_with("http") {
        Some(path)
    } else {
        Some(format!("https://www.youtube.com{path}"))
    }
}

fn is_youtube_host(url: &str) -> bool {
    let host = url::Url::parse(url).ok().and_then(|u| u.host_str().map(str::to_string));
    match host {
        Some(h) => {
            let h = h.trim_start_matches("www.").trim_start_matches("m.");
            h == "youtube.com"
                || h == "youtu.be"
                || h == "music.youtube.com"
                || h == "youtube-nocookie.com"
                || h.ends_with(".youtube.com")
        }
        None => false,
    }
}

/// Extract the 11-character video id from any common YouTube URL shape.
pub fn parse_video_id(url: &str) -> Option<String> {
    let re = Regex::new(
        r"(?x)
        (?:
            v=|/shorts/|/embed/|/live/|/v/|/e/|youtu\.be/
        )
        (?P<id>[0-9A-Za-z_-]{11})
        ",
    )
    .unwrap();
    re.captures(url)
        .and_then(|c| c.name("id"))
        .map(|m| m.as_str().to_string())
}

fn parse_thumbnails(resp: &Value, video_id: &str) -> Vec<Thumbnail> {
    let mut out = Vec::new();
    if let Some(arr) = resp
        .pointer("/videoDetails/thumbnail/thumbnails")
        .and_then(Value::as_array)
    {
        for t in arr {
            if let Some(url) = t.get("url").and_then(Value::as_str) {
                out.push(Thumbnail {
                    url: url.to_string(),
                    width: t.get("width").and_then(Value::as_u64).map(|v| v as u32),
                    height: t.get("height").and_then(Value::as_u64).map(|v| v as u32),
                    ..Default::default()
                });
            }
        }
    }
    if out.is_empty() {
        out.push(Thumbnail {
            url: format!("https://i.ytimg.com/vi/{video_id}/maxresdefault.jpg"),
            ..Default::default()
        });
    }
    out
}

fn parse_upload_date(resp: &Value) -> Option<String> {
    let raw = resp
        .pointer("/microformat/playerMicroformatRenderer/uploadDate")
        .or_else(|| resp.pointer("/microformat/playerMicroformatRenderer/publishDate"))
        .and_then(Value::as_str)?;
    // "2009-10-25T07:57:33-07:00" -> "20091025"
    let date = raw.get(0..10)?;
    Some(date.replace('-', ""))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_ids_from_url_shapes() {
        let cases = [
            ("https://www.youtube.com/watch?v=dQw4w9WgXcQ", "dQw4w9WgXcQ"),
            ("https://youtu.be/dQw4w9WgXcQ?t=10", "dQw4w9WgXcQ"),
            ("https://www.youtube.com/shorts/abcdefghijk", "abcdefghijk"),
            ("https://m.youtube.com/watch?v=dQw4w9WgXcQ&list=x", "dQw4w9WgXcQ"),
            ("https://www.youtube.com/embed/dQw4w9WgXcQ", "dQw4w9WgXcQ"),
        ];
        for (url, id) in cases {
            assert_eq!(parse_video_id(url).as_deref(), Some(id), "url: {url}");
        }
    }

    #[test]
    fn matcher_accepts_youtube_rejects_others() {
        let e = YoutubeExtractor::new();
        assert!(e.matches("https://www.youtube.com/watch?v=dQw4w9WgXcQ"));
        assert!(e.matches("https://youtu.be/dQw4w9WgXcQ"));
        assert!(!e.matches("https://vimeo.com/12345678"));
        assert!(!e.matches("https://example.com/watch?v=dQw4w9WgXcQ"));
    }

    #[test]
    fn finds_js_url_relative_and_absolute() {
        let html = r#"...,"jsUrl":"\/s\/player\/abcd1234\/player_ias.vflset\/en_US\/base.js",..."#;
        assert_eq!(
            find_player_js_url(html).as_deref(),
            Some("https://www.youtube.com/s/player/abcd1234/player_ias.vflset/en_US/base.js")
        );
    }

    #[test]
    fn upload_date_normalised() {
        let v = serde_json::json!({
            "microformat": {"playerMicroformatRenderer": {"uploadDate": "2009-10-25T07:57:33-07:00"}}
        });
        assert_eq!(parse_upload_date(&v).as_deref(), Some("20091025"));
    }
}
