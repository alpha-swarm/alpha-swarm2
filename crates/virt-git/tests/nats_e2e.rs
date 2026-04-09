//! E2E test: VirtWorkspace with WasiBlobStoreAdapter backed by NATS Object Store.
//!
//! Simulates what a WASI component would do:
//! 1. Create workspace from repo files
//! 2. Agent makes edits (add doc comment)
//! 3. Extract diff
//! 4. Commit
//! 5. Verify blobs are in NATS
//!
//! Requires: NATS with JetStream running on localhost:4223
//! Run: cargo test -p virt-git --test nats_e2e --features nats

#[cfg(feature = "nats")]
mod tests {
    use std::sync::{Arc, Mutex};
    use virt_git::*;

    #[tokio::test(flavor = "multi_thread")]
    async fn workspace_e2e_with_nats_blobstore() {
        // Connect to NATS
        let nats_url = std::env::var("NATS_URL")
            .unwrap_or_else(|_| "nats://127.0.0.1:4223".into());

        let client = async_nats::connect(&nats_url).await
            .expect("NATS not running — start with: nats-server --jetstream");

        let js = async_nats::jetstream::new(client.clone());

        // Create object store bucket for this test
        let bucket_name = format!("virt-git-test-{}", std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH).unwrap().as_millis());

        let obj_store = js.create_object_store(
            async_nats::jetstream::object_store::Config {
                bucket: bucket_name.clone(),
                ..Default::default()
            }
        ).await.expect("Failed to create object store bucket");

        println!("Created NATS bucket: {bucket_name}");

        // Create WasiBlobStoreAdapter with NATS callbacks
        // This is what a WASI component would do after wit_bindgen wires up wasi:blobstore
        let obj_store_put = obj_store.clone();
        let obj_store_get = obj_store.clone();
        let obj_store_exists = obj_store.clone();
        let obj_store_del = obj_store.clone();
        let rt = tokio::runtime::Handle::current();

        let rt_put = rt.clone();
        let rt_get = rt.clone();
        let rt_exists = rt.clone();
        let rt_del = rt.clone();

        let mut store = WasiBlobStoreAdapter::new(&bucket_name)
            .with_callbacks(
                // put
                move |key: &str, data: &[u8]| {
                    let store = obj_store_put.clone();
                    let key = key.to_string();
                    let data = data.to_vec();
                    tokio::task::block_in_place(|| {
                        rt_put.block_on(async {
                            let mut reader = &data[..];
                            let _ = store.put(key.as_str(), &mut reader).await;
                        });
                    });
                },
                // get
                move |key: &str| -> Option<Vec<u8>> {
                    let store = obj_store_get.clone();
                    let key = key.to_string();
                    tokio::task::block_in_place(|| {
                        rt_get.block_on(async {
                            match store.get(&key).await {
                                Ok(mut obj) => {
                                    use tokio::io::AsyncReadExt;
                                    let mut buf = Vec::new();
                                    obj.read_to_end(&mut buf).await.ok()?;
                                    Some(buf)
                                }
                                Err(_) => None,
                            }
                        })
                    })
                },
                // exists
                move |key: &str| -> bool {
                    let store = obj_store_exists.clone();
                    let key = key.to_string();
                    tokio::task::block_in_place(|| {
                        rt_exists.block_on(async {
                            store.info(&key).await.is_ok()
                        })
                    })
                },
                // delete
                move |key: &str| {
                    let store = obj_store_del.clone();
                    let key = key.to_string();
                    tokio::task::block_in_place(|| {
                        rt_del.block_on(async {
                            let _ = store.delete(&key).await;
                        });
                    });
                },
            );

        // === Simulate agent workflow ===

        // 1. Load repo files into workspace (like reading from git HEAD)
        let mut ws = VirtWorkspace::from_files(&mut store, &[
            ("crates/orchestrator/src/runner.rs",
             "fn discover_source_files(repo: &Path) -> Result<Vec<String>> {\n    let mut files = Vec::new();\n    files\n}\n"),
            ("Cargo.toml", "[workspace]\nmembers = [\"crates/*\"]\n"),
        ]);

        assert_eq!(ws.list_files().len(), 2);
        assert!(!ws.has_changes());

        // 2. Agent makes an edit (add doc comment)
        ws.write_file(&mut store,
            "crates/orchestrator/src/runner.rs",
            "/// Discovers source files in the repository.\n/// Scans for .rs, .ts, .js, .go, .py extensions.\n/// Skips .git, target, and node_modules directories.\nfn discover_source_files(repo: &Path) -> Result<Vec<String>> {\n    let mut files = Vec::new();\n    files\n}\n",
        );

        assert!(ws.has_changes());

        // 3. Extract diff
        let diffs = ws.diff(&store);
        assert_eq!(diffs.len(), 1);
        assert_eq!(diffs[0].path, "crates/orchestrator/src/runner.rs");
        assert_eq!(diffs[0].kind, DiffKind::Modified);

        let diff_text = ws.diff_text(&store);
        println!("=== DIFF ===\n{diff_text}");
        assert!(diff_text.contains("+/// Discovers source files"));
        assert!(diff_text.contains("+/// Scans for .rs, .ts, .js"));
        assert!(diff_text.contains("+/// Skips .git, target"));

        // 4. Commit
        let commit = ws.commit("Add doc comment to discover_source_files");
        println!("Commit: {} — {}", commit.id, commit.message);
        assert!(!ws.has_changes()); // base now matches working
        assert_eq!(ws.commits().len(), 1);

        // 5. Verify blobs are in NATS
        // The file content should be stored as blob/{sha256} in the object store
        let info = obj_store.list().await;
        let mut blob_count = 0;
        if let Ok(mut list) = info {
            use futures::StreamExt;
            while let Some(Ok(obj)) = list.next().await {
                println!("NATS blob: {} ({} bytes)", obj.name, obj.size);
                blob_count += 1;
            }
        }
        println!("Total blobs in NATS: {blob_count}");
        assert!(blob_count >= 2, "Expected at least 2 blobs (original + edited file)");

        // 6. Read back from NATS (proves data round-trips)
        let content = ws.read_file(&store, "crates/orchestrator/src/runner.rs");
        assert!(content.is_some());
        assert!(content.unwrap().contains("/// Discovers source files"));

        // Cleanup: delete the test bucket
        let _ = js.delete_object_store(&bucket_name).await;
        println!("Cleaned up bucket: {bucket_name}");
        println!("=== E2E PASSED ===");
    }
}
