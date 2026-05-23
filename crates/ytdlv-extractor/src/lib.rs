//! Extractor framework and the YouTube extractor.
//!
//! An [`Extractor`] maps a URL to an [`Extraction`] (a video or a playlist),
//! using the shared HTTP client and a [`JsRuntime`] for sites (like YouTube)
//! that require executing player JavaScript.

use async_trait::async_trait;
use ytdlv_core::{Extraction, HttpClient};
use ytdlv_jsruntime::JsRuntime;

pub mod generic;
pub mod youtube;

pub use generic::GenericExtractor;
pub use youtube::YoutubeExtractor;

/// Everything an extractor needs from the host application.
pub struct ExtractContext<'a> {
    pub http: &'a HttpClient,
    pub js: &'a dyn JsRuntime,
    pub options: ExtractOptions,
}

/// Per-extraction knobs.
#[derive(Debug, Clone, Default)]
pub struct ExtractOptions {
    /// InnerTube clients to query, in priority order. Empty = extractor default.
    pub player_clients: Vec<String>,
}

#[async_trait]
pub trait Extractor: Send + Sync {
    /// Stable identifier, e.g. `"Youtube"` (mirrors yt-dlp's extractor key).
    fn key(&self) -> &'static str;

    /// Whether this extractor handles `url` (the `_VALID_URL` check).
    fn matches(&self, url: &str) -> bool;

    async fn extract(&self, url: &str, ctx: &ExtractContext<'_>) -> ytdlv_core::Result<Extraction>;
}

/// All registered extractors, in match priority order. The generic extractor is
/// last so site-specific extractors win.
pub fn registry() -> Vec<Box<dyn Extractor>> {
    vec![
        Box::new(YoutubeExtractor::new()),
        Box::new(GenericExtractor::new()),
    ]
}

/// Find the first extractor that claims `url`.
pub fn find_extractor(url: &str) -> Option<Box<dyn Extractor>> {
    registry().into_iter().find(|e| e.matches(url))
}
