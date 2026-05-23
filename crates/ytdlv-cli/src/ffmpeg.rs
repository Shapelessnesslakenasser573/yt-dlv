//! Thin ffmpeg wrapper for muxing. Like yt-dlp, we shell out to the `ffmpeg`
//! binary rather than linking libav.

use std::path::Path;
use std::process::Command;

use anyhow::{bail, Result};

/// Mux a video-only and an audio-only file into `out` by stream copy (no
/// re-encode). Falls back to `.mkv` if the chosen container rejects the codecs.
pub fn merge(video: &Path, audio: &Path, out: &Path) -> Result<()> {
    ensure_available()?;

    let status = run_merge(video, audio, out)?;
    if status.success() {
        return Ok(());
    }

    // Common case: mp4 can't hold opus/vp9 by stream-copy. Retry as mkv.
    if out.extension().and_then(|e| e.to_str()) != Some("mkv") {
        let mkv = out.with_extension("mkv");
        tracing::warn!(
            "ffmpeg copy into {} failed; retrying as {}",
            out.display(),
            mkv.display()
        );
        let status = run_merge(video, audio, &mkv)?;
        if status.success() {
            return Ok(());
        }
    }
    bail!("ffmpeg failed to merge streams");
}

fn run_merge(video: &Path, audio: &Path, out: &Path) -> Result<std::process::ExitStatus> {
    Ok(Command::new("ffmpeg")
        .args(["-y", "-loglevel", "warning", "-i"])
        .arg(video)
        .arg("-i")
        .arg(audio)
        .args(["-c", "copy", "-movflags", "+faststart"])
        .arg(out)
        .status()?)
}

fn ensure_available() -> Result<()> {
    match Command::new("ffmpeg").arg("-version").output() {
        Ok(o) if o.status.success() => Ok(()),
        _ => bail!(
            "ffmpeg not found on PATH — required to merge separate video and audio streams. \
             Install ffmpeg, or select a muxed format (e.g. `-f b` / `-f 18`)."
        ),
    }
}
