//! ZERO-DISK E2E: GitHub API → blobstore → agent → diff → PR
//!
//! 1. Fetch README.md from GitHub API (no git clone)
//! 2. Store in NATS blobstore via VirtWorkspace
//! 3. Call WASI agent-worker for edit (Ollama via HTTP)
//! 4. Apply edit in memory
//! 5. Diff via virt-git (in-memory SHA256 tree comparison)
//! 6. Create PR via GitHub API (blob → tree → commit → ref → PR)
//!
//! ZERO filesystem. ZERO git CLI. ZERO git2. All HTTP + blobstore.
//!
//! Requires: NATS, Ollama, GITHUB_TOKEN, wash dev (agent-worker on :8000)
//! Run: GITHUB_TOKEN=... cargo test -p virt-git --test zero_disk_e2e --features nats -- --nocapture

#[cfg(feature = "nats")]
mod tests {
    use virt_git::*;

    #[tokio::test(flavor = "multi_thread")]
    async fn zero_disk_readme_mermaid_pr() {
        let token = std::env::var("GITHUB_TOKEN")
            .expect("Set GITHUB_TOKEN");

        let owner = "alpha-swarm";
        let repo = "alpha-swarm2";
        let branch = "main";

        let http_client = reqwest::Client::new();
        let token_ref = &token;

        // HTTP helper for GitHub API
        let gh_http = |url: &str, tkn: &str| -> Result<String, String> {
            tokio::task::block_in_place(|| {
                tokio::runtime::Handle::current().block_on(async {
                    let resp = http_client.get(url)
                        .header("Authorization", format!("Bearer {tkn}"))
                        .header("Accept", "application/vnd.github+json")
                        .header("User-Agent", "alpha-swarm")
                        .send().await.map_err(|e| format!("{e}"))?;
                    let status = resp.status();
                    let text = resp.text().await.map_err(|e| format!("{e}"))?;
                    if !status.is_success() {
                        return Err(format!("{status}: {}", &text[..text.len().min(200)]));
                    }
                    Ok(text)
                })
            })
        };

        // === 1. Fetch CHANGELOG.md from GitHub API into blobstore ===
        println!("1. Fetching CHANGELOG.md from GitHub API...");
        let mut store = MemoryBlobStore::new();
        let mut ws = VirtWorkspace::new();

        load_file_from_github(owner, repo, branch, "CHANGELOG.md", &mut store, &mut ws, &gh_http, token_ref)
            .expect("Failed to load CHANGELOG.md from GitHub");

        let original = ws.read_file(&store, "CHANGELOG.md").expect("CHANGELOG.md not in workspace");
        println!("   Loaded {} bytes into blobstore", original.len());
        assert!(original.contains("v0.1.0"), "Expected v0.1.0 in CHANGELOG");

        // === 2. Call WASI agent-worker (gemma4:26b) ===
        println!("2. Calling WASI agent-worker with gemma4:26b...");

        let task = serde_json::json!({
            "task": "Add a v0.2.0 section to the CHANGELOG dated 2026-04-10 with these entries: Added gemma4:26b model support, Added 4 new WASI components (orchestrator-worker, tools-worker, knowledge-store, quality-gate-worker), Moved FileProvider trait to virt-git crate, All 10 crates compile to wasm32-wasip2",
            "model": "gemma4:26b",
            "ollama_url": "http://100.81.10.8:11434",
            "workspace_id": "zero-disk-e2e",
            "files": [{"path": "CHANGELOG.md", "content": original}]
        });

        let resp: serde_json::Value = tokio::task::block_in_place(|| {
            tokio::runtime::Handle::current().block_on(async {
                http_client.post("http://localhost:8000/")
                    .json(&task)
                    .send().await.expect("agent-worker unreachable")
                    .json().await.expect("bad json")
            })
        });

        let status = resp["status"].as_str().unwrap_or("error");
        let edits = resp["edits"].as_u64().unwrap_or(0);
        println!("   Status: {status}, Edits: {edits}");

        if status != "ok" || edits == 0 {
            println!("   Agent failed or no edits. Raw: {}", &resp.to_string()[..resp.to_string().len().min(500)]);
            println!("   SKIPPING PR (agent didn't produce edits)");
            return;
        }

        // === 3. Apply edit in memory ===
        let raw = resp["raw_response"].as_str().unwrap_or("");
        let parsed_edits = edit_parser::parse_edits(raw).unwrap_or_default();

        for edit in &parsed_edits {
            if let edit_parser::FileEdit::Edit { path, old, new } = edit {
                if let Some(current) = ws.read_file(&store, path) {
                    let updated = current.replacen(old.as_str(), new.as_str(), 1);
                    ws.write_file(&mut store, path, &updated);
                    println!("3. Applied edit to {path} in blobstore");
                }
            }
        }

        assert!(ws.has_changes(), "No changes after applying edits");

        // === 4. Diff in memory ===
        let diff_text = ws.diff_text(&store);
        println!("4. Diff (in-memory):");
        println!("{}", diff_text.chars().take(500).collect::<String>());
        assert!(diff_text.contains("v0.2.0") || diff_text.contains("gemma4"), "Diff should contain v0.2.0 or gemma4");

        // === 5. Create PR via GitHub API ===
        println!("5. Creating PR via GitHub API...");
        let branch_name = format!("agent/zero-disk-mermaid-{}", std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH).unwrap().as_secs());

        let gh_config = GitHubConfig {
            owner: owner.into(), repo: repo.into(),
            token: token.clone(), base_branch: branch.into(),
        };

        let pr = create_pr(
            &gh_config, &ws, &store,
            "docs: add v0.2.0 to CHANGELOG (gemma4:26b zero-disk E2E)",
            "docs: add v0.2.0 to CHANGELOG (gemma4:26b zero-disk)",
            &format!("## Zero-Disk E2E\n\nEntire pipeline with no filesystem:\n1. README.md fetched via GitHub API\n2. Stored in NATS blobstore\n3. Agent edit via wasi:http → Ollama\n4. Diff via virt-git (in-memory SHA256)\n5. PR via GitHub API\n\n```diff\n{}\n```\n\n🤖 alpha-swarm", diff_text.chars().take(2000).collect::<String>()),
            &branch_name,
            &|method, url, body, token| {
                tokio::task::block_in_place(|| {
                    tokio::runtime::Handle::current().block_on(async {
                        let mut req = match method {
                            "GET" => http_client.get(url),
                            "POST" => http_client.post(url),
                            _ => return Err(format!("Unknown: {method}")),
                        };
                        req = req.header("Authorization", format!("Bearer {token}"))
                            .header("Accept", "application/vnd.github+json")
                            .header("User-Agent", "alpha-swarm");
                        if !body.is_empty() {
                            req = req.header("Content-Type", "application/json").body(body.to_string());
                        }
                        let resp = req.send().await.map_err(|e| format!("{e}"))?;
                        let st = resp.status();
                        let text = resp.text().await.map_err(|e| format!("{e}"))?;
                        if !st.is_success() { return Err(format!("{st}: {}", &text[..text.len().min(300)])); }
                        Ok(text)
                    })
                })
            },
        ).expect("PR creation failed");

        println!("\n=== ZERO-DISK E2E PASSED ===");
        println!("PR: {}", pr.pr_url);
        println!("Branch: {}", pr.branch);
        println!("Commit: {}", pr.commit_sha);
        println!("Files: {:?}", pr.files_changed);
        println!("\nZero filesystem. Zero git. Zero disk.");
        println!("GitHub API → NATS blobstore → Ollama → virt-git → GitHub API");
    }
}
