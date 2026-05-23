//! Pluggable JavaScript execution for solving YouTube's `base.js` challenges
//! (signature and `n` parameter descrambling).
//!
//! YouTube's player code is too complex for a hand-written interpreter to keep
//! up with, so we run the real extracted functions in a real engine. The engine
//! is hidden behind [`JsRuntime`] so the default (embedded QuickJS, no external
//! dependency) can be swapped for an external runtime (Deno/Node/Bun/QuickJS)
//! when heavier challenges (e.g. BotGuard / PO tokens) demand it.

use anyhow::Result;

mod quickjs;
pub use quickjs::QuickJsRuntime;

mod external;
pub use external::{ExternalRuntime, ExternalRuntimeKind};

/// A JavaScript execution backend.
///
/// Implementations must be cheap to share across threads. The contract is
/// deliberately tiny: evaluate a self-contained snippet and return the value of
/// its final expression coerced to a string. The caller (the extractor) is
/// responsible for assembling a self-contained snippet — concatenating any
/// player global var, helper objects, the target function definition, and the
/// invocation — because only it knows how to carve those out of `base.js`.
pub trait JsRuntime: Send + Sync {
    /// Evaluate `code` and return the final expression's value as a string.
    fn eval(&self, code: &str) -> Result<String>;

    /// Human-readable name of the backend, for diagnostics.
    fn name(&self) -> &'static str;
}

/// Convenience: define `code` (helpers + a function), then call
/// `func_name(arg)` and return the result as a string. `arg` is JSON-encoded so
/// arbitrary strings pass through safely.
pub fn call_string_fn(
    rt: &dyn JsRuntime,
    code: &str,
    func_name: &str,
    arg: &str,
) -> Result<String> {
    let snippet = format!("{code}\n{func_name}({});", json_string(arg));
    rt.eval(&snippet)
}

/// Minimal JSON string encoder (avoids pulling serde_json into this crate just
/// for one call site).
fn json_string(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A reverse+swap+splice routine structurally identical to YouTube's
    /// signature transform, to prove the engine actually runs extracted code.
    const SIG_LIKE: &str = r#"
        var helper = {
            reverse: function(a){ a.reverse(); },
            swap: function(a, b){ var c = a[0]; a[0] = a[b % a.length]; a[b % a.length] = c; },
            splice: function(a, b){ a.splice(0, b); }
        };
        function descramble(sig){
            var a = sig.split("");
            helper.reverse(a, 0);
            helper.swap(a, 3);
            helper.splice(a, 2);
            helper.swap(a, 1);
            return a.join("");
        }
    "#;

    #[test]
    fn quickjs_runs_signature_like_function() {
        let rt = QuickJsRuntime::new();
        let out = call_string_fn(&rt, SIG_LIKE, "descramble", "abcdefgh").unwrap();
        // Mirror the algorithm in Rust to assert exact agreement.
        let mut a: Vec<char> = "abcdefgh".chars().collect();
        a.reverse();
        let n = a.len();
        a.swap(0, 3 % n);
        a.drain(0..2);
        let n2 = a.len();
        a.swap(0, 1 % n2);
        let expected: String = a.into_iter().collect();
        assert_eq!(out, expected);
    }

    #[test]
    fn quickjs_evaluates_expression() {
        let rt = QuickJsRuntime::new();
        assert_eq!(rt.eval("1 + 2 + 3").unwrap(), "6");
        assert_eq!(rt.eval("'a' + 'b'").unwrap(), "ab");
    }

    #[test]
    fn quickjs_reports_exceptions() {
        let rt = QuickJsRuntime::new();
        let err = rt.eval("throw new Error('boom')").unwrap_err();
        assert!(err.to_string().contains("boom"), "got: {err}");
    }
}
