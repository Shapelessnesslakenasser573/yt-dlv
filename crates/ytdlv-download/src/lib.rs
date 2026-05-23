//! HTTP download engine: ranged, resumable, single-stream download of one
//! [`Format`] to disk with a progress bar.
//!
//! YouTube's googlevideo URLs are range-capable, so we download into a
//! `<dest>.part` file and, if interrupted, resume from its current length via a
//! `Range` request. HLS/DASH fragment downloaders are future work; for now the
//! YouTube progressive and adaptive formats we target are plain HTTP.

use std::path::{Path, PathBuf};

use anyhow::{anyhow, Context, Result};
use futures_util::StreamExt;
use indicatif::{ProgressBar, ProgressStyle};
use reqwest::header::HeaderMap;
use tokio::io::AsyncWriteExt;
use ytdlv_core::{Format, HttpClient, Protocol};

#[derive(Debug, Clone, Default)]
pub struct DownloadOptions {
    /// Overwrite an existing completed file instead of skipping it.
    pub overwrite: bool,
    /// Suppress the progress bar.
    pub quiet: bool,
}

/// Download a single format to `dest`. Returns the path written.
pub async fn download_format(
    http: &HttpClient,
    format: &Format,
    dest: &Path,
    opts: &DownloadOptions,
) -> Result<PathBuf> {
    if !matches!(format.protocol, Protocol::Https) {
        return Err(anyhow!(
            "protocol {:?} not yet supported by the native downloader",
            format.protocol
        ));
    }

    if dest.exists() && !opts.overwrite {
        tracing::info!("{} already exists, skipping", dest.display());
        return Ok(dest.to_path_buf());
    }

    let mut headers = HeaderMap::new();
    for (k, v) in &format.http_headers {
        if let (Ok(name), Ok(val)) = (
            reqwest::header::HeaderName::from_bytes(k.as_bytes()),
            reqwest::header::HeaderValue::from_str(v),
        ) {
            headers.insert(name, val);
        }
    }

    let part = part_path(dest);
    let mut downloaded = std::fs::metadata(&part).map(|m| m.len()).unwrap_or(0);

    let total = format
        .filesize
        .or(format.filesize_approx)
        .or(http.content_length(&format.url, &headers).await);

    // If the partial is already complete, just finalize it.
    if let Some(total) = total {
        if downloaded >= total && total > 0 {
            std::fs::rename(&part, dest)?;
            return Ok(dest.to_path_buf());
        }
    }

    let pb = build_progress(total, downloaded, opts.quiet, dest);

    let mut file = tokio::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&part)
        .await
        .with_context(|| format!("opening {}", part.display()))?;

    let resp = http
        .get_range(&format.url, downloaded, None, &headers)
        .await
        .context("starting ranged download")?;

    // If the server ignored our Range (200 not 206) but we'd resumed, restart.
    if downloaded > 0 && resp.status() == reqwest::StatusCode::OK {
        tracing::warn!("server ignored Range; restarting download from 0");
        drop(file);
        file = tokio::fs::File::create(&part).await?;
        downloaded = 0;
        pb.set_position(0);
    }

    let mut stream = resp.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.context("reading response chunk")?;
        file.write_all(&chunk).await.context("writing to disk")?;
        downloaded += chunk.len() as u64;
        pb.set_position(downloaded);
    }
    file.flush().await?;
    drop(file);

    std::fs::rename(&part, dest).with_context(|| format!("finalizing {}", dest.display()))?;
    pb.finish_and_clear();
    if !opts.quiet {
        tracing::info!("downloaded {}", dest.display());
    }
    Ok(dest.to_path_buf())
}

fn part_path(dest: &Path) -> PathBuf {
    let mut s = dest.as_os_str().to_os_string();
    s.push(".part");
    PathBuf::from(s)
}

fn build_progress(total: Option<u64>, start: u64, quiet: bool, dest: &Path) -> ProgressBar {
    if quiet {
        return ProgressBar::hidden();
    }
    let pb = match total {
        Some(t) => {
            let pb = ProgressBar::new(t);
            pb.set_style(
                ProgressStyle::with_template(
                    "{msg} {bar:30.cyan/blue} {bytes}/{total_bytes} ({bytes_per_sec}, {eta})",
                )
                .unwrap()
                .progress_chars("=>-"),
            );
            pb
        }
        None => {
            let pb = ProgressBar::new_spinner();
            pb.set_style(
                ProgressStyle::with_template("{msg} {spinner} {bytes} ({bytes_per_sec})").unwrap(),
            );
            pb
        }
    };
    pb.set_position(start);
    pb.set_message(
        dest.file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "download".into()),
    );
    pb
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn part_path_appends_suffix() {
        assert_eq!(
            part_path(Path::new("video.mp4")),
            PathBuf::from("video.mp4.part")
        );
    }
}
