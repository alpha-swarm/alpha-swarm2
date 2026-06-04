//! Verifies the wasi:http sandbox path: load the fetch component, grant network
//! to one host, GET a URL through the WASM tool. Requires outbound internet.

use std::sync::Arc;

use tool_host::WassetteHost;

#[tokio::test]
async fn wasm_fetch_url() {
    let repo = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .to_path_buf();
    let wasm = repo.join("target/wasm32-wasip2/release/tool_fetch.wasm");
    assert!(wasm.exists(), "build tool-fetch first; missing {wasm:?}");

    let dir = std::env::temp_dir().join(format!("toolhost-fetch-{}", std::process::id()));
    let host = Arc::new(WassetteHost::new(&dir).await.expect("new host"));
    let (cid, fns) = host
        .load(&format!("file://{}", wasm.display()))
        .await
        .expect("load tool-fetch");
    assert!(fns.iter().any(|f| f == "fetch-url"), "fns: {fns:?}");

    // Deny-by-default: without this grant the fetch is blocked.
    host.grant_network(&cid, "example.com").await.expect("grant network");

    let args = serde_json::json!({ "url": "http://example.com/" }).to_string();
    let out = host.call(&cid, "fetch-url", &args).await;
    let _ = std::fs::remove_dir_all(&dir);

    let raw = out.unwrap_or_else(|e| panic!("fetch call failed (network?): {e}"));
    let v: serde_json::Value = serde_json::from_str(&raw).expect("json");
    let ok = v.pointer("/result/ok").or_else(|| v.pointer("/ok")).and_then(|x| x.as_str());
    match ok {
        Some(body) => {
            assert!(
                body.contains("Example Domain") || body.to_lowercase().contains("example"),
                "unexpected body: {}",
                &body[..body.len().min(200)]
            );
            eprintln!("wasm fetch OK: {} bytes", body.len());
        }
        None => panic!("fetch returned err envelope: {raw}"),
    }
}
