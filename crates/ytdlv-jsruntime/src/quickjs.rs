//! Default backend: QuickJS embedded in the binary via `rquickjs`. No external
//! runtime required.

use anyhow::{anyhow, Result};
use rquickjs::{CatchResultExt, Coerced, Context, Runtime};

/// Embedded QuickJS engine.
///
/// A fresh [`Runtime`]/[`Context`] is created per evaluation. QuickJS startup is
/// sub-millisecond and the extractor caches solved sig/nsig results per player,
/// so the cost is negligible — and creating per call sidesteps QuickJS's
/// single-threaded affinity, letting this type be freely `Send + Sync`.
#[derive(Debug, Default, Clone, Copy)]
pub struct QuickJsRuntime {
    /// Soft memory ceiling in bytes (0 = engine default / unlimited).
    memory_limit: usize,
}

impl QuickJsRuntime {
    pub fn new() -> Self {
        // 64 MiB is comfortably more than any nsig transform needs while still
        // bounding a runaway/hostile script.
        Self {
            memory_limit: 64 * 1024 * 1024,
        }
    }

    pub fn with_memory_limit(memory_limit: usize) -> Self {
        Self { memory_limit }
    }
}

impl super::JsRuntime for QuickJsRuntime {
    fn eval(&self, code: &str) -> Result<String> {
        let rt = Runtime::new().map_err(|e| anyhow!("quickjs init: {e}"))?;
        if self.memory_limit > 0 {
            rt.set_memory_limit(self.memory_limit);
        }
        let ctx = Context::full(&rt).map_err(|e| anyhow!("quickjs context: {e}"))?;

        ctx.with(
            |ctx| match ctx.eval::<Coerced<String>, _>(code).catch(&ctx) {
                Ok(v) => Ok(v.0),
                Err(caught) => Err(anyhow!("{caught}")),
            },
        )
    }

    fn name(&self) -> &'static str {
        "quickjs (embedded)"
    }
}
