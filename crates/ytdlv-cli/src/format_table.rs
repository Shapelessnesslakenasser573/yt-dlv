//! `-F` output: a human-readable table of available formats, ordered roughly
//! like yt-dlp (audio-only first, then by resolution).

use ytdlv_core::{Format, InfoDict};

pub fn print(info: &InfoDict) {
    println!("Available formats for {}:", info.id);
    println!(
        "{:<8} {:<5} {:<11} {:>5} {:>9} {:<14} {:<14} {}",
        "ID", "EXT", "RESOLUTION", "FPS", "TBR", "VCODEC", "ACODEC", "NOTE"
    );
    println!("{}", "-".repeat(96));

    let mut formats: Vec<&Format> = info.formats.iter().collect();
    formats.sort_by(|a, b| {
        a.height
            .unwrap_or(0)
            .cmp(&b.height.unwrap_or(0))
            .then(a.effective_tbr().unwrap_or(0.0).total_cmp(&b.effective_tbr().unwrap_or(0.0)))
    });

    for f in formats {
        let resolution = match (f.width, f.height) {
            (Some(w), Some(h)) => format!("{w}x{h}"),
            _ if f.is_audio_only() => "audio only".to_string(),
            _ => "-".to_string(),
        };
        let fps = f.fps.map(|v| format!("{}", v as u32)).unwrap_or_else(|| "-".into());
        let tbr = f.effective_tbr().map(|v| format!("{:.0}k", v)).unwrap_or_else(|| "-".into());
        let note = build_note(f);
        println!(
            "{:<8} {:<5} {:<11} {:>5} {:>9} {:<14} {:<14} {}",
            f.format_id,
            f.ext,
            resolution,
            fps,
            tbr,
            short_codec(f.vcodec.as_deref()),
            short_codec(f.acodec.as_deref()),
            note
        );
    }
}

fn build_note(f: &Format) -> String {
    let mut parts = Vec::new();
    if let Some(n) = &f.format_note {
        parts.push(n.clone());
    }
    if f.is_audio_only() {
        parts.push("audio only".into());
    } else if f.is_video_only() {
        parts.push("video only".into());
    }
    if f.has_drm {
        parts.push("DRM".into());
    }
    if let Some(size) = f.filesize.or(f.filesize_approx) {
        parts.push(human_size(size));
    }
    parts.join(", ")
}

fn short_codec(c: Option<&str>) -> String {
    match c {
        None | Some("none") | Some("") => "-".to_string(),
        Some(c) => c.split('.').next().unwrap_or(c).to_string(),
    }
}

fn human_size(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KiB", "MiB", "GiB", "TiB"];
    let mut size = bytes as f64;
    let mut unit = 0;
    while size >= 1024.0 && unit < UNITS.len() - 1 {
        size /= 1024.0;
        unit += 1;
    }
    format!("{size:.2}{}", UNITS[unit])
}
