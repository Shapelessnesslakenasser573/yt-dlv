//! Escape-hatch backend: shell out to an external JS runtime.
//!
//! Not used by default. This exists so that when YouTube's challenges outgrow
//! what we want to run in-process (notably BotGuard / PO-token attestation,
//! whose community solver scripts target Node/Deno/Bun), we can route through a
//! full engine without changing the extractor — it still just sees a
//! [`JsRuntime`]. See the PO-token tracking issue for the wiring that builds on
//! this.

use std::io::Write;
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

use anyhow::{anyhow, bail, Context, Result};

/// Which external runtime to invoke.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExternalRuntimeKind {
    Deno,
    Node,
    Bun,
    /// The standalone QuickJS interpreter (`qjs`).
    QuickJs,
}

impl ExternalRuntimeKind {
    pub fn parse(s: &str) -> Option<Self> {
        match s.to_ascii_lowercase().as_str() {
            "deno" => Some(Self::Deno),
            "node" => Some(Self::Node),
            "bun" => Some(Self::Bun),
            "quickjs" | "qjs" => Some(Self::QuickJs),
            _ => None,
        }
    }

    fn default_binary(self) -> &'static str {
        match self {
            Self::Deno => "deno",
            Self::Node => "node",
            Self::Bun => "bun",
            Self::QuickJs => "qjs",
        }
    }

    fn args(self, script: &str) -> Vec<String> {
        match self {
            Self::Deno => vec!["run".into(), "--quiet".into(), script.into()],
            Self::Node | Self::Bun | Self::QuickJs => vec![script.into()],
        }
    }
}

/// Runs JS by spawning an external interpreter on a temp driver script.
#[derive(Debug, Clone)]
pub struct ExternalRuntime {
    kind: ExternalRuntimeKind,
    binary: String,
}

impl ExternalRuntime {
    pub fn new(kind: ExternalRuntimeKind) -> Self {
        Self {
            kind,
            binary: kind.default_binary().to_string(),
        }
    }

    /// Override the binary path (e.g. a non-PATH install).
    pub fn with_binary(kind: ExternalRuntimeKind, binary: impl Into<String>) -> Self {
        Self {
            kind,
            binary: binary.into(),
        }
    }
}

static COUNTER: AtomicU64 = AtomicU64::new(0);

impl super::JsRuntime for ExternalRuntime {
    fn eval(&self, code: &str) -> Result<String> {
        // Indirect `eval` returns the completion value of the final statement,
        // matching QuickJsRuntime's `ctx.eval` semantics across all runtimes.
        let driver = format!(
            "const __code = {code};\n\
             const __r = (0, eval)(__code);\n\
             console.log((__r === undefined || __r === null) ? \"\" : String(__r));\n",
            code = super::json_string(code),
        );

        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!("ytdlv-js-{}-{n}.js", std::process::id()));
        {
            let mut f = std::fs::File::create(&path)
                .with_context(|| format!("creating temp script {}", path.display()))?;
            f.write_all(driver.as_bytes())?;
        }
        let script = path.to_string_lossy().into_owned();

        let result = Command::new(&self.binary)
            .args(self.kind.args(&script))
            .output();
        let _ = std::fs::remove_file(&path);

        let out = result
            .map_err(|e| anyhow!("failed to spawn external runtime '{}': {e}", self.binary))?;

        if !out.status.success() {
            bail!(
                "external runtime '{}' failed ({}): {}",
                self.binary,
                out.status,
                String::from_utf8_lossy(&out.stderr).trim()
            );
        }

        let mut s = String::from_utf8_lossy(&out.stdout).into_owned();
        if s.ends_with('\n') {
            s.pop();
            if s.ends_with('\r') {
                s.pop();
            }
        }
        Ok(s)
    }

    fn name(&self) -> &'static str {
        match self.kind {
            ExternalRuntimeKind::Deno => "deno (external)",
            ExternalRuntimeKind::Node => "node (external)",
            ExternalRuntimeKind::Bun => "bun (external)",
            ExternalRuntimeKind::QuickJs => "quickjs (external)",
        }
    }
}
