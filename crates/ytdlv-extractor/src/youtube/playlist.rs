//! YouTube playlist extraction via the InnerTube `browse` endpoint. Returns the
//! first page of entries as lightweight stubs (id + title + URL); the
//! orchestrator re-extracts each entry to get formats (yt-dlp's model).
//!
//! Pagination (continuations) is future work — this yields the first page
//! (~100 entries).

use anyhow::{anyhow, bail, Result};
use serde_json::{json, Value};
use ytdlv_core::{HttpClient, InfoDict, Playlist};

use super::clients;
use super::player;

pub async fn extract_playlist(http: &HttpClient, playlist_id: &str) -> Result<Playlist> {
    let client = clients::by_key("web").ok_or_else(|| anyhow!("web client unavailable"))?;
    let url = format!(
        "https://www.youtube.com/youtubei/v1/browse?key={}&prettyPrint=false",
        client.api_key
    );
    let body = json!({
        "context": { "client": {
            "clientName": client.client_name,
            "clientVersion": client.client_version,
            "hl": "en", "gl": "US",
        }},
        "browseId": format!("VL{playlist_id}"),
    });
    let headers = player::build_headers(&client)?;
    let resp: Value = http
        .post_json(&url, &body, headers)
        .await
        .map_err(|e| anyhow!("playlist browse failed: {e}"))?;

    let entries = parse_entries(&resp);
    if entries.is_empty() {
        bail!(
            "no entries found for playlist {playlist_id} (mixes/private lists may be unsupported)"
        );
    }
    Ok(Playlist {
        id: playlist_id.to_string(),
        title: find_playlist_title(&resp),
        entries,
        uploader: None,
        webpage_url: Some(format!(
            "https://www.youtube.com/playlist?list={playlist_id}"
        )),
    })
}

fn parse_entries(resp: &Value) -> Vec<InfoDict> {
    let mut out = Vec::new();
    let Some(list) = find_key(resp, "playlistVideoListRenderer") else {
        return out;
    };
    let Some(items) = list.get("contents").and_then(Value::as_array) else {
        return out;
    };
    for it in items {
        let Some(r) = it.get("playlistVideoRenderer") else {
            continue;
        };
        let Some(vid) = r.get("videoId").and_then(Value::as_str) else {
            continue;
        };
        let title = r
            .pointer("/title/runs/0/text")
            .or_else(|| r.pointer("/title/simpleText"))
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        out.push(InfoDict {
            id: vid.to_string(),
            title,
            webpage_url: Some(format!("https://www.youtube.com/watch?v={vid}")),
            extractor: Some("youtube".into()),
            extractor_key: Some("Youtube".into()),
            ..Default::default()
        });
    }
    out
}

fn find_playlist_title(resp: &Value) -> Option<String> {
    if let Some(h) = find_key(resp, "playlistHeaderRenderer") {
        if let Some(t) = h
            .pointer("/title/simpleText")
            .or_else(|| h.pointer("/title/runs/0/text"))
            .and_then(Value::as_str)
        {
            return Some(t.to_string());
        }
    }
    resp.pointer("/microformat/microformatDataRenderer/title")
        .and_then(Value::as_str)
        .map(str::to_string)
}

/// Depth-first search for the first object value under `key` — robust to
/// YouTube's frequent response-layout shuffling.
fn find_key<'a>(v: &'a Value, key: &str) -> Option<&'a Value> {
    match v {
        Value::Object(m) => {
            if let Some(x) = m.get(key) {
                return Some(x);
            }
            m.values().find_map(|vv| find_key(vv, key))
        }
        Value::Array(a) => a.iter().find_map(|vv| find_key(vv, key)),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_playlist_video_renderers() {
        let resp = json!({
            "x": {"playlistVideoListRenderer": {"contents": [
                {"playlistVideoRenderer": {"videoId": "aaaaaaaaaaa",
                 "title": {"runs": [{"text": "First"}]}}},
                {"playlistVideoRenderer": {"videoId": "bbbbbbbbbbb",
                 "title": {"simpleText": "Second"}}},
                {"continuationItemRenderer": {}}
            ]}}
        });
        let entries = parse_entries(&resp);
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].id, "aaaaaaaaaaa");
        assert_eq!(entries[0].title, "First");
        assert_eq!(
            entries[0].webpage_url.as_deref(),
            Some("https://www.youtube.com/watch?v=aaaaaaaaaaa")
        );
        assert_eq!(entries[1].title, "Second");
    }
}
