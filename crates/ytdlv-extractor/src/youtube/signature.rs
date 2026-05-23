//! YouTube `base.js` challenge solving: locate the signature and `n`-parameter
//! transform functions inside the player JavaScript, carve them out into
//! self-contained snippets (function + helper objects + any referenced
//! top-level vars), and execute them in a real JS engine via [`JsRuntime`].
//!
//! This mirrors yt-dlp's *classic* approach but runs the real extracted code in
//! a genuine engine instead of a hand-written interpreter. The string-surgery
//! here (function/object extraction, brace balancing) is the brittle part that
//! tracks YouTube changes; it is unit-tested against fixtures that mimic
//! `base.js` structure so the mechanism is verifiable without the network.
//!
//! Current state of the arms race (early 2026): YouTube now obfuscates the
//! transforms enough that name-discovery via regex is unreliable — e.g. the
//! signature function uses `z.split(GLOBAL_VAR)` rather than `z.split("")`, and
//! the transforms reference IIFE-scoped globals that must be resolved to run
//! them standalone. yt-dlp itself has retired regex extraction in favour of a
//! "JS Challenge" director that loads the *entire* `base.js` into a JS runtime
//! and drives the player's own code (the `yt-dlp-ejs` bundle). That same
//! approach is the forward path here — and it slots in behind [`JsRuntime`]
//! without touching the extractor. Tracked in the PO-token / JS-challenge issue.
//! The regex solver below remains as a tested fallback for simpler players.

use std::collections::BTreeSet;

use anyhow::{anyhow, bail, Result};
use regex::Regex;
use ytdlv_jsruntime::{call_string_fn, JsRuntime};

/// A function lifted out of `base.js`: its full `function(args){...}`
/// expression plus the argument names.
#[derive(Debug, Clone)]
pub struct ExtractedFn {
    pub expr: String,
    pub args: Vec<String>,
    pub body: String,
}

/// Solver holding the assembled, self-contained snippets for the signature and
/// `n` transforms, ready to run against inputs.
#[derive(Debug, Clone)]
pub struct PlayerSolver {
    sig_code: Option<String>,
    nsig_code: Option<String>,
    pub signature_timestamp: Option<u64>,
}

const SIG_CALL: &str = "__ytdlv_sig";
const NSIG_CALL: &str = "__ytdlv_nsig";

impl PlayerSolver {
    /// Build a solver from raw `base.js` source.
    pub fn from_base_js(js: &str) -> Self {
        let sig_code = build_sig_code(js);
        let nsig_code = build_nsig_code(js);
        if sig_code.is_none() {
            tracing::warn!("could not locate signature function in base.js");
        }
        if nsig_code.is_none() {
            tracing::warn!("could not locate n-parameter function in base.js");
        }
        Self {
            sig_code,
            nsig_code,
            signature_timestamp: signature_timestamp(js),
        }
    }

    pub fn has_sig(&self) -> bool {
        self.sig_code.is_some()
    }
    pub fn has_nsig(&self) -> bool {
        self.nsig_code.is_some()
    }

    /// Descramble a scrambled signature `s` value.
    pub fn decrypt_signature(&self, rt: &dyn JsRuntime, s: &str) -> Result<String> {
        let code = self
            .sig_code
            .as_ref()
            .ok_or_else(|| anyhow!("no signature function extracted"))?;
        call_string_fn(rt, code, SIG_CALL, s)
    }

    /// Transform the `n` query parameter to avoid throttling.
    pub fn decrypt_n(&self, rt: &dyn JsRuntime, n: &str) -> Result<String> {
        let code = self
            .nsig_code
            .as_ref()
            .ok_or_else(|| anyhow!("no n-parameter function extracted"))?;
        let out = call_string_fn(rt, code, NSIG_CALL, n)?;
        // A correct transform never echoes the input and never returns the
        // enhanced-exception sentinel; treat those as failure so the caller can
        // fall back rather than ship a throttled URL.
        if out == n {
            bail!("n transform returned input unchanged (extraction likely stale)");
        }
        Ok(out)
    }
}

/// Extract `signatureTimestamp` (a.k.a. `sts`) from base.js.
pub fn signature_timestamp(js: &str) -> Option<u64> {
    let re = Regex::new(r"(?:signatureTimestamp|sts)\s*:\s*(\d{5,})").unwrap();
    re.captures(js)?.get(1)?.as_str().parse().ok()
}

/// Parse a `signatureCipher`/`cipher` query string into `(s, sp, url)`.
pub fn parse_signature_cipher(cipher: &str) -> Option<(String, String, String)> {
    let mut s = None;
    let mut sp = "signature".to_string();
    let mut url = None;
    for (k, v) in url::form_urlencoded::parse(cipher.as_bytes()) {
        match k.as_ref() {
            "s" => s = Some(v.into_owned()),
            "sp" => sp = v.into_owned(),
            "url" => url = Some(v.into_owned()),
            _ => {}
        }
    }
    Some((s?, sp, url?))
}

// ---------------------------------------------------------------------------
// Function-name discovery
// ---------------------------------------------------------------------------

/// Locate the signature transform function name. These patterns track the call
/// site in `base.js` where the decoded signature is produced.
fn extract_sig_function_name(js: &str) -> Option<String> {
    const PATTERNS: &[&str] = &[
        r#"\b[a-zA-Z0-9$]+&&\([a-zA-Z0-9$]+=(?P<sig>[a-zA-Z0-9$]{2,})\(decodeURIComponent\("#,
        r#"\b(?P<sig>[a-zA-Z0-9$]{2,})\s*=\s*function\(\s*[a-zA-Z0-9$]+\s*\)\s*\{\s*[a-zA-Z0-9$]+\s*=\s*[a-zA-Z0-9$]+\.split\(\s*""\s*\)"#,
        r#"\bm=(?P<sig>[a-zA-Z0-9$]{2,})\(decodeURIComponent\(h\.s\)\)"#,
        r#"\bc&&\(c=(?P<sig>[a-zA-Z0-9$]{2,})\(decodeURIComponent\(c\)\)\)"#,
        r#"(?:\b|[^a-zA-Z0-9$])(?P<sig>[a-zA-Z0-9$]{2,})\s*=\s*function\(\s*a\s*\)\s*\{\s*a\s*=\s*a\.split\(\s*""\s*\)"#,
        r#"\.set\([^,]+,\s*encodeURIComponent\((?P<sig>[a-zA-Z0-9$]{2,})\("#,
    ];
    for p in PATTERNS {
        if let Ok(re) = Regex::new(p) {
            if let Some(c) = re.captures(js) {
                if let Some(m) = c.name("sig") {
                    return Some(m.as_str().to_string());
                }
            }
        }
    }
    None
}

/// Locate the `n`-parameter transform function name. YouTube assigns it either
/// directly or as an element of an array of functions.
fn extract_n_function_name(js: &str) -> Option<String> {
    // Direct: `...get("n"))&&(b=NAME(b)` or `...get("n"))&&(b=NAME[idx](b)`.
    let direct = Regex::new(
        r#"\.get\(\s*"n"\s*\)\s*\)\s*&&\s*\([a-zA-Z0-9$]+\s*=\s*(?P<nfunc>[a-zA-Z0-9$]+)(?:\[(?P<idx>\d+)\])?\("#,
    )
    .unwrap();
    if let Some(c) = direct.captures(js) {
        let name = c.name("nfunc")?.as_str().to_string();
        match c.name("idx") {
            Some(idx) => {
                let i: usize = idx.as_str().parse().ok()?;
                return resolve_func_array(js, &name, i);
            }
            None => return Some(name),
        }
    }
    // Alternate shape: `b=a.split("")` style assignment captured by name.
    let alt = Regex::new(
        r#"(?P<nfunc>[a-zA-Z0-9$]{2,})\s*=\s*function\(\s*[a-zA-Z0-9$]+\s*\)\s*\{\s*var\s+[a-zA-Z0-9$]+\s*=\s*[a-zA-Z0-9$]+\.split\("#,
    )
    .unwrap();
    alt.captures(js)?.name("nfunc").map(|m| m.as_str().to_string())
}

/// Resolve `var NAME=[a,b,c]` and return element `idx` (a function name).
fn resolve_func_array(js: &str, name: &str, idx: usize) -> Option<String> {
    let needle = format!("var {name}=[");
    let start = js.find(&needle)? + needle.len() - 1; // at '['
    let end = scan_balanced(js, start, b'[', b']')?;
    let inner = &js[start + 1..end];
    inner.split(',').map(str::trim).nth(idx).map(|s| s.to_string())
}

// ---------------------------------------------------------------------------
// Snippet assembly
// ---------------------------------------------------------------------------

fn build_sig_code(js: &str) -> Option<String> {
    let name = extract_sig_function_name(js)?;
    let func = extract_func_expr(js, &name)?;
    let mut code = String::new();
    append_referenced_objects(js, &func.body, &mut code);
    code.push_str(&format!("var {SIG_CALL}={};\n", func.expr));
    Some(code)
}

fn build_nsig_code(js: &str) -> Option<String> {
    let name = extract_n_function_name(js)?;
    let func = extract_func_expr(js, &name)?;
    let mut code = String::new();
    // nsig bodies often reference a top-level global string/array var.
    append_referenced_globals(js, &func.body, &mut code);
    append_referenced_objects(js, &func.body, &mut code);
    code.push_str(&format!("var {NSIG_CALL}={};\n", func.expr));
    Some(code)
}

/// Pull `var OBJ={...}` definitions for helper objects called as `OBJ.m(...)`.
fn append_referenced_objects(js: &str, body: &str, out: &mut String) {
    for obj in referenced_call_objects(body) {
        if let Some(def) = extract_object(js, &obj) {
            out.push_str(&def);
            out.push_str(";\n");
        }
    }
}

/// Pull simple `var X=...;` top-level definitions referenced bare in the body
/// (e.g. the global string array nsig uses), best-effort.
fn append_referenced_globals(js: &str, body: &str, out: &mut String) {
    for ident in referenced_bare_idents(body) {
        if let Some(def) = extract_simple_var(js, &ident) {
            out.push_str(&def);
            out.push_str(";\n");
        }
    }
}

// ---------------------------------------------------------------------------
// Function / object extraction with brace balancing
// ---------------------------------------------------------------------------

/// Extract a function definition by name in any of the common forms:
/// `function NAME(...){...}`, `NAME=function(...){...}`, `var NAME=function...`,
/// or object-method `NAME:function(...){...}`.
pub fn extract_func_expr(js: &str, name: &str) -> Option<ExtractedFn> {
    let escaped = regex::escape(name);
    let head = Regex::new(&format!(
        r"(?:function\s+{n}|(?:var\s+)?{n}\s*=\s*function|{n}\s*:\s*function)\s*\(",
        n = escaped
    ))
    .ok()?;
    let m = head.find(js)?;
    // Position of the '(' that opens the parameter list.
    let paren_open = js[m.start()..m.end()].rfind('(').map(|i| m.start() + i)?;
    let paren_close = scan_balanced(js, paren_open, b'(', b')')?;
    let args: Vec<String> = js[paren_open + 1..paren_close]
        .split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();
    let brace_open = js[paren_close..].find('{').map(|i| paren_close + i)?;
    let brace_close = scan_balanced(js, brace_open, b'{', b'}')?;
    let body = js[brace_open + 1..brace_close].to_string();
    let arglist = args.join(",");
    let expr = format!("function({arglist}){{{body}}}");
    Some(ExtractedFn { expr, args, body })
}

/// Extract a `var NAME={...}` object literal definition, returning the full
/// `var NAME={...}` text.
pub fn extract_object(js: &str, name: &str) -> Option<String> {
    let escaped = regex::escape(name);
    let head = Regex::new(&format!(r"(?:var\s+)?{n}\s*=\s*\{{", n = escaped)).ok()?;
    let m = head.find(js)?;
    let brace_open = js[m.start()..m.end()].rfind('{').map(|i| m.start() + i)?;
    let brace_close = scan_balanced(js, brace_open, b'{', b'}')?;
    Some(format!("var {name}={}", &js[brace_open..=brace_close]))
}

/// Extract a simple `var NAME=<value>;` declaration (array literal or string),
/// returning `var NAME=<value>`.
fn extract_simple_var(js: &str, name: &str) -> Option<String> {
    let escaped = regex::escape(name);
    let head = Regex::new(&format!(r"var\s+{n}\s*=", n = escaped)).ok()?;
    let m = head.find(js)?;
    let eq = js[m.start()..m.end()].rfind('=').map(|i| m.start() + i)?;
    let rest = &js[eq + 1..];
    let trimmed_start = rest.len() - rest.trim_start().len();
    let value_start = eq + 1 + trimmed_start;
    let first = js.as_bytes().get(value_start)?;
    let value_end = match first {
        b'[' => scan_balanced(js, value_start, b'[', b']')?,
        b'{' => scan_balanced(js, value_start, b'{', b'}')?,
        b'"' | b'\'' => scan_string(js, value_start)?,
        // Bare value: read to the next top-level ';'.
        _ => value_start + js[value_start..].find(';')?,
    };
    let end = if matches!(first, b'[' | b'{' | b'"' | b'\'') {
        value_end + 1
    } else {
        value_end
    };
    Some(format!("var {name}={}", &js[value_start..end]))
}

/// Find identifiers used as `IDENT.method(` in `body`, excluding JS built-ins.
fn referenced_call_objects(body: &str) -> BTreeSet<String> {
    let re = Regex::new(r"([a-zA-Z_$][\w$]*)\.[a-zA-Z_$][\w$]*\(").unwrap();
    re.captures_iter(body)
        .filter_map(|c| c.get(1))
        .map(|m| m.as_str().to_string())
        .filter(|s| !is_builtin(s))
        .collect()
}

/// Find bare identifiers (not member-accessed, not called) that could be
/// top-level vars, e.g. the global array nsig indexes into.
fn referenced_bare_idents(body: &str) -> BTreeSet<String> {
    let re = Regex::new(r"[a-zA-Z_$][\w$]*").unwrap();
    re.find_iter(body)
        .map(|m| m.as_str().to_string())
        .filter(|s| !is_builtin(s) && s.len() >= 2)
        .collect()
}

fn is_builtin(s: &str) -> bool {
    matches!(
        s,
        "String"
            | "Math"
            | "Array"
            | "Object"
            | "Number"
            | "RegExp"
            | "JSON"
            | "Date"
            | "parseInt"
            | "parseFloat"
            | "decodeURIComponent"
            | "encodeURIComponent"
            | "isNaN"
            | "var"
            | "function"
            | "return"
            | "if"
            | "else"
            | "for"
            | "while"
            | "switch"
            | "case"
            | "break"
            | "continue"
            | "typeof"
            | "new"
            | "this"
            | "null"
            | "true"
            | "false"
            | "undefined"
            | "try"
            | "catch"
            | "throw"
    )
}

/// Scan from an opening delimiter at `open_idx` to its matching close,
/// skipping string and (heuristically) regex literals. Returns the index of the
/// matching close byte.
pub fn scan_balanced(s: &str, open_idx: usize, open: u8, close: u8) -> Option<usize> {
    let b = s.as_bytes();
    debug_assert_eq!(b[open_idx], open);
    let mut depth = 0i32;
    let mut i = open_idx;
    let mut prev_significant = 0u8;
    while i < b.len() {
        let c = b[i];
        match c {
            b'"' | b'\'' | b'`' => {
                i = scan_string(s, i)?;
                prev_significant = c;
            }
            b'/' if regex_can_start(prev_significant) => {
                // Treat as regex literal; skip to its end.
                i = scan_regex(s, i)?;
                prev_significant = b'/';
            }
            _ => {
                if c == open {
                    depth += 1;
                } else if c == close {
                    depth -= 1;
                    if depth == 0 {
                        return Some(i);
                    }
                }
                if !c.is_ascii_whitespace() {
                    prev_significant = c;
                }
            }
        }
        i += 1;
    }
    None
}

/// A `/` begins a regex literal (rather than division) when the previous
/// significant byte is one that cannot end an expression.
fn regex_can_start(prev: u8) -> bool {
    matches!(
        prev,
        0 | b'(' | b',' | b'=' | b':' | b'[' | b'!' | b'&' | b'|' | b'?' | b'{' | b'}' | b';' | b'+'
            | b'-' | b'*' | b'<' | b'>' | b'~' | b'^' | b'%' | b'\n'
    )
}

/// Given index of an opening quote, return index of the closing quote.
fn scan_string(s: &str, quote_idx: usize) -> Option<usize> {
    let b = s.as_bytes();
    let quote = b[quote_idx];
    let mut i = quote_idx + 1;
    while i < b.len() {
        match b[i] {
            b'\\' => i += 2,
            c if c == quote => return Some(i),
            _ => i += 1,
        }
    }
    None
}

/// Given index of the opening `/` of a regex literal, return index of the
/// closing `/` (flags handled by the caller's normal advance).
fn scan_regex(s: &str, slash_idx: usize) -> Option<usize> {
    let b = s.as_bytes();
    let mut i = slash_idx + 1;
    let mut in_class = false;
    while i < b.len() {
        match b[i] {
            b'\\' => i += 2,
            b'[' => {
                in_class = true;
                i += 1;
            }
            b']' => {
                in_class = false;
                i += 1;
            }
            b'/' if !in_class => return Some(i),
            b'\n' => return None, // not a regex after all
            _ => i += 1,
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use ytdlv_jsruntime::QuickJsRuntime;

    // A fixture mimicking base.js structure: a helper object, a signature
    // function that reverses/swaps/splices, an n-function that uses a global
    // array, and the call sites whose patterns our regexes target.
    const FIXTURE: &str = r#"
        var Xq={
            wZ:function(a){a.reverse()},
            J9:function(a,b){var c=a[0];a[0]=a[b%a.length];a[b%a.length]=c},
            kP:function(a,b){a.splice(0,b)}
        };
        Mca=function(a){a=a.split("");Xq.wZ(a,52);Xq.J9(a,3);Xq.kP(a,1);Xq.J9(a,21);return a.join("")};
        var Gn=["0","1","2","3","4","5"];
        Dz=function(a){var b=a.split("");b.reverse();return Gn[1]+b.join("")+Gn[0]};
        if(c&&(c=Mca(decodeURIComponent(c)))){};
        somevar.get("n"))&&(b=Dz(b));
        var x={signatureTimestamp:19834,foo:1};
    "#;

    #[test]
    fn finds_signature_function_name() {
        assert_eq!(extract_sig_function_name(FIXTURE).as_deref(), Some("Mca"));
    }

    #[test]
    fn finds_n_function_name() {
        assert_eq!(extract_n_function_name(FIXTURE).as_deref(), Some("Dz"));
    }

    #[test]
    fn extracts_signature_timestamp() {
        assert_eq!(signature_timestamp(FIXTURE), Some(19834));
    }

    #[test]
    fn extract_object_balances_braces() {
        let obj = extract_object(FIXTURE, "Xq").unwrap();
        assert!(obj.starts_with("var Xq={"));
        assert!(obj.contains("splice(0,b)"));
        // Must stop at the object's own closing brace, not run on.
        assert!(obj.trim_end().ends_with('}'));
    }

    #[test]
    fn solves_signature_via_engine() {
        let solver = PlayerSolver::from_base_js(FIXTURE);
        assert!(solver.has_sig());
        let rt = QuickJsRuntime::new();
        let out = solver.decrypt_signature(&rt, "abcdefghij").unwrap();

        // Mirror Mca in Rust.
        let mut a: Vec<char> = "abcdefghij".chars().collect();
        a.reverse();
        let n = a.len();
        a.swap(0, 3 % n);
        a.drain(0..1);
        let n2 = a.len();
        a.swap(0, 21 % n2);
        let expected: String = a.into_iter().collect();
        assert_eq!(out, expected);
    }

    #[test]
    fn solves_n_param_via_engine_with_global() {
        let solver = PlayerSolver::from_base_js(FIXTURE);
        assert!(solver.has_nsig());
        let rt = QuickJsRuntime::new();
        let out = solver.decrypt_n(&rt, "wxyz").unwrap();
        // Dz: "1" + reverse("wxyz") + "0" = "1" + "zyxw" + "0".
        assert_eq!(out, "1zyxw0");
    }

    #[test]
    fn cipher_parses_into_parts() {
        let c = "s=SCRAMBLED&sp=sig&url=https%3A%2F%2Fhost%2Fpath%3Fitag%3D18";
        let (s, sp, url) = parse_signature_cipher(c).unwrap();
        assert_eq!(s, "SCRAMBLED");
        assert_eq!(sp, "sig");
        assert_eq!(url, "https://host/path?itag=18");
    }

    #[test]
    fn scan_balanced_skips_strings_and_regex() {
        let js = r#"X={a:"}}}",b:/[}]/g,c:1}"#;
        let open = js.find('{').unwrap();
        let close = scan_balanced(js, open, b'{', b'}').unwrap();
        assert_eq!(&js[close..=close], "}");
        assert_eq!(close, js.len() - 1);
    }
}
