//! In-process embed proof: load the codegraph WASM component into an embedded
//! Wassette runtime, grant fs-read, invoke `extract-graph`, assert entities.
//!
//! Requires the component to be built first:
//!   cargo build --release --target wasm32-wasip2 -p tool-codegraph
//!   (with the wasi-sdk env — see crates/tool-host or the wasi-sdk memory)

use tool_host::WassetteHost;

#[tokio::test]
async fn embed_roundtrip_codegraph() {
    let manifest = env!("CARGO_MANIFEST_DIR"); // .../crates/tool-host
    let repo = std::path::Path::new(manifest)
        .parent()
        .unwrap()
        .parent()
        .unwrap(); // workspace root
    let wasm = repo.join("target/wasm32-wasip2/release/tool_codegraph.wasm");
    assert!(wasm.exists(), "build tool-codegraph first; missing {wasm:?}");

    let dir = std::env::temp_dir().join(format!("toolhost-test-{}", std::process::id()));
    let host = WassetteHost::new(&dir).await.expect("new host");

    let (cid, tools) = host
        .load(&format!("file://{}", wasm.display()))
        .await
        .expect("load component");
    assert!(tools.iter().any(|t| t == "extract-graph"), "tools: {tools:?}");

    // Deny-by-default: grant fs read on the repo so the tool can read sources.
    host.grant_storage(&cid, &format!("fs://{}", repo.display()), false)
        .await
        .expect("grant storage");

    let params = serde_json::json!({
        "repo-path": repo.to_str().unwrap(),
        "files": ["crates/tools/src/codegraph.rs"],
    })
    .to_string();
    let out = host
        .call(&cid, "extract-graph", &params)
        .await
        .expect("call extract-graph");

    // Result envelope may be {"result":{"ok":"<json>"}} or {"ok":"<json>"}.
    let v: serde_json::Value = serde_json::from_str(&out).expect("result is json");
    let ok = v
        .pointer("/result/ok")
        .or_else(|| v.pointer("/ok"))
        .and_then(|x| x.as_str())
        .unwrap_or_else(|| panic!("no ok payload in: {out}"));
    let graph: serde_json::Value = serde_json::from_str(ok).expect("inner graph json");
    let n = graph["entities"].as_array().map(|a| a.len()).unwrap_or(0);

    let _ = std::fs::remove_dir_all(&dir);
    assert!(n > 0, "expected entities from codegraph, got {n}; out={out}");
    eprintln!("embed round-trip OK: {n} entities extracted in-process");
}
