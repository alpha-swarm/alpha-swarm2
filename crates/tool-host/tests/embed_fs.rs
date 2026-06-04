//! Exercises the full agent tool-call path for a WASM fs tool: WasmTool injects
//! `repo-path` from ToolContext, lazily grants read on the repo, invokes the
//! sandboxed `grep`, and unwraps the result — no static config grant.

use std::sync::Arc;
use std::time::Duration;

use swarm_tools::wasm_tools::{WasmTool, WasmToolHost, WasmToolSpec};
use swarm_tools::{Tool, ToolContext};
use tool_host::WassetteHost;

#[tokio::test]
async fn wasm_grep_via_tool_path() {
    let repo = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .to_path_buf();
    let wasm = repo.join("target/wasm32-wasip2/release/tool_fs.wasm");
    assert!(wasm.exists(), "build tool-fs first; missing {wasm:?}");

    let dir = std::env::temp_dir().join(format!("toolhost-fs-{}", std::process::id()));
    let host = Arc::new(WassetteHost::new(&dir).await.expect("new host"));
    let (cid, fns) = host
        .load(&format!("file://{}", wasm.display()))
        .await
        .expect("load tool-fs");
    assert!(fns.iter().any(|f| f == "grep"), "fns: {fns:?}");

    // No startup grant — rely on WasmTool's lazy ensure_read from ctx.repo_path.
    let host_dyn: Arc<dyn WasmToolHost> = host;
    let spec = WasmToolSpec {
        tool_name: "grep".into(),
        component_id: cid,
        function: "grep".into(),
        description: "grep".into(),
        parameters_schema: serde_json::json!({
            "type": "object",
            "properties": {
                "repo-path": {"type": "string"},
                "pattern": {"type": "string"},
                "path": {"type": "string"},
                "glob": {"type": "string"}
            },
            "required": ["pattern"]
        }),
    };
    let tool = WasmTool::new(host_dyn, spec);

    let ctx = ToolContext {
        repo_path: repo.clone(),
        project: "test".into(),
        timeout: Duration::from_secs(30),
        file_provider: None,
    };

    // Agent supplies only {pattern, glob}; repo-path is injected from ctx.
    let res = tool
        .execute(serde_json::json!({ "pattern": "WasmToolHost", "glob": "rs" }), &ctx)
        .await;

    assert!(!res.is_error, "grep errored: {}", res.content);
    assert!(
        res.content.contains("wasm_tools.rs") && res.content.contains("WasmToolHost"),
        "expected a hit in wasm_tools.rs, got:\n{}",
        res.content
    );
    let _ = std::fs::remove_dir_all(&dir);
    eprintln!("wasm fs grep OK:\n{}", res.content.lines().take(3).collect::<Vec<_>>().join("\n"));
}
