//! Subtitle listing and downloading (`--list-subs`, `--write-subs`,
//! `--write-auto-subs`).

use std::collections::BTreeMap;

use anyhow::{Context, Result};
use ytdlv_core::{output, HttpClient, InfoDict, Subtitle};

use crate::cli::Cli;

/// Print available subtitle/auto-caption languages, like yt-dlp's `--list-subs`.
pub fn list(info: &InfoDict) {
    print_group("Available subtitles", &info.subtitles);
    print_group("Available automatic captions", &info.automatic_captions);
    if info.subtitles.is_empty() && info.automatic_captions.is_empty() {
        println!("{} has no subtitles", info.id);
    }
}

fn print_group(heading: &str, group: &BTreeMap<String, Vec<Subtitle>>) {
    if group.is_empty() {
        return;
    }
    println!("{heading} for {} languages:", group.len());
    println!("{:<10} Formats", "Language");
    for (lang, entries) in group {
        let formats: Vec<&str> = entries.iter().filter_map(|s| s.ext.as_deref()).collect();
        println!("{lang:<10} {}", formats.join(", "));
    }
}

/// Download the selected subtitle languages/formats next to the media file.
pub async fn write(info: &InfoDict, cli: &Cli, http: &HttpClient) -> Result<()> {
    let mut sources: Vec<&BTreeMap<String, Vec<Subtitle>>> = Vec::new();
    if cli.write_subs {
        sources.push(&info.subtitles);
    }
    if cli.write_auto_subs {
        sources.push(&info.automatic_captions);
    }

    for group in sources {
        let langs = resolve_langs(&cli.sub_langs, group);
        for lang in langs {
            let Some(entries) = group.get(&lang) else {
                tracing::warn!("subtitle language '{lang}' not available");
                continue;
            };
            let chosen = entries
                .iter()
                .find(|s| s.ext.as_deref() == Some(cli.sub_format.as_str()))
                .or_else(|| entries.first());
            let Some(sub) = chosen else { continue };
            let ext = sub.ext.as_deref().unwrap_or("vtt");

            let path = output::render(&cli.output, info, &format!("{lang}.{ext}"));
            match http.get_text(&sub.url).await {
                Ok(body) => {
                    std::fs::write(&path, body)
                        .with_context(|| format!("writing subtitle {path}"))?;
                    println!("Saved subtitle: {path}");
                }
                Err(e) => tracing::warn!("failed to download {lang} subtitle: {e}"),
            }
        }
    }
    Ok(())
}

/// Resolve `--sub-langs` against what's available: `all` expands to every
/// language, otherwise the comma-separated list is used as-is.
fn resolve_langs(spec: &str, group: &BTreeMap<String, Vec<Subtitle>>) -> Vec<String> {
    if spec.trim() == "all" {
        return group.keys().cloned().collect();
    }
    spec.split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect()
}
