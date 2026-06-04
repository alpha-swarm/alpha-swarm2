//! WASM tool bridge: lets agents call WebAssembly tool components hosted by an
//! embedded Wassette runtime, WITHOUT this crate depending on wasmtime.
//!
//! The concrete host (wasmtime) lives in `tool-host` and implements
//! [`WasmToolHost`]; the daemon installs it into a process-global set on
//! startup (mirroring how the embedded ruvector index is a global OnceLock).
//! `ToolRegistry::with_wasm_tools()` then surfaces each configured WASM tool as
//! a normal [`Tool`], so the agent prompt + dispatch path treat it like any
//! other tool. Keeps `swarm-tools` WASI-portable (only a trait object here).

use std::sync::{Arc, OnceLock};

use serde_json::Value;

use crate::{Tool, ToolContext, ToolResult};

/// Implemented by the embedded Wassette runtime (`tool-host`). Invokes an
/// exported component function with a JSON-object argument string and returns
/// the JSON result envelope (`{"result":{"ok"|"err":...}}`).
#[async_trait::async_trait]
pub trait WasmToolHost: Send + Sync {
    async fn call(&self, component_id: &str, function: &str, params_json: &str) -> Result<String, String>;
    /// Idempotently grant a component filesystem READ on `dir`. Called lazily
    /// per tool invocation so fs-backed WASM tools work on the run's actual
    /// repo (which isn't known at startup); deny-by-default otherwise.
    async fn ensure_read(&self, component_id: &str, dir: &str) -> Result<(), String>;
}

/// One agent-facing WASM tool = one exported component function.
#[derive(Clone)]
pub struct WasmToolSpec {
    /// Name the agent calls (appears in the prompt's tool list).
    pub tool_name: String,
    /// Loaded component id (e.g. `tool_codegraph`).
    pub component_id: String,
    /// Exported WIT function name (e.g. `extract-graph`).
    pub function: String,
    pub description: String,
    /// JSON Schema advertised to the agent for the call args.
    pub parameters_schema: Value,
}

/// The installed host + the tools it exposes.
pub struct WasmToolSet {
    pub host: Arc<dyn WasmToolHost>,
    pub specs: Vec<WasmToolSpec>,
}

static WASM_TOOLS: OnceLock<WasmToolSet> = OnceLock::new();

/// Install the process-global WASM tool set (call once at daemon startup).
/// Returns false if already initialized.
pub fn init_wasm_tools(set: WasmToolSet) -> bool {
    WASM_TOOLS.set(set).is_ok()
}

/// The installed WASM tool set, if any.
pub fn wasm_tools() -> Option<&'static WasmToolSet> {
    WASM_TOOLS.get()
}

/// A `Tool` backed by a WASM component function over [`WasmToolHost`].
pub struct WasmTool {
    host: Arc<dyn WasmToolHost>,
    spec: WasmToolSpec,
}

impl WasmTool {
    pub fn new(host: Arc<dyn WasmToolHost>, spec: WasmToolSpec) -> Self {
        Self { host, spec }
    }

    /// Whether this tool's WIT signature includes a `repo-path` parameter.
    fn wants_repo_path(&self) -> bool {
        self.spec
            .parameters_schema
            .get("properties")
            .and_then(|p| p.get("repo-path"))
            .is_some()
    }
}

#[async_trait::async_trait]
impl Tool for WasmTool {
    fn name(&self) -> &str {
        &self.spec.tool_name
    }
    fn description(&self) -> &str {
        &self.spec.description
    }
    fn parameters_schema(&self) -> Value {
        self.spec.parameters_schema.clone()
    }
    async fn execute(&self, params: Value, ctx: &ToolContext) -> ToolResult {
        let start = std::time::Instant::now();
        let repo = ctx.repo_path.display().to_string();
        let mut args = params;

        // If the component takes a `repo-path` arg, the agent never supplies it
        // (it uses the native param shape) — inject it from the run context and
        // make sure the component is granted read on that repo.
        if self.wants_repo_path() {
            if let Value::Object(ref mut m) = args {
                m.entry("repo-path".to_string())
                    .or_insert_with(|| Value::String(repo.clone()));
            }
            let _ = self
                .host
                .ensure_read(&self.spec.component_id, &format!("fs://{repo}"))
                .await;
        }

        // Wassette binds every WIT param positionally and rejects a missing
        // field even for `option<>` args — fill any schema-declared param the
        // agent omitted with null (an absent `option<>` becomes `none`).
        if let (Value::Object(m), Some(props)) = (
            &mut args,
            self.spec.parameters_schema.get("properties").and_then(|p| p.as_object()),
        ) {
            for key in props.keys() {
                m.entry(key.clone()).or_insert(Value::Null);
            }
        }

        let payload = args.to_string();
        let out = self
            .host
            .call(&self.spec.component_id, &self.spec.function, &payload)
            .await;
        let dur = start.elapsed().as_millis() as u64;
        match out {
            Ok(raw) => match unwrap_envelope(&raw) {
                Ok(content) => ToolResult::ok(content, dur),
                Err(e) => ToolResult::err(e, dur),
            },
            Err(e) => ToolResult::err(format!("wasm tool '{}' failed: {e}", self.spec.tool_name), dur),
        }
    }
}

/// Unwrap Wassette's result envelope. Accepts `{"result":{"ok"|"err":..}}` or a
/// bare `{"ok"|"err":..}`; a string `ok` is returned as-is, otherwise re-encoded.
fn unwrap_envelope(out: &str) -> Result<String, String> {
    let v: Value = match serde_json::from_str(out) {
        Ok(v) => v,
        Err(_) => return Ok(out.to_string()),
    };
    let res = v.get("result").unwrap_or(&v);
    if let Some(ok) = res.get("ok") {
        Ok(ok.as_str().map(str::to_string).unwrap_or_else(|| ok.to_string()))
    } else if let Some(err) = res.get("err") {
        Err(err.as_str().map(str::to_string).unwrap_or_else(|| err.to_string()))
    } else {
        Ok(out.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unwraps_result_ok_and_err() {
        assert_eq!(unwrap_envelope(r#"{"result":{"ok":"hello"}}"#).unwrap(), "hello");
        assert_eq!(unwrap_envelope(r#"{"ok":"bare"}"#).unwrap(), "bare");
        assert!(unwrap_envelope(r#"{"result":{"err":"boom"}}"#).is_err());
        // No envelope → raw passthrough.
        assert_eq!(unwrap_envelope("plain text").unwrap(), "plain text");
    }
}
