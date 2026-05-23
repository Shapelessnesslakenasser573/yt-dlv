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

    /// Write the info JSON next to the downloaded file.
    #[arg(long = "write-info-json")]
    pub write_info_json: bool,

    /// Output filename template.
    #[arg(short = 'o', long = "output", default_value = "%(title)s [%(id)s].%(ext)s")]
    pub output: String,

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
