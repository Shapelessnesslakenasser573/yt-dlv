//! Core types and algorithms shared across yt-dlv: the info-dict contract, the
//! `-f` format-selection language, output templating, and a shared HTTP client.

pub mod format_selection;
pub mod info;
pub mod net;
pub mod output;

pub use format_selection::{FormatSelector, Selection};
pub use info::{Chapter, Extraction, Format, InfoDict, Playlist, Protocol, Subtitle, Thumbnail};
pub use net::HttpClient;

/// The default format selector, matching yt-dlp: best video+audio merged, else
/// best single muxed file.
pub const DEFAULT_FORMAT: &str = "bv*+ba/b";

/// Errors surfaced across the pipeline.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("unsupported URL: no extractor matched {0}")]
    UnsupportedUrl(String),

    #[error("extraction failed: {0}")]
    Extraction(String),

    #[error("requested format not available: {0}")]
    FormatUnavailable(String),

    #[error("network error: {0}")]
    Network(#[from] reqwest::Error),

    #[error(transparent)]
    Other(#[from] anyhow::Error),
}

pub type Result<T> = std::result::Result<T, Error>;
