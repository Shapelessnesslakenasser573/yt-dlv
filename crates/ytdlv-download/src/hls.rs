//! Native HLS (m3u8) downloader: resolve a master playlist to its
//! highest-bandwidth media playlist, then download and concatenate the
//! segments. Unencrypted segments only for now — `#EXT-X-KEY` (AES-128) is
//! detected and rejected with a clear error.

use std::path::{Path, PathBuf};

use anyhow::{anyhow, bail, Context, Result};
use indicatif::{ProgressBar, ProgressStyle};
use reqwest::Url;
use tokio::io::AsyncWriteExt;
use ytdlv_core::HttpClient;

use crate::part_path;

pub async fn download_hls(
    http: &HttpClient,
    playlist_url: &str,
    dest: &Path,
    quiet: bool,
) -> Result<PathBuf> {
    let top = http
        .get_text(playlist_url)
        .await
        .context("fetching m3u8 playlist")?;

    // Master playlist? Pick the highest-bandwidth variant's media playlist.
    let (media_url, media_text) = if top.contains("#EXT-X-STREAM-INF") {
        let variant = pick_variant(&top, playlist_url)?;
        let text = http
            .get_text(&variant)
            .await
            .context("fetching media playlist")?;
        (variant, text)
    } else {
        (playlist_url.to_string(), top)
    };

    if media_text
        .lines()
        .any(|l| l.starts_with("#EXT-X-KEY") && !l.contains("METHOD=NONE"))
    {
        bail!("encrypted HLS (#EXT-X-KEY) is not yet supported; try a different format");
    }

    let segments = parse_segments(&media_text, &media_url)?;
    if segments.is_empty() {
        bail!("no segments found in HLS media playlist");
    }

    let part = part_path(dest);
    let mut file = tokio::fs::File::create(&part)
        .await
        .with_context(|| format!("creating {}", part.display()))?;

    let pb = progress(segments.len() as u64, quiet, dest);
    for (i, seg) in segments.iter().enumerate() {
        let bytes = http
            .raw()
            .get(seg)
            .send()
            .await
            .with_context(|| format!("requesting segment {}", i + 1))?
            .error_for_status()
            .with_context(|| format!("segment {} failed", i + 1))?
            .bytes()
            .await
            .with_context(|| format!("reading segment {}", i + 1))?;
        file.write_all(&bytes).await.context("writing segment")?;
        pb.set_position((i + 1) as u64);
    }
    file.flush().await?;
    drop(file);

    std::fs::rename(&part, dest).with_context(|| format!("finalizing {}", dest.display()))?;
    pb.finish_and_clear();
    if !quiet {
        tracing::info!(
            "downloaded {} ({} segments)",
            dest.display(),
            segments.len()
        );
    }
    Ok(dest.to_path_buf())
}

/// From a master playlist, return the absolute URL of the highest-bandwidth
/// variant's media playlist.
fn pick_variant(master: &str, base: &str) -> Result<String> {
    let mut best: Option<(u64, &str)> = None;
    let lines: Vec<&str> = master.lines().collect();
    for (i, line) in lines.iter().enumerate() {
        if line.starts_with("#EXT-X-STREAM-INF") {
            // BANDWIDTH= is an attribute on the EXT-X-STREAM-INF line; find it
            // anywhere and read the digits that follow.
            let bw = line
                .split("BANDWIDTH=")
                .nth(1)
                .map(|s| {
                    s.chars()
                        .take_while(|c| c.is_ascii_digit())
                        .collect::<String>()
                })
                .and_then(|d| d.parse::<u64>().ok())
                .unwrap_or(0);
            // The URI is the next non-comment line.
            if let Some(uri) = lines.get(i + 1..).and_then(|rest| {
                rest.iter()
                    .find(|l| !l.trim().is_empty() && !l.starts_with('#'))
            }) {
                if best.map(|(b, _)| bw > b).unwrap_or(true) {
                    best = Some((bw, uri));
                }
            }
        }
    }
    let uri = best
        .map(|(_, u)| u)
        .ok_or_else(|| anyhow!("no variants in master playlist"))?;
    resolve(base, uri.trim())
}

/// Segment URIs are the non-empty, non-comment lines of a media playlist.
fn parse_segments(media: &str, base: &str) -> Result<Vec<String>> {
    media
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .map(|l| resolve(base, l))
        .collect()
}

fn resolve(base: &str, link: &str) -> Result<String> {
    if link.starts_with("http://") || link.starts_with("https://") {
        return Ok(link.to_string());
    }
    Url::parse(base)
        .and_then(|b| b.join(link))
        .map(|u| u.to_string())
        .map_err(|e| anyhow!("resolving '{link}' against '{base}': {e}"))
}

fn progress(total: u64, quiet: bool, dest: &Path) -> ProgressBar {
    if quiet {
        return ProgressBar::hidden();
    }
    let pb = ProgressBar::new(total);
    pb.set_style(
        ProgressStyle::with_template("{msg} {bar:30.cyan/blue} {pos}/{len} segments ({eta})")
            .unwrap()
            .progress_chars("=>-"),
    );
    pb.set_message(
        dest.file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "hls".into()),
    );
    pb
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn picks_highest_bandwidth_variant() {
        let master = "#EXTM3U\n\
            #EXT-X-STREAM-INF:BANDWIDTH=800000,RESOLUTION=640x360\n\
            low/index.m3u8\n\
            #EXT-X-STREAM-INF:BANDWIDTH=2400000,RESOLUTION=1280x720\n\
            high/index.m3u8\n";
        let v = pick_variant(master, "https://h/master.m3u8").unwrap();
        assert_eq!(v, "https://h/high/index.m3u8");
    }

    #[test]
    fn parses_and_resolves_segments() {
        let media = "#EXTM3U\n\
            #EXT-X-TARGETDURATION:10\n\
            #EXTINF:9.0,\n\
            seg0.ts\n\
            #EXTINF:9.0,\n\
            https://cdn/seg1.ts\n\
            #EXT-X-ENDLIST\n";
        let segs = parse_segments(media, "https://h/v/index.m3u8").unwrap();
        assert_eq!(
            segs,
            vec![
                "https://h/v/seg0.ts".to_string(),
                "https://cdn/seg1.ts".to_string()
            ]
        );
    }
}
