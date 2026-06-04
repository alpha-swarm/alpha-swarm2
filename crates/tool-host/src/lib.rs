//! Embedded Wassette tool host.
//!
//! Owns the wassette `LifecycleManager` (which embeds wasmtime + the WASI
//! capability sandbox) and exposes a minimal API for the daemon to load WASM
//! tool components and invoke them IN-PROCESS — mirroring how `knowledge-base`
//! owns the embedded SurrealDB handle (sole owner; everyone else goes through
//! it). Per-component capability policies are persisted to the component
//! directory on disk and restored on load.

use anyhow::Result;
use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::{Arc, Mutex};
use wassette::LifecycleManager;

use swarm_tools::wasm_tools::{init_wasm_tools, WasmToolSet, WasmToolSpec};

/// Handle to the embedded Wassette runtime. Cheap to clone (`Arc` inside).
#[derive(Clone)]
pub struct WassetteHost {
    mgr: Arc<LifecycleManager>,
    /// Dedupe of already-granted `component|uri` pairs (for lazy `ensure_read`).
    granted: Arc<Mutex<HashSet<String>>>,
}

impl WassetteHost {
    /// Open (or create) the component store at `component_dir` and eagerly load
    /// any components already cached there, restoring their policies.
    pub async fn new(component_dir: impl AsRef<Path>) -> Result<Self> {
        let mgr = LifecycleManager::new(component_dir).await?;
        Ok(Self { mgr: Arc::new(mgr), granted: Arc::new(Mutex::new(HashSet::new())) })
    }

    /// Load a component from a `file://` path or `oci://` ref. Returns its
    /// component id and the tool names it exposes (one per exported WIT fn).
    pub async fn load(&self, uri: &str) -> Result<(String, Vec<String>)> {
        let outcome = self.mgr.load_component(uri).await?;
        Ok((outcome.component_id, outcome.tool_names))
    }

    /// Grant a component filesystem access at `uri` (e.g. `fs:///path`).
    pub async fn grant_storage(&self, component_id: &str, uri: &str, write: bool) -> Result<()> {
        let access = if write {
            serde_json::json!(["read", "write"])
        } else {
            serde_json::json!(["read"])
        };
        self.mgr
            .grant_permission(component_id, "storage", &serde_json::json!({ "uri": uri, "access": access }))
            .await?;
        // Record read grants so lazy `ensure_read` doesn't re-grant them.
        if !write {
            if let Ok(mut set) = self.granted.lock() {
                set.insert(format!("{component_id}|{uri}"));
            }
        }
        Ok(())
    }

    /// Grant a component outbound network access to `host`.
    pub async fn grant_network(&self, component_id: &str, host: &str) -> Result<()> {
        self.mgr
            .grant_permission(component_id, "network", &serde_json::json!({ "host": host }))
            .await
    }

    /// Invoke an exported function of a loaded component. `params_json` is a
    /// JSON object keyed by WIT arg name; returns the JSON-encoded result
    /// (`{"result":{"ok"|"err":...}}`). Capability policy is enforced.
    pub async fn call(&self, component_id: &str, function: &str, params_json: &str) -> Result<String> {
        self.mgr.execute_component_call(component_id, function, params_json).await
    }

    /// All currently-loaded tools as MCP-shaped JSON (name + input schema).
    pub async fn list_tools(&self) -> Vec<serde_json::Value> {
        self.mgr.list_tools().await
    }

    /// Escape hatch for ops not yet wrapped (grant/revoke permissions, schema).
    pub fn manager(&self) -> &Arc<LifecycleManager> {
        &self.mgr
    }
}

/// Build a `file://` URI from a possibly-relative path (resolved against cwd).
fn to_uri(spec: &str) -> String {
    if spec.contains("://") {
        return spec.to_string();
    }
    let p = std::path::Path::new(spec);
    let abs = if p.is_absolute() {
        p.to_path_buf()
    } else {
        std::env::current_dir().map(|d| d.join(p)).unwrap_or_else(|_| p.to_path_buf())
    };
    format!("file://{}", abs.display())
}

fn fs_uri(dir: &str) -> String {
    if dir.starts_with("fs://") { dir.to_string() } else { format!("fs://{dir}") }
}

/// Load the WASM tool components declared in config, grant their capabilities,
/// and install the process-global tool set so the agent dispatch path
/// (`ToolRegistry::with_wasm_tools`) can surface them. No-op (returns 0) when
/// disabled. Call ONCE at daemon startup. Returns the number of tools exposed.
pub async fn install_from_config(cfg: &swarm_config::WassetteConfig) -> Result<usize> {
    if !cfg.enabled {
        tracing::info!("Wassette tool host disabled (set [wassette] enabled = true to opt in)");
        return Ok(0);
    }
    let host = Arc::new(WassetteHost::new(&cfg.component_dir).await?);

    // Load each component + grant its capabilities; remember (component_id, fns).
    let mut loaded: Vec<(String, Vec<String>)> = Vec::new();
    for t in &cfg.tools {
        let uri = to_uri(&t.wasm);
        let (cid, fns) = host.load(&uri).await?;
        for d in &t.fs_read {
            host.grant_storage(&cid, &fs_uri(d), false).await?;
        }
        for d in &t.fs_write {
            host.grant_storage(&cid, &fs_uri(d), true).await?;
        }
        for h in &t.net {
            host.grant_network(&cid, h).await?;
        }
        tracing::info!(component = %cid, tools = ?fns, "Loaded WASM tool component");
        loaded.push((cid, fns));
    }

    // Pull descriptions + input schemas straight from the loaded components.
    let mut meta: HashMap<String, (String, serde_json::Value)> = HashMap::new();
    for def in host.list_tools().await {
        if let Some(name) = def.get("name").and_then(|v| v.as_str()) {
            let desc = def.get("description").and_then(|v| v.as_str()).unwrap_or("").to_string();
            let schema = def.get("inputSchema").cloned().unwrap_or_else(|| serde_json::json!({ "type": "object" }));
            meta.insert(name.to_string(), (desc, schema));
        }
    }

    let mut specs = Vec::new();
    for (cid, fns) in loaded {
        for f in fns {
            let (description, parameters_schema) = meta.get(&f).cloned().unwrap_or_else(|| {
                (format!("WASM tool: {f}"), serde_json::json!({ "type": "object" }))
            });
            specs.push(WasmToolSpec {
                tool_name: f.clone(),
                component_id: cid.clone(),
                function: f,
                description,
                parameters_schema,
            });
        }
    }

    let count = specs.len();
    let host_dyn: Arc<dyn swarm_tools::wasm_tools::WasmToolHost> = host;
    if !init_wasm_tools(WasmToolSet { host: host_dyn, specs }) {
        tracing::warn!("WASM tool set already installed — ignoring second init");
    }
    tracing::info!(tools = count, "Wassette tool host installed");
    Ok(count)
}

/// Bridge to the agent tool dispatch path: lets `swarm-tools` invoke WASM
/// components via this embedded host without depending on wasmtime.
#[async_trait::async_trait]
impl swarm_tools::wasm_tools::WasmToolHost for WassetteHost {
    async fn call(&self, component_id: &str, function: &str, params_json: &str) -> Result<String, String> {
        self.mgr
            .execute_component_call(component_id, function, params_json)
            .await
            .map_err(|e| e.to_string())
    }

    async fn ensure_read(&self, component_id: &str, dir: &str) -> Result<(), String> {
        let key = format!("{component_id}|{dir}");
        {
            let set = self.granted.lock().map_err(|_| "granted lock poisoned")?;
            if set.contains(&key) {
                return Ok(());
            }
        }
        self.grant_storage(component_id, dir, false).await.map_err(|e| e.to_string())?;
        if let Ok(mut set) = self.granted.lock() {
            set.insert(key);
        }
        Ok(())
    }
}
