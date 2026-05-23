//! Output-filename templating, a focused subset of yt-dlp's `-o` engine.
//! Supports `%(field)s` and `%(field)d` with the common metadata fields, and
//! sanitises the result into a safe filename.

use std::collections::HashMap;

use once_cell_lite::Lazy;
use regex::Regex;

use crate::info::InfoDict;

mod once_cell_lite {
    //! Tiny `Lazy` so this crate needn't depend on `once_cell`.
    use std::ops::Deref;
    use std::sync::OnceLock;

    pub struct Lazy<T> {
        cell: OnceLock<T>,
        init: fn() -> T,
    }
    impl<T> Lazy<T> {
        pub const fn new(init: fn() -> T) -> Self {
            Self {
                cell: OnceLock::new(),
                init,
            }
        }
    }
    impl<T> Deref for Lazy<T> {
        type Target = T;
        fn deref(&self) -> &T {
            self.cell.get_or_init(self.init)
        }
    }
}

// Supports `%(key)s`, `%(key)d`, an optional `|default` (`%(uploader|Unknown)s`),
// and a format spec: zero/space pad (`%(n)05d`) and string truncation
// (`%(title).40s`).
static FIELD_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(
        r"%\((?P<key>[a-zA-Z_][a-zA-Z0-9_]*)(?:\|(?P<default>[^)]*))?\)(?P<spec>\d*(?:\.\d+)?)(?P<conv>[sd])",
    )
    .unwrap()
});

/// Render `template` against `info` (using `ext` for `%(ext)s`) into a safe
/// filename.
pub fn render(template: &str, info: &InfoDict, ext: &str) -> String {
    sanitize_path(&substitute(template, info, ext))
}

/// Like [`render`] but without filename sanitisation — for `--print`, where the
/// raw field values are wanted.
pub fn render_raw(template: &str, info: &InfoDict, ext: &str) -> String {
    substitute(template, info, ext)
}

fn substitute(template: &str, info: &InfoDict, ext: &str) -> String {
    let fields = build_fields(info, ext);
    FIELD_RE
        .replace_all(template, |caps: &regex::Captures| {
            let key = &caps["key"];
            let value = match fields.get(key) {
                Some(v) => v.clone(),
                None => match caps.name("default") {
                    Some(d) => d.as_str().to_string(),
                    None => "NA".to_string(),
                },
            };
            let spec = caps.name("spec").map(|m| m.as_str()).unwrap_or("");
            let conv = caps["conv"].chars().next().unwrap_or('s');
            apply_spec(&value, spec, conv)
        })
        .into_owned()
}

/// Apply a printf-style spec subset: width (`5`), zero-pad (`05`) for `d`, and
/// precision/truncation (`.40`) for `s`.
fn apply_spec(value: &str, spec: &str, conv: char) -> String {
    let (width_str, precision) = match spec.split_once('.') {
        Some((w, p)) => (w, p.parse::<usize>().ok()),
        None => (spec, None),
    };
    let width = width_str.parse::<usize>().ok();
    let zero_pad = width_str.starts_with('0');

    if conv == 'd' {
        if let Ok(n) = value.parse::<i64>() {
            return match (width, zero_pad) {
                (Some(w), true) => format!("{n:0w$}"),
                (Some(w), false) => format!("{n:w$}"),
                _ => n.to_string(),
            };
        }
        return value.to_string();
    }

    // String: truncate to precision, then pad to width.
    let mut s: String = match precision {
        Some(p) => value.chars().take(p).collect(),
        None => value.to_string(),
    };
    if let Some(w) = width {
        if s.chars().count() < w {
            s = format!("{s:<w$}");
        }
    }
    s
}

fn build_fields(info: &InfoDict, ext: &str) -> HashMap<String, String> {
    let mut m = HashMap::new();
    let mut put = |k: &str, v: String| {
        m.insert(k.to_string(), v);
    };
    put("id", info.id.clone());
    put("title", info.title.clone());
    put("ext", ext.to_string());
    if let Some(v) = &info.uploader {
        put("uploader", v.clone());
    }
    if let Some(v) = &info.uploader_id {
        put("uploader_id", v.clone());
    }
    if let Some(v) = &info.channel {
        put("channel", v.clone());
    }
    if let Some(v) = &info.channel_id {
        put("channel_id", v.clone());
    }
    if let Some(v) = &info.upload_date {
        put("upload_date", v.clone());
    }
    if let Some(v) = info.duration {
        put("duration", (v as i64).to_string());
    }
    if let Some(v) = info.view_count {
        put("view_count", v.to_string());
    }
    if let Some(v) = info.like_count {
        put("like_count", v.to_string());
    }
    if let Some(v) = &info.extractor {
        put("extractor", v.clone());
    }
    m
}

/// Sanitise into a filename safe across platforms while preserving any path
/// separators the template intentionally produced.
fn sanitize_path(s: &str) -> String {
    s.split('/')
        .map(sanitize_component)
        .collect::<Vec<_>>()
        .join("/")
}

fn sanitize_component(s: &str) -> String {
    let mut out: String = s
        .chars()
        .map(|c| match c {
            '<' | '>' | ':' | '"' | '\\' | '|' | '?' | '*' => '_',
            c if (c as u32) < 0x20 => '_',
            c => c,
        })
        .collect();
    // Avoid trailing dots/spaces (Windows-hostile) and empty names.
    while out.ends_with('.') || out.ends_with(' ') {
        out.pop();
    }
    if out.is_empty() {
        out.push('_');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> InfoDict {
        InfoDict {
            id: "dQw4w9WgXcQ".into(),
            title: "Rick Astley - Never Gonna Give You Up".into(),
            view_count: Some(1_600_000_000),
            ..Default::default()
        }
    }

    #[test]
    fn renders_default_template() {
        let out = render("%(title)s [%(id)s].%(ext)s", &sample(), "mp4");
        assert_eq!(
            out,
            "Rick Astley - Never Gonna Give You Up [dQw4w9WgXcQ].mp4"
        );
    }

    #[test]
    fn sanitises_illegal_chars() {
        let mut info = sample();
        info.title = "a/b:c*d?".into();
        // The '/' becomes a path separator; the others are scrubbed.
        let out = render("%(title)s.%(ext)s", &info, "mkv");
        assert_eq!(out, "a/b_c_d_.mkv");
    }

    #[test]
    fn missing_field_is_na() {
        let out = render("%(uploader)s.%(ext)s", &sample(), "mp4");
        assert_eq!(out, "NA.mp4");
    }

    #[test]
    fn numeric_conversion() {
        let out = render("%(view_count)d.%(ext)s", &sample(), "mp4");
        assert_eq!(out, "1600000000.mp4");
    }

    #[test]
    fn default_value_when_field_missing() {
        let out = render("%(uploader|Unknown)s.%(ext)s", &sample(), "mp4");
        assert_eq!(out, "Unknown.mp4");
        // Present field ignores the default.
        let mut info = sample();
        info.uploader = Some("Rick".into());
        assert_eq!(render("%(uploader|Unknown)s", &info, "mp4"), "Rick");
    }

    #[test]
    fn string_truncation() {
        let out = render("%(title).10s.%(ext)s", &sample(), "mp4");
        assert_eq!(out, "Rick Astle.mp4");
    }

    #[test]
    fn integer_zero_padding() {
        let mut info = sample();
        info.view_count = Some(7);
        assert_eq!(render("%(view_count)05d", &info, "mp4"), "00007");
    }
}
