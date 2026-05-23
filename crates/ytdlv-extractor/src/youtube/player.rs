//! InnerTube player request/response handling: build the API call for a client,
//! parse `streamingData` into [`Format`]s, and resolve each format's final URL
//! (signature decryption + `n`-parameter descrambling).

use anyhow::{anyhow, bail, Result};
use reqwest::header::{HeaderMap, HeaderName, HeaderValue};
use serde_json::{json, Value};
use url::Url;
use ytdlv_core::{Format, HttpClient, Protocol};
use ytdlv_jsruntime::JsRuntime;

use super::clients::InnerTubeClient;
use super::signature::{parse_signature_cipher, PlayerSolver};

/// Metadata pulled from `videoDetails`.
#[derive(Debug, Clone, Default)]
pub struct VideoDetails {
    pub id: String,
    pub title: String,
    pub duration: Option<f64>,
    pub author: Option<String>,
    pub channel_id: Option<String>,
    pub view_count: Option<u64>,
    pub description: Option<String>,
    pub is_live: Option<bool>,
}

fn player_url(client: &InnerTubeClient) -> String {
    format!(
        "https://www.youtube.com/youtubei/v1/player?key={}&prettyPrint=false",
        client.api_key
    )
}

/// Build the `context`/request JSON for a player call.
pub fn build_request_body(client: &InnerTubeClient, video_id: &str, sts: Option<u64>) -> Value {
    let mut clientctx = json!({
        "clientName": client.client_name,
        "clientVersion": client.client_version,
        "hl": "en",
        "gl": "US",
    });
    if let Some(m) = client.device_model {
        clientctx["deviceModel"] = json!(m);
    }
    if let Some(os) = client.os_name {
        clientctx["osName"] = json!(os);
    }
    if let Some(v) = client.os_version {
        clientctx["osVersion"] = json!(v);
    }

    let mut body = json!({
        "context": { "client": clientctx },
        "videoId": video_id,
        "contentCheckOk": true,
        "racyCheckOk": true,
    });
    if let Some(sts) = sts {
        body["playbackContext"] = json!({
            "contentPlaybackContext": {
                "html5Preference": "HTML5_PREF_WANTS",
                "signatureTimestamp": sts,
            }
        });
    }
    body
}

/// Build request headers identifying the client.
pub fn build_headers(client: &InnerTubeClient) -> Result<HeaderMap> {
    let mut h = HeaderMap::new();
    h.insert(
        reqwest::header::CONTENT_TYPE,
        HeaderValue::from_static("application/json"),
    );
    h.insert(
        reqwest::header::ORIGIN,
        HeaderValue::from_static("https://www.youtube.com"),
    );
    h.insert(
        reqwest::header::USER_AGENT,
        HeaderValue::from_str(client.user_agent)?,
    );
    h.insert(
        HeaderName::from_static("x-youtube-client-name"),
        HeaderValue::from_str(&client.client_id.to_string())?,
    );
    h.insert(
        HeaderName::from_static("x-youtube-client-version"),
        HeaderValue::from_str(client.client_version)?,
    );
    Ok(h)
}

/// Make the player API call and return the raw JSON.
pub async fn call_player(
    http: &HttpClient,
    client: &InnerTubeClient,
    video_id: &str,
    sts: Option<u64>,
) -> Result<Value> {
    let body = build_request_body(client, video_id, sts);
    let headers = build_headers(client)?;
    let v: Value = http
        .post_json(&player_url(client), &body, headers)
        .await
        .map_err(|e| anyhow!("player API call failed: {e}"))?;
    Ok(v)
}

/// Check `playabilityStatus`; return a descriptive error if not OK.
pub fn check_playability(resp: &Value) -> Result<()> {
    let status = resp
        .pointer("/playabilityStatus/status")
        .and_then(Value::as_str)
        .unwrap_or("UNKNOWN");
    if status == "OK" {
        return Ok(());
    }
    let reason = resp
        .pointer("/playabilityStatus/reason")
        .or_else(|| resp.pointer("/playabilityStatus/messages/0"))
        .and_then(Value::as_str)
        .unwrap_or("");
    bail!("not playable ({status}): {reason}");
}

pub fn parse_video_details(resp: &Value) -> VideoDetails {
    let d = resp.get("videoDetails").cloned().unwrap_or(Value::Null);
    VideoDetails {
        id: d
            .get("videoId")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        title: d
            .get("title")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        duration: d
            .get("lengthSeconds")
            .and_then(Value::as_str)
            .and_then(|s| s.parse::<f64>().ok()),
        author: d.get("author").and_then(Value::as_str).map(str::to_string),
        channel_id: d
            .get("channelId")
            .and_then(Value::as_str)
            .map(str::to_string),
        view_count: d
            .get("viewCount")
            .and_then(Value::as_str)
            .and_then(|s| s.parse::<u64>().ok()),
        description: d
            .get("shortDescription")
            .and_then(Value::as_str)
            .map(str::to_string),
        is_live: d.get("isLiveContent").and_then(Value::as_bool),
    }
}

/// Parse all `formats` + `adaptiveFormats`, resolving URLs via the solver.
pub fn parse_formats(resp: &Value, solver: &PlayerSolver, rt: &dyn JsRuntime) -> Vec<Format> {
    let mut out = Vec::new();
    for key in ["formats", "adaptiveFormats"] {
        if let Some(arr) = resp
            .pointer(&format!("/streamingData/{key}"))
            .and_then(Value::as_array)
        {
            for f in arr {
                match parse_one_format(f, solver, rt) {
                    Ok(Some(fmt)) => out.push(fmt),
                    Ok(None) => {}
                    Err(e) => tracing::debug!("skipping format: {e}"),
                }
            }
        }
    }
    out
}

fn parse_one_format(
    f: &Value,
    solver: &PlayerSolver,
    rt: &dyn JsRuntime,
) -> Result<Option<Format>> {
    let itag = f
        .get("itag")
        .and_then(Value::as_i64)
        .ok_or_else(|| anyhow!("no itag"))?;

    // DRM-protected formats are unusable; skip but mark.
    let has_drm = f.get("drmFamilies").is_some();

    let mime = f.get("mimeType").and_then(Value::as_str).unwrap_or("");
    let (container, codecs) = parse_mime(mime);
    let (vcodec, acodec) = split_codecs(&codecs, f);

    let mut fmt = Format {
        format_id: itag.to_string(),
        ext: pick_ext(&container, vcodec.is_some()),
        protocol: Protocol::Https,
        container: Some(container),
        vcodec: Some(vcodec.unwrap_or_else(|| "none".into())),
        acodec: Some(acodec.unwrap_or_else(|| "none".into())),
        width: f.get("width").and_then(Value::as_u64).map(|v| v as u32),
        height: f.get("height").and_then(Value::as_u64).map(|v| v as u32),
        fps: f.get("fps").and_then(Value::as_f64),
        tbr: f.get("bitrate").and_then(Value::as_f64).map(|b| b / 1000.0),
        asr: f
            .get("audioSampleRate")
            .and_then(Value::as_str)
            .and_then(|s| s.parse().ok()),
        audio_channels: f
            .get("audioChannels")
            .and_then(Value::as_u64)
            .map(|v| v as u32),
        filesize: f
            .get("contentLength")
            .and_then(Value::as_str)
            .and_then(|s| s.parse().ok()),
        quality: f.get("itag").and_then(Value::as_f64),
        has_drm,
        ..Default::default()
    };

    fmt.url = resolve_url(f, solver, rt)?;
    if fmt.url.is_empty() {
        return Ok(None);
    }
    Ok(Some(fmt))
}

/// Produce the final, playable URL: decrypt the signature cipher if present,
/// then descramble the `n` parameter.
fn resolve_url(f: &Value, solver: &PlayerSolver, rt: &dyn JsRuntime) -> Result<String> {
    let mut url = if let Some(cipher) = f
        .get("signatureCipher")
        .or_else(|| f.get("cipher"))
        .and_then(Value::as_str)
    {
        let (s, sp, base) =
            parse_signature_cipher(cipher).ok_or_else(|| anyhow!("malformed signatureCipher"))?;
        let sig = solver.decrypt_signature(rt, &s)?;
        set_query_param(&base, &sp, &sig)
    } else if let Some(u) = f.get("url").and_then(Value::as_str) {
        u.to_string()
    } else {
        return Ok(String::new());
    };

    // n-parameter throttling mitigation.
    if let Some(n) = get_query_param(&url, "n") {
        match solver.decrypt_n(rt, &n) {
            Ok(new_n) => url = set_query_param(&url, "n", &new_n),
            Err(e) => tracing::warn!("n-param descramble failed (download may throttle): {e}"),
        }
    }
    Ok(url)
}

// ---------------------------------------------------------------------------
// mimeType / codec parsing
// ---------------------------------------------------------------------------

/// `video/mp4; codecs="avc1.640028, mp4a.40.2"` -> ("mp4", "avc1.640028, mp4a.40.2").
fn parse_mime(mime: &str) -> (String, String) {
    let container = mime
        .split(';')
        .next()
        .and_then(|t| t.split('/').nth(1))
        .unwrap_or("mp4")
        .to_string();
    let codecs = mime
        .split("codecs=")
        .nth(1)
        .map(|c| c.trim().trim_matches('"').to_string())
        .unwrap_or_default();
    (container, codecs)
}

/// Decide vcodec/acodec from the codecs string and the format's audio fields.
fn split_codecs(codecs: &str, f: &Value) -> (Option<String>, Option<String>) {
    let parts: Vec<&str> = codecs
        .split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .collect();
    let is_audio = f.get("audioSampleRate").is_some() && f.get("width").is_none();
    let is_video_only = f.get("width").is_some() && f.get("audioSampleRate").is_none();

    match parts.len() {
        0 => (None, None),
        1 => {
            if is_audio {
                (None, Some(parts[0].to_string()))
            } else if is_video_only {
                (Some(parts[0].to_string()), None)
            } else if looks_like_audio(parts[0]) {
                (None, Some(parts[0].to_string()))
            } else {
                (Some(parts[0].to_string()), None)
            }
        }
        // Progressive/muxed: first is video, second audio.
        _ => (Some(parts[0].to_string()), Some(parts[1].to_string())),
    }
}

fn looks_like_audio(codec: &str) -> bool {
    codec.starts_with("mp4a")
        || codec.starts_with("opus")
        || codec.starts_with("vorbis")
        || codec.starts_with("ac-3")
        || codec.starts_with("ec-3")
}

fn pick_ext(container: &str, has_video: bool) -> String {
    match container {
        "mp4" => if has_video { "mp4" } else { "m4a" }.to_string(),
        "webm" => "webm".to_string(),
        "3gpp" => "3gp".to_string(),
        other => other.to_string(),
    }
}

// ---------------------------------------------------------------------------
// URL query helpers
// ---------------------------------------------------------------------------

fn get_query_param(u: &str, key: &str) -> Option<String> {
    let parsed = Url::parse(u).ok()?;
    parsed
        .query_pairs()
        .find(|(k, _)| k == key)
        .map(|(_, v)| v.into_owned())
}

/// Set (or append) a query parameter, preserving order of existing pairs.
fn set_query_param(u: &str, key: &str, val: &str) -> String {
    let Ok(mut parsed) = Url::parse(u) else {
        return u.to_string();
    };
    let pairs: Vec<(String, String)> = parsed
        .query_pairs()
        .map(|(k, v)| (k.into_owned(), v.into_owned()))
        .collect();
    let mut found = false;
    {
        let mut qp = parsed.query_pairs_mut();
        qp.clear();
        for (k, v) in &pairs {
            if k == key {
                qp.append_pair(k, val);
                found = true;
            } else {
                qp.append_pair(k, v);
            }
        }
        if !found {
            qp.append_pair(key, val);
        }
    }
    parsed.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_video_mime() {
        let (c, codecs) = parse_mime(r#"video/mp4; codecs="avc1.640028""#);
        assert_eq!(c, "mp4");
        assert_eq!(codecs, "avc1.640028");
    }

    #[test]
    fn parses_progressive_codecs() {
        let f = json!({"width": 640, "height": 360, "audioSampleRate": "44100"});
        let (v, a) = split_codecs("avc1.42001E, mp4a.40.2", &f);
        assert_eq!(v.as_deref(), Some("avc1.42001E"));
        assert_eq!(a.as_deref(), Some("mp4a.40.2"));
    }

    #[test]
    fn audio_only_ext_is_m4a() {
        assert_eq!(pick_ext("mp4", false), "m4a");
        assert_eq!(pick_ext("mp4", true), "mp4");
        assert_eq!(pick_ext("webm", false), "webm");
    }

    #[test]
    fn set_query_param_replaces_and_appends() {
        let u = "https://h/p?itag=18&n=OLD&x=1";
        assert_eq!(get_query_param(u, "n").as_deref(), Some("OLD"));
        let out = set_query_param(u, "n", "NEW");
        assert_eq!(get_query_param(&out, "n").as_deref(), Some("NEW"));
        assert!(out.contains("itag=18"));
        let appended = set_query_param(u, "sig", "ABC");
        assert_eq!(get_query_param(&appended, "sig").as_deref(), Some("ABC"));
    }
}
