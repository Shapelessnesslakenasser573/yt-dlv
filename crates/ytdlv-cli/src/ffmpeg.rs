//! Thin ffmpeg wrapper for muxing. Like yt-dlp, we shell out to the `ffmpeg`
//! binary rather than linking libav.

use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{bail, Result};

/// Extract/convert the audio track of `input` to `audio_format` (`best` copies
/// the source codec into m4a). Returns the produced path.
pub fn extract_audio(input: &Path, audio_format: &str) -> Result<PathBuf> {
    ensure_available()?;
    let (ext, codec_args): (&str, &[&str]) = match audio_format {
        "mp3" => ("mp3", &["-c:a", "libmp3lame"]),
        "aac" | "m4a" => ("m4a", &["-c:a", "aac"]),
        "opus" => ("opus", &["-c:a", "libopus"]),
        "flac" => ("flac", &["-c:a", "flac"]),
        "wav" => ("wav", &[]),
        "ogg" | "vorbis" => ("ogg", &["-c:a", "libvorbis"]),
        _ => ("m4a", &["-c:a", "copy"]), // "best": keep source codec
    };
    let final_out = input.with_extension(ext);
    // Write to a temp name first so input==output (e.g. .m4a -> .m4a) is safe.
    let tmp_out = input.with_extension(format!("extract.{ext}"));

    let ok = run_extract(input, &tmp_out, codec_args)?;
    if !ok {
        // "best" copy can fail if the container rejects the codec; re-encode.
        let recovered = audio_format == "best" && run_extract(input, &tmp_out, &["-c:a", "aac"])?;
        if !recovered {
            let _ = std::fs::remove_file(&tmp_out);
            bail!("ffmpeg failed to extract audio");
        }
    }
    std::fs::rename(&tmp_out, &final_out)?;
    Ok(final_out)
}

/// Embed metadata tags into `path` in-place (stream copy, no re-encode).
pub fn embed_metadata(path: &Path, tags: &[(String, String)]) -> Result<()> {
    ensure_available()?;
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("mkv");
    let tmp = path.with_extension(format!("meta.{ext}"));

    let mut cmd = Command::new("ffmpeg");
    cmd.args(["-y", "-loglevel", "warning", "-i"])
        .arg(path)
        .args(["-map", "0", "-c", "copy"]);
    for (k, v) in tags {
        if !v.is_empty() {
            cmd.arg("-metadata").arg(format!("{k}={v}"));
        }
    }
    let status = cmd.arg(&tmp).status()?;
    if !status.success() {
        let _ = std::fs::remove_file(&tmp);
        bail!("ffmpeg failed to embed metadata");
    }
    std::fs::rename(&tmp, path)?;
    Ok(())
}

fn run_extract(input: &Path, output: &Path, codec_args: &[&str]) -> Result<bool> {
    Ok(Command::new("ffmpeg")
        .args(["-y", "-loglevel", "warning", "-i"])
        .arg(input)
        .arg("-vn")
        .args(codec_args)
        .arg(output)
        .status()?
        .success())
}

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
