//! A parser and evaluator for yt-dlp's `-f` format-selection mini-language.
//!
//! Supported today: special selectors (`best`, `worst`, `bestvideo`/`bv`,
//! `bestaudio`/`ba`, the `*` "may be muxed" variants, `b`/`w`), explicit format
//! ids (`137`), `+` merges (`bv*+ba`), `/` fallbacks (`bv*+ba/b`), and bracket
//! filters (`[height<=720][ext=mp4][vcodec^=avc1]`).
//!
//! Not yet: parenthesised groups and comma-separated multiple outputs.

use crate::info::Format;

/// One or more formats chosen for a single output. Length > 1 means the parts
/// must be muxed together (e.g. video-only + audio-only).
#[derive(Debug, Clone)]
pub struct Selection {
    pub formats: Vec<Format>,
}

impl Selection {
    pub fn needs_merge(&self) -> bool {
        self.formats.len() > 1
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Base {
    /// Best/worst muxed (video+audio in one file).
    BestMuxed,
    WorstMuxed,
    /// Best/worst video stream that is video-only.
    BestVideoOnly,
    WorstVideoOnly,
    /// Best/worst audio stream that is audio-only.
    BestAudioOnly,
    WorstAudioOnly,
    /// `bv*` / `ba*` — best/worst that *has* video / audio (muxed allowed).
    BestWithVideo,
    BestWithAudio,
    /// `b*`/`best*` — best of anything.
    BestAny,
    WorstAny,
    /// A literal format id.
    Id(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Op {
    Lt,
    Le,
    Gt,
    Ge,
    Eq,
    Ne,
    StartsWith,
    EndsWith,
    Contains,
}

#[derive(Debug, Clone)]
struct Filter {
    field: String,
    op: Op,
    value: String,
}

#[derive(Debug, Clone)]
struct Component {
    base: Base,
    filters: Vec<Filter>,
}

/// A parsed selector: alternatives separated by `/`, each a `+`-merge of
/// components.
#[derive(Debug, Clone)]
pub struct FormatSelector {
    alternatives: Vec<Vec<Component>>,
}

impl FormatSelector {
    pub fn parse(spec: &str) -> Result<Self, String> {
        let mut alternatives = Vec::new();
        for alt in spec.split('/') {
            let alt = alt.trim();
            if alt.is_empty() {
                continue;
            }
            let mut components = Vec::new();
            for comp in alt.split('+') {
                components.push(parse_component(comp.trim())?);
            }
            alternatives.push(components);
        }
        if alternatives.is_empty() {
            return Err("empty format selector".into());
        }
        Ok(Self { alternatives })
    }

    /// Evaluate against available formats, returning the first alternative whose
    /// every component resolves.
    pub fn select(&self, formats: &[Format]) -> Option<Selection> {
        for alt in &self.alternatives {
            let mut chosen = Vec::new();
            let mut ok = true;
            for comp in alt {
                match comp.pick(formats) {
                    Some(f) => chosen.push(f.clone()),
                    None => {
                        ok = false;
                        break;
                    }
                }
            }
            if ok && !chosen.is_empty() {
                return Some(Selection { formats: chosen });
            }
        }
        None
    }
}

fn parse_component(s: &str) -> Result<Component, String> {
    // Split the leading selector from any trailing `[..]` filter groups.
    let bracket = s.find('[');
    let (name, rest) = match bracket {
        Some(i) => (&s[..i], &s[i..]),
        None => (s, ""),
    };
    let base = parse_base(name.trim())?;
    let filters = parse_filters(rest)?;
    Ok(Component { base, filters })
}

fn parse_base(name: &str) -> Result<Base, String> {
    Ok(match name {
        "" | "best" | "b" => Base::BestMuxed,
        "worst" | "w" => Base::WorstMuxed,
        "bestvideo" | "bv" => Base::BestVideoOnly,
        "worstvideo" | "wv" => Base::WorstVideoOnly,
        "bestaudio" | "ba" => Base::BestAudioOnly,
        "worstaudio" | "wa" => Base::WorstAudioOnly,
        "bestvideo*" | "bv*" => Base::BestWithVideo,
        "bestaudio*" | "ba*" => Base::BestWithAudio,
        "best*" | "b*" => Base::BestAny,
        "worst*" | "w*" => Base::WorstAny,
        id => Base::Id(id.to_string()),
    })
}

fn parse_filters(mut s: &str) -> Result<Vec<Filter>, String> {
    let mut filters = Vec::new();
    while !s.is_empty() {
        s = s.trim_start();
        if s.is_empty() {
            break;
        }
        if !s.starts_with('[') {
            return Err(format!("expected '[' in filter, got {s:?}"));
        }
        let end = s.find(']').ok_or_else(|| "unterminated '[' filter".to_string())?;
        let inner = &s[1..end];
        filters.push(parse_filter(inner.trim())?);
        s = &s[end + 1..];
    }
    Ok(filters)
}

fn parse_filter(s: &str) -> Result<Filter, String> {
    // Order matters: try two-char operators before single-char.
    const OPS: &[(&str, Op)] = &[
        ("<=", Op::Le),
        (">=", Op::Ge),
        ("!=", Op::Ne),
        ("^=", Op::StartsWith),
        ("$=", Op::EndsWith),
        ("*=", Op::Contains),
        ("<", Op::Lt),
        (">", Op::Gt),
        ("=", Op::Eq),
    ];
    for (tok, op) in OPS {
        if let Some(i) = s.find(tok) {
            let field = s[..i].trim().to_string();
            let value = s[i + tok.len()..].trim().trim_matches(|c| c == '\'' || c == '"');
            if field.is_empty() {
                return Err(format!("empty field in filter {s:?}"));
            }
            return Ok(Filter { field, op: *op, value: value.to_string() });
        }
    }
    Err(format!("no operator in filter {s:?}"))
}

impl Component {
    fn pick<'a>(&self, formats: &'a [Format]) -> Option<&'a Format> {
        // First, restrict by the base selector's role.
        let candidates: Vec<&Format> = formats
            .iter()
            .filter(|f| self.base.matches_role(f))
            .filter(|f| self.filters.iter().all(|flt| flt.matches(f)))
            .collect();
        if candidates.is_empty() {
            return None;
        }
        let base = &self.base;
        if let Base::Id(_) = base {
            return candidates.into_iter().next();
        }
        if base.is_worst() {
            candidates
                .into_iter()
                .min_by(|a, c| rank(base, a).total_cmp(&rank(base, c)))
        } else {
            candidates
                .into_iter()
                .max_by(|a, c| rank(base, a).total_cmp(&rank(base, c)))
        }
    }
}

impl Base {
    fn is_worst(&self) -> bool {
        matches!(
            self,
            Base::WorstMuxed | Base::WorstVideoOnly | Base::WorstAudioOnly | Base::WorstAny
        )
    }

    fn matches_role(&self, f: &Format) -> bool {
        match self {
            Base::BestMuxed | Base::WorstMuxed => f.is_muxed(),
            Base::BestVideoOnly | Base::WorstVideoOnly => f.is_video_only(),
            Base::BestAudioOnly | Base::WorstAudioOnly => f.is_audio_only(),
            Base::BestWithVideo => f.has_video(),
            Base::BestWithAudio => f.has_audio(),
            Base::BestAny | Base::WorstAny => true,
            Base::Id(id) => &f.format_id == id,
        }
    }
}

/// Ranking score; higher is better. Audio-oriented selectors rank by bitrate,
/// everything else by resolution then bitrate then fps.
fn rank(base: &Base, f: &Format) -> f64 {
    let audio_oriented = matches!(base, Base::BestAudioOnly | Base::WorstAudioOnly | Base::BestWithAudio);
    if audio_oriented {
        let abr = f.abr.or(f.effective_tbr()).unwrap_or(0.0);
        let asr = f.asr.unwrap_or(0) as f64;
        return abr * 1000.0 + asr / 1000.0;
    }
    if let Some(q) = f.quality {
        return q * 1e12 + f.effective_tbr().unwrap_or(0.0);
    }
    let height = f.height.unwrap_or(0) as f64;
    let fps = f.fps.unwrap_or(0.0);
    let tbr = f.effective_tbr().unwrap_or(0.0);
    height * 1e6 + tbr * 100.0 + fps
}

impl Filter {
    fn matches(&self, f: &Format) -> bool {
        if let Some(num) = self.numeric_field(f) {
            let Ok(rhs) = self.value.parse::<f64>().or_else(|_| parse_size(&self.value)) else {
                return false;
            };
            return match self.op {
                Op::Lt => num < rhs,
                Op::Le => num <= rhs,
                Op::Gt => num > rhs,
                Op::Ge => num >= rhs,
                Op::Eq => num == rhs,
                Op::Ne => num != rhs,
                // String ops on numeric fields never match.
                _ => false,
            };
        }
        let Some(s) = self.string_field(f) else {
            return false;
        };
        match self.op {
            Op::Eq => s == self.value,
            Op::Ne => s != self.value,
            Op::StartsWith => s.starts_with(&self.value),
            Op::EndsWith => s.ends_with(&self.value),
            Op::Contains => s.contains(&self.value),
            _ => false,
        }
    }

    fn numeric_field(&self, f: &Format) -> Option<f64> {
        match self.field.as_str() {
            "height" => f.height.map(|v| v as f64),
            "width" => f.width.map(|v| v as f64),
            "fps" => f.fps,
            "tbr" => f.tbr,
            "abr" => f.abr,
            "vbr" => f.vbr,
            "asr" => f.asr.map(|v| v as f64),
            "filesize" => f.filesize.map(|v| v as f64),
            "filesize_approx" => f.filesize_approx.map(|v| v as f64),
            _ => None,
        }
    }

    fn string_field(&self, f: &Format) -> Option<String> {
        match self.field.as_str() {
            "ext" => Some(f.ext.clone()),
            "format_id" => Some(f.format_id.clone()),
            "vcodec" => f.vcodec.clone(),
            "acodec" => f.acodec.clone(),
            "container" => f.container.clone(),
            "language" => f.language.clone(),
            "protocol" => Some(format!("{:?}", f.protocol).to_lowercase()),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vid(id: &str, h: u32, vcodec: &str, ext: &str, tbr: f64) -> Format {
        Format {
            format_id: id.into(),
            ext: ext.into(),
            height: Some(h),
            vcodec: Some(vcodec.into()),
            acodec: Some("none".into()),
            tbr: Some(tbr),
            ..Default::default()
        }
    }
    fn aud(id: &str, ext: &str, abr: f64) -> Format {
        Format {
            format_id: id.into(),
            ext: ext.into(),
            vcodec: Some("none".into()),
            acodec: Some("mp4a.40.2".into()),
            abr: Some(abr),
            ..Default::default()
        }
    }
    fn muxed(id: &str, h: u32, ext: &str, tbr: f64) -> Format {
        Format {
            format_id: id.into(),
            ext: ext.into(),
            height: Some(h),
            vcodec: Some("avc1".into()),
            acodec: Some("mp4a.40.2".into()),
            tbr: Some(tbr),
            ..Default::default()
        }
    }

    fn pool() -> Vec<Format> {
        vec![
            vid("137", 1080, "avc1", "mp4", 4000.0),
            vid("248", 1080, "vp9", "webm", 3000.0),
            vid("136", 720, "avc1", "mp4", 2000.0),
            aud("140", "m4a", 128.0),
            aud("251", "webm", 160.0),
            muxed("18", 360, "mp4", 600.0),
            muxed("22", 720, "mp4", 1500.0),
        ]
    }

    fn ids(sel: &Selection) -> Vec<String> {
        sel.formats.iter().map(|f| f.format_id.clone()).collect()
    }

    #[test]
    fn default_selector_merges_best_video_and_audio() {
        let sel = FormatSelector::parse("bv*+ba/b").unwrap().select(&pool()).unwrap();
        // bv* = best with video (1080 avc1 @4000 beats vp9 @3000), ba = 251 @160.
        assert_eq!(ids(&sel), vec!["137", "251"]);
        assert!(sel.needs_merge());
    }

    #[test]
    fn fallback_to_muxed_when_no_adaptive() {
        let only_muxed = vec![muxed("18", 360, "mp4", 600.0), muxed("22", 720, "mp4", 1500.0)];
        let sel = FormatSelector::parse("bv+ba/b").unwrap().select(&only_muxed).unwrap();
        // No video-only/audio-only present, so the +-merge fails and we fall to b.
        assert_eq!(ids(&sel), vec!["22"]);
        assert!(!sel.needs_merge());
    }

    #[test]
    fn height_filter_caps_resolution() {
        let sel = FormatSelector::parse("bv*[height<=720]").unwrap().select(&pool()).unwrap();
        assert_eq!(ids(&sel), vec!["136"]);
    }

    #[test]
    fn ext_filter_and_codec_prefix() {
        let sel = FormatSelector::parse("bv*[ext=webm][vcodec^=vp9]")
            .unwrap()
            .select(&pool())
            .unwrap();
        assert_eq!(ids(&sel), vec!["248"]);
    }

    #[test]
    fn explicit_id_selection() {
        let sel = FormatSelector::parse("136+140").unwrap().select(&pool()).unwrap();
        assert_eq!(ids(&sel), vec!["136", "140"]);
    }

    #[test]
    fn unavailable_returns_none() {
        let sel = FormatSelector::parse("999").unwrap().select(&pool());
        assert!(sel.is_none());
    }

    #[test]
    fn size_suffix_parses() {
        assert_eq!(parse_size("50M").unwrap(), 50e6);
        assert_eq!(parse_size("1.5G").unwrap(), 1.5e9);
    }
}

/// Parse human sizes like `50M`, `1.5G`, `700k` into bytes.
fn parse_size(s: &str) -> Result<f64, std::num::ParseFloatError> {
    let s = s.trim();
    let (num, mult) = match s.chars().last() {
        Some('k') | Some('K') => (&s[..s.len() - 1], 1e3),
        Some('m') | Some('M') => (&s[..s.len() - 1], 1e6),
        Some('g') | Some('G') => (&s[..s.len() - 1], 1e9),
        _ => (s, 1.0),
    };
    Ok(num.trim().parse::<f64>()? * mult)
}
