//! `yt-dlv` entry point and the orchestration layer (yt-dlp's `YoutubeDL`
//! equivalent): resolve a URL to an info dict, select formats, download, and
//! mux with ffmpeg.

mod cli;
mod ffmpeg;
mod format_table;

use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{anyhow, bail, Context, Result};
use clap::Parser;
use ytdlv_core::{output, Extraction, FormatSelector, HttpClient, InfoDict, Selection};
use ytdlv_download::{download_format, DownloadOptions};
use ytdlv_extractor::{ExtractContext, ExtractOptions};
use ytdlv_jsruntime::{ExternalRuntime, ExternalRuntimeKind, JsRuntime, QuickJsRuntime};

#[tokio::main]
async fn main() {
    let cli = cli::Cli::parse();
    init_tracing(cli.verbose, cli.quiet);

    if cli.urls.is_empty() {
        eprintln!("yt-dlv: no URLs provided. Try `yt-dlv --help`.");
        std::process::exit(2);
    }

    let exit = match run(cli).await {
        Ok(0) => 0,
        Ok(_) => 1,
        Err(e) => {
            eprintln!("error: {e:#}");
            1
        }
    };
    std::process::exit(exit);
}

async fn run(cli: cli::Cli) -> Result<usize> {
    let http = Arc::new(build_http_client(&cli)?);
    let js = build_js_runtime(cli.js_runtime.as_deref())?;
    tracing::debug!("js runtime: {}", js.name());

    let mut failures = 0usize;
    for url in &cli.urls {
        if let Err(e) = process_url(url, &cli, &http, js.as_ref()).await {
            eprintln!("error processing {url}: {e:#}");
            failures += 1;
        }
    }
    Ok(failures)
}

async fn process_url(
    url: &str,
    cli: &cli::Cli,
    http: &HttpClient,
    js: &dyn JsRuntime,
) -> Result<()> {
    let extractor = ytdlv_extractor::find_extractor(url)
        .ok_or_else(|| anyhow!("unsupported URL: no extractor matched"))?;
    tracing::info!("using extractor: {}", extractor.key());

    let ctx = ExtractContext {
        http,
        js,
        options: ExtractOptions {
            player_clients: cli.player_client.clone(),
        },
    };

    let extraction = extractor
        .extract(url, &ctx)
        .await
        .map_err(|e| anyhow!("{e}"))?;
    match extraction {
        Extraction::Video(info) => handle_video(*info, cli, http).await,
        Extraction::Playlist(pl) => {
            tracing::info!("playlist '{}' with {} entries", pl.id, pl.entries.len());
            let mut err = None;
            for entry in pl.entries {
                if let Err(e) = handle_video(entry, cli, http).await {
                    eprintln!("  entry error: {e:#}");
                    err = Some(e);
                }
            }
            match err {
                Some(e) => Err(e),
                None => Ok(()),
            }
        }
    }
}

async fn handle_video(info: InfoDict, cli: &cli::Cli, http: &HttpClient) -> Result<()> {
    if cli.dump_json {
        println!("{}", serde_json::to_string_pretty(&info)?);
        return Ok(());
    }
    if cli.list_formats {
        format_table::print(&info);
        return Ok(());
    }

    let spec = cli.format.as_deref().unwrap_or(ytdlv_core::DEFAULT_FORMAT);
    let selector = FormatSelector::parse(spec).map_err(|e| anyhow!("bad format selector: {e}"))?;
    let selection = selector
        .select(&info.formats)
        .ok_or_else(|| anyhow!("requested format '{spec}' not available"))?;

    if cli.write_info_json {
        let path = render_output(cli, &info, "info.json");
        std::fs::write(&path, serde_json::to_vec_pretty(&info)?)
            .with_context(|| format!("writing {}", path.display()))?;
        tracing::info!("wrote {}", path.display());
    }

    if cli.simulate {
        for f in &selection.formats {
            println!(
                "would download format {} ({}, {})",
                f.format_id,
                f.ext,
                f.height
                    .map(|h| format!("{h}p"))
                    .unwrap_or_else(|| "audio".into())
            );
        }
        return Ok(());
    }

    download_selection(&selection, &info, cli, http).await
}

async fn download_selection(
    selection: &Selection,
    info: &InfoDict,
    cli: &cli::Cli,
    http: &HttpClient,
) -> Result<()> {
    let dl_opts = DownloadOptions {
        overwrite: cli.force_overwrites,
        quiet: cli.quiet,
    };

    if !selection.needs_merge() {
        let f = &selection.formats[0];
        let dest = render_output(cli, info, &f.ext);
        download_format(http, f, &dest, &dl_opts).await?;
        println!("Saved: {}", dest.display());
        return Ok(());
    }

    // Merge path: download each part to a temp file, then mux.
    let final_path = render_output(cli, info, &cli.merge_output_format);
    let stem = final_path
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| info.id.clone());
    let dir = final_path.parent().map(PathBuf::from).unwrap_or_default();

    let mut parts = Vec::new();
    for f in &selection.formats {
        let part_name = format!("{stem}.f{}.{}", f.format_id, f.ext);
        let part_path = dir.join(part_name);
        download_format(http, f, &part_path, &dl_opts).await?;
        parts.push((f, part_path));
    }

    let video = parts
        .iter()
        .find(|(f, _)| f.has_video())
        .map(|(_, p)| p.clone());
    let audio = parts
        .iter()
        .find(|(f, _)| f.is_audio_only())
        .map(|(_, p)| p.clone());

    match (video, audio) {
        (Some(v), Some(a)) => {
            ffmpeg::merge(&v, &a, &final_path).context("merging video and audio with ffmpeg")?;
            let _ = std::fs::remove_file(&v);
            let _ = std::fs::remove_file(&a);
            println!("Saved: {}", final_path.display());
        }
        _ => {
            bail!(
                "could not identify separate video and audio streams to merge \
                 (downloaded parts left in place)"
            );
        }
    }
    Ok(())
}

fn render_output(cli: &cli::Cli, info: &InfoDict, ext: &str) -> PathBuf {
    PathBuf::from(output::render(&cli.output, info, ext))
}

fn build_http_client(cli: &cli::Cli) -> Result<HttpClient> {
    let mut b = HttpClient::builder().proxy(cli.proxy.clone());
    if let Some(ua) = &cli.user_agent {
        b = b.user_agent(ua.clone());
    }
    if let Some(path) = &cli.cookies {
        let jar = ytdlv_core::cookies::load_cookie_file(path)?;
        b = b.cookie_jar(jar);
        tracing::info!("loaded cookies from {}", path.display());
    } else if let Some(spec) = &cli.cookies_from_browser {
        let jar = ytdlv_core::cookies_browser::load_from_browser(spec)?;
        b = b.cookie_jar(jar);
    }
    b.build()
}

fn build_js_runtime(spec: Option<&str>) -> Result<Box<dyn JsRuntime>> {
    match spec {
        None => Ok(Box::new(QuickJsRuntime::new())),
        Some(s) => {
            let (kind_str, path) = match s.split_once(':') {
                Some((k, p)) => (k, Some(p)),
                None => (s, None),
            };
            let kind = ExternalRuntimeKind::parse(kind_str)
                .ok_or_else(|| anyhow!("unknown js runtime '{kind_str}'"))?;
            let rt = match path {
                Some(p) => ExternalRuntime::with_binary(kind, p),
                None => ExternalRuntime::new(kind),
            };
            Ok(Box::new(rt))
        }
    }
}

fn init_tracing(verbose: u8, quiet: bool) {
    use tracing_subscriber::EnvFilter;
    let level = if quiet {
        "error"
    } else {
        // The binary's crate name is `yt_dlv` (from `[[bin]] name`), not
        // `ytdlv_cli`, so target that for the CLI's own logs.
        match verbose {
            0 => "yt_dlv=info,ytdlv_extractor=info,ytdlv_download=info,warn",
            1 => "debug,info",
            _ => "trace",
        }
    };
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(level));
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_target(false)
        .without_time()
        .with_writer(std::io::stderr)
        .init();
}
