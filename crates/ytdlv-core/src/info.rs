//! The "info dict" contract — yt-dlp's central data structure, ported to typed
//! Rust. Every extractor produces these; the downloader and post-processors
//! consume them. Field names and `serde` renames mirror yt-dlp so
//! `--write-info-json` output stays familiar.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// The result of extracting a URL: either one media item or a playlist of them.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "_type", rename_all = "snake_case")]
pub enum Extraction {
    Video(Box<InfoDict>),
    Playlist(Playlist),
}

/// A playlist / channel / multi-video page.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Playlist {
    pub id: String,
    pub title: Option<String>,
    pub entries: Vec<InfoDict>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub uploader: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub webpage_url: Option<String>,
}

/// A single extractable media item and all its formats/metadata.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct InfoDict {
    pub id: String,
    pub title: String,

    #[serde(default)]
    pub formats: Vec<Format>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub thumbnails: Vec<Thumbnail>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Seconds.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration: Option<f64>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub uploader: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub uploader_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub channel: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub channel_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub channel_url: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub view_count: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub like_count: Option<u64>,
    /// `YYYYMMDD`, matching yt-dlp.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub upload_date: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timestamp: Option<i64>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub webpage_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extractor: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extractor_key: Option<String>,

    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub subtitles: BTreeMap<String, Vec<Subtitle>>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub automatic_captions: BTreeMap<String, Vec<Subtitle>>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub chapters: Vec<Chapter>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub age_limit: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub is_live: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub live_status: Option<String>,
}

/// How a format's bytes are delivered. Mirrors yt-dlp's `protocol` field, which
/// selects the downloader.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Protocol {
    /// Plain HTTP(S), range-capable. The common case for YouTube progressive
    /// and adaptive formats.
    Https,
    /// Native HLS (m3u8) fragment download.
    M3u8Native,
    /// DASH segments described by an MPD manifest.
    HttpDashSegments,
}

impl Default for Protocol {
    fn default() -> Self {
        Protocol::Https
    }
}

/// A single downloadable rendition.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Format {
    pub format_id: String,
    pub url: String,
    pub ext: String,

    #[serde(default)]
    pub protocol: Protocol,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub format_note: Option<String>,
    /// `"none"` (or absent) means no video stream.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub vcodec: Option<String>,
    /// `"none"` (or absent) means no audio stream.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub acodec: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub width: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub height: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fps: Option<f64>,

    /// Total average bitrate (kbps).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tbr: Option<f64>,
    /// Audio bitrate (kbps).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub abr: Option<f64>,
    /// Video bitrate (kbps).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub vbr: Option<f64>,
    /// Audio sample rate (Hz).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub asr: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub audio_channels: Option<u32>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub filesize: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub filesize_approx: Option<u64>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub container: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<String>,
    /// Extractor-assigned quality hint; higher is better.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub quality: Option<f64>,

    #[serde(default, skip_serializing_if = "is_false")]
    pub has_drm: bool,

    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub http_headers: BTreeMap<String, String>,
}

fn is_false(b: &bool) -> bool {
    !*b
}

fn codec_present(c: &Option<String>) -> bool {
    matches!(c.as_deref(), Some(v) if !v.is_empty() && v != "none")
}

impl Format {
    pub fn has_video(&self) -> bool {
        codec_present(&self.vcodec)
    }

    pub fn has_audio(&self) -> bool {
        codec_present(&self.acodec)
    }

    /// Both a video and an audio stream in one file (progressive / muxed).
    pub fn is_muxed(&self) -> bool {
        self.has_video() && self.has_audio()
    }

    pub fn is_video_only(&self) -> bool {
        self.has_video() && !self.has_audio()
    }

    pub fn is_audio_only(&self) -> bool {
        self.has_audio() && !self.has_video()
    }

    /// Best-effort total bitrate for ranking, falling back across tbr/vbr+abr.
    pub fn effective_tbr(&self) -> Option<f64> {
        self.tbr.or_else(|| match (self.vbr, self.abr) {
            (Some(v), Some(a)) => Some(v + a),
            (Some(v), None) => Some(v),
            (None, Some(a)) => Some(a),
            (None, None) => None,
        })
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Thumbnail {
    pub url: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub width: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub height: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub preference: Option<i32>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Subtitle {
    pub url: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ext: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Chapter {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    pub start_time: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub end_time: Option<f64>,
}
