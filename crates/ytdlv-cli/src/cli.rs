//! Command-line interface — a focused subset of yt-dlp's options, oriented to
//! the YouTube MVP.

use clap::Parser;

#[derive(Parser, Debug)]
#[command(
    name = "yt-dlv",
    version,
    about = "A YouTube downloader, rewritten in Rust (a yt-dlp reimplementation).",
    long_about = None,
)]
pub struct Cli {
    /// URLs to download.
    #[arg(value_name = "URL")]
    pub urls: Vec<String>,

    /// Video format selector (yt-dlp `-f` syntax), e.g. `bv*+ba/b`, `137+140`,
    /// `bv*[height<=720]`.
    #[arg(short = 'f', long = "format")]
    pub format: Option<String>,

    /// List available formats and exit (no download).
    #[arg(short = 'F', long = "list-formats")]
    pub list_formats: bool,

    /// Print the extracted info as JSON and exit.
    #[arg(short = 'j', long = "dump-json")]
    pub dump_json: bool,

    /// Print specific field(s) and exit, e.g. `--print title --print id`.
    /// Use `url` to print the selected format URL(s). Repeatable.
    #[arg(long = "print", value_name = "FIELD")]
    pub print: Vec<String>,

    /// Don't download the video (sidecars like --write-subs still run).
    #[arg(long = "skip-download")]
    pub skip_download: bool,

    /// For playlists, list entries without re-extracting each video.
    #[arg(long = "flat-playlist")]
    pub flat_playlist: bool,

    /// 1-based playlist items to take, e.g. `1-3,7,10-` (ranges + lists).
    #[arg(short = 'I', long = "playlist-items", value_name = "ITEMS")]
    pub playlist_items: Option<String>,

    /// Extract audio after download (requires ffmpeg).
    #[arg(short = 'x', long = "extract-audio")]
    pub extract_audio: bool,

    /// Audio format for -x: best, mp3, m4a, aac, opus, flac, wav, ogg.
    #[arg(long = "audio-format", default_value = "best")]
    pub audio_format: String,

    /// Write the info JSON next to the downloaded file.
    #[arg(long = "write-info-json")]
    pub write_info_json: bool,

    /// Write the video thumbnail to a file.
    #[arg(long = "write-thumbnail")]
    pub write_thumbnail: bool,

    /// Write the video description to a .description file.
    #[arg(long = "write-description")]
    pub write_description: bool,

    /// List available subtitle languages and exit.
    #[arg(long = "list-subs")]
    pub list_subs: bool,

    /// Write subtitle files for the selected languages.
    #[arg(long = "write-subs")]
    pub write_subs: bool,

    /// Write automatic (ASR) captions for the selected languages.
    #[arg(long = "write-auto-subs")]
    pub write_auto_subs: bool,

    /// Comma-separated subtitle languages (e.g. `en,de`), or `all`.
    #[arg(long = "sub-langs", default_value = "en")]
    pub sub_langs: String,

    /// Subtitle format to download (vtt, srv3, ttml, json3).
    #[arg(long = "sub-format", default_value = "vtt")]
    pub sub_format: String,

    /// Output filename template.
    #[arg(
        short = 'o',
        long = "output",
        default_value = "%(title)s [%(id)s].%(ext)s"
    )]
    pub output: String,

    /// Base output directory (prepended to the output template).
    #[arg(short = 'P', long = "paths", value_name = "DIR")]
    pub paths: Option<String>,

    /// Restrict filenames to ASCII and avoid spaces/special characters.
    #[arg(long = "restrict-filenames")]
    pub restrict_filenames: bool,

    /// Record downloaded video ids in FILE and skip ids already present.
    #[arg(long = "download-archive", value_name = "FILE")]
    pub download_archive: Option<std::path::PathBuf>,

    /// Container for merged video+audio output.
    #[arg(long = "merge-output-format", default_value = "mp4")]
    pub merge_output_format: String,

    /// Don't actually download; just resolve formats.
    #[arg(long = "simulate")]
    pub simulate: bool,

    /// Overwrite existing files.
    #[arg(long = "force-overwrites")]
    pub force_overwrites: bool,

    /// Proxy URL for all traffic: `http://`, `https://`, or `socks5://`
    /// (credentials allowed, e.g. `socks5://user:pass@host:1080`). Pass an
    /// empty string (`--proxy ""`) to ignore HTTP(S)_PROXY env vars. Useful to
    /// route through a residential IP when YouTube blocks datacenter IPs.
    #[arg(long = "proxy")]
    pub proxy: Option<String>,

    /// Override the User-Agent.
    #[arg(long = "user-agent")]
    pub user_agent: Option<String>,

    /// Netscape-format cookies.txt file. Authenticated requests reduce bot
    /// flagging and can unlock formats that need the JS player.
    #[arg(long = "cookies", value_name = "FILE")]
    pub cookies: Option<std::path::PathBuf>,

    /// Load cookies directly from a local browser profile:
    /// `firefox|chrome|chromium|brave|edge[:PROFILE]`.
    #[arg(
        long = "cookies-from-browser",
        value_name = "BROWSER[:PROFILE]",
        conflicts_with = "cookies"
    )]
    pub cookies_from_browser: Option<String>,

    /// InnerTube player client(s) to use, in priority order
    /// (e.g. `web`, `ios`, `android_vr`, `tv`). Repeatable.
    #[arg(long = "player-client")]
    pub player_client: Vec<String>,

    /// Use an external JS runtime for sig/n solving instead of the embedded
    /// QuickJS engine. Format: `deno|node|bun|quickjs[:/path/to/bin]`.
    #[arg(long = "js-runtime")]
    pub js_runtime: Option<String>,

    /// Quiet: suppress progress output.
    #[arg(short = 'q', long = "quiet")]
    pub quiet: bool,

    /// Verbose logging (repeat for more: -v, -vv).
    #[arg(short = 'v', long = "verbose", action = clap::ArgAction::Count)]
    pub verbose: u8,
}
