//! GitHub Git Data API client — creates commits and PRs without touching disk.
//!
//! All operations use REST calls (works with wasi:http/outgoing-handler).
//! No git CLI, no git2, no filesystem needed.
//!
//! Flow:
//!   1. Get base branch SHA
//!   2. For each changed file: create blob
//!   3. Create tree from blobs
//!   4. Create commit pointing to tree
//!   5. Create branch ref pointing to commit
//!   6. Create PR from branch to base

use serde::{Deserialize, Serialize};
use crate::store::BlobStore;
use crate::workspace::VirtWorkspace;

/// GitHub API client configuration.
#[derive(Clone)]
pub struct GitHubConfig {
    pub owner: String,
    pub repo: String,
    pub token: String,
    pub base_branch: String,
}

/// Result of creating a PR via GitHub API.
#[derive(Debug, Serialize)]
pub struct PrResult {
    pub pr_url: String,
    pub pr_number: u64,
    pub branch: String,
    pub commit_sha: String,
    pub files_changed: Vec<String>,
    pub diff_summary: String,
}

// --- GitHub API request/response types ---

#[derive(Serialize)]
struct CreateBlobRequest {
    content: String,
    encoding: String,
}

#[derive(Deserialize)]
struct CreateBlobResponse {
    sha: String,
}

#[derive(Serialize)]
struct TreeEntry {
    path: String,
    mode: String,
    #[serde(rename = "type")]
    entry_type: String,
    sha: String,
}

#[derive(Serialize)]
struct CreateTreeRequest {
    base_tree: String,
    tree: Vec<TreeEntry>,
}

#[derive(Deserialize)]
struct CreateTreeResponse {
    sha: String,
}

#[derive(Serialize)]
struct CreateCommitRequest {
    message: String,
    tree: String,
    parents: Vec<String>,
}

#[derive(Deserialize)]
struct CreateCommitResponse {
    sha: String,
}

#[derive(Serialize)]
struct CreateRefRequest {
    #[serde(rename = "ref")]
    ref_name: String,
    sha: String,
}

#[derive(Serialize)]
struct CreatePrRequest {
    title: String,
    body: String,
    head: String,
    base: String,
}

#[derive(Deserialize)]
struct CreatePrResponse {
    number: u64,
    html_url: String,
}

#[derive(Deserialize)]
struct RefResponse {
    object: RefObject,
}

#[derive(Deserialize)]
struct RefObject {
    sha: String,
}

/// Build all the GitHub API request bodies needed to create a PR from a VirtWorkspace.
/// Returns a sequence of (method, path, body) tuples that the caller sends via HTTP.
///
/// This is pure computation — the caller handles the actual HTTP calls
/// (native reqwest, or wasi:http, or any other HTTP client).
pub fn build_pr_requests(
    config: &GitHubConfig,
    workspace: &VirtWorkspace,
    store: &dyn BlobStore,
    commit_message: &str,
    pr_title: &str,
    pr_body: &str,
    branch_name: &str,
) -> Vec<GitHubApiCall> {
    let diffs = workspace.diff(store);
    if diffs.is_empty() {
        return vec![];
    }

    let mut calls = Vec::new();
    let api_base = format!("https://api.github.com/repos/{}/{}", config.owner, config.repo);

    // Step 1: Get base branch SHA
    calls.push(GitHubApiCall {
        step: "get_base_sha",
        method: "GET",
        path: format!("{}/git/ref/heads/{}", api_base, config.base_branch),
        body: String::new(),
    });

    // Step 2: Create blobs for each changed file
    for diff in &diffs {
        if let Some(content) = workspace.read_file(store, &diff.path) {
            let blob_req = CreateBlobRequest {
                content: base64_encode(content.as_bytes()),
                encoding: "base64".into(),
            };
            calls.push(GitHubApiCall {
                step: "create_blob",
                method: "POST",
                path: format!("{}/git/blobs", api_base),
                body: serde_json::to_string(&blob_req).unwrap_or_default(),
            });
        }
    }

    // Steps 3-6 depend on responses from previous steps.
    // The caller chains them. We provide templates:

    // Step 3: Create tree (needs base_tree_sha + blob SHAs from step 2)
    // Template — caller fills in SHAs
    let tree_entries: Vec<TreeEntry> = diffs.iter()
        .map(|d| TreeEntry {
            path: d.path.clone(),
            mode: "100644".into(),
            entry_type: "blob".into(),
            sha: format!("{{blob_sha_{}}}", d.path), // Placeholder
        })
        .collect();

    calls.push(GitHubApiCall {
        step: "create_tree_template",
        method: "POST",
        path: format!("{}/git/trees", api_base),
        body: serde_json::to_string(&CreateTreeRequest {
            base_tree: "{base_tree_sha}".into(),
            tree: tree_entries,
        }).unwrap_or_default(),
    });

    // Step 4: Create commit
    calls.push(GitHubApiCall {
        step: "create_commit_template",
        method: "POST",
        path: format!("{}/git/commits", api_base),
        body: serde_json::to_string(&CreateCommitRequest {
            message: commit_message.into(),
            tree: "{tree_sha}".into(),
            parents: vec!["{base_sha}".into()],
        }).unwrap_or_default(),
    });

    // Step 5: Create branch ref
    calls.push(GitHubApiCall {
        step: "create_ref",
        method: "POST",
        path: format!("{}/git/refs", api_base),
        body: serde_json::to_string(&CreateRefRequest {
            ref_name: format!("refs/heads/{branch_name}"),
            sha: "{commit_sha}".into(),
        }).unwrap_or_default(),
    });

    // Step 6: Create PR
    calls.push(GitHubApiCall {
        step: "create_pr",
        method: "POST",
        path: format!("{}/pulls", api_base),
        body: serde_json::to_string(&CreatePrRequest {
            title: pr_title.into(),
            body: pr_body.into(),
            head: branch_name.into(),
            base: config.base_branch.clone(),
        }).unwrap_or_default(),
    });

    calls
}

/// A GitHub API call to be executed by the caller's HTTP client.
#[derive(Debug, Clone, Serialize)]
pub struct GitHubApiCall {
    pub step: &'static str,
    pub method: &'static str,
    pub path: String,
    pub body: String,
}

/// Execute the full PR creation flow using a synchronous HTTP callback.
/// The callback sends HTTP requests and returns response bodies.
/// Works with any HTTP client (native reqwest, wasi:http, etc.)
pub fn create_pr(
    config: &GitHubConfig,
    workspace: &VirtWorkspace,
    store: &dyn BlobStore,
    commit_message: &str,
    pr_title: &str,
    pr_body: &str,
    branch_name: &str,
    http: &dyn Fn(&str, &str, &str, &str) -> Result<String, String>, // (method, url, body, token) -> response
) -> Result<PrResult, String> {
    let diffs = workspace.diff(store);
    if diffs.is_empty() {
        return Err("No changes to create PR from".into());
    }

    let api = format!("https://api.github.com/repos/{}/{}", config.owner, config.repo);

    // 1. Get base branch SHA
    let base_resp = http("GET", &format!("{}/git/ref/heads/{}", api, config.base_branch), "", &config.token)?;
    let base_ref: RefResponse = serde_json::from_str(&base_resp)
        .map_err(|e| format!("parse base ref: {e}"))?;
    let base_sha = base_ref.object.sha;

    // 2. Create blobs for changed files
    let mut blob_shas: Vec<(String, String)> = Vec::new(); // (path, sha)
    for diff in &diffs {
        if let Some(content) = workspace.read_file(store, &diff.path) {
            let blob_req = serde_json::to_string(&CreateBlobRequest {
                content: base64_encode(content.as_bytes()),
                encoding: "base64".into(),
            }).map_err(|e| format!("serialize blob: {e}"))?;

            let blob_resp = http("POST", &format!("{}/git/blobs", api), &blob_req, &config.token)?;
            let blob: CreateBlobResponse = serde_json::from_str(&blob_resp)
                .map_err(|e| format!("parse blob response: {e}"))?;
            blob_shas.push((diff.path.clone(), blob.sha));
        }
    }

    // 3. Create tree
    let tree_entries: Vec<TreeEntry> = blob_shas.iter()
        .map(|(path, sha)| TreeEntry {
            path: path.clone(),
            mode: "100644".into(),
            entry_type: "blob".into(),
            sha: sha.clone(),
        })
        .collect();

    let tree_req = serde_json::to_string(&CreateTreeRequest {
        base_tree: base_sha.clone(),
        tree: tree_entries,
    }).map_err(|e| format!("serialize tree: {e}"))?;

    let tree_resp = http("POST", &format!("{}/git/trees", api), &tree_req, &config.token)?;
    let tree: CreateTreeResponse = serde_json::from_str(&tree_resp)
        .map_err(|e| format!("parse tree response: {e}"))?;

    // 4. Create commit
    let commit_req = serde_json::to_string(&CreateCommitRequest {
        message: commit_message.into(),
        tree: tree.sha,
        parents: vec![base_sha],
    }).map_err(|e| format!("serialize commit: {e}"))?;

    let commit_resp = http("POST", &format!("{}/git/commits", api), &commit_req, &config.token)?;
    let commit: CreateCommitResponse = serde_json::from_str(&commit_resp)
        .map_err(|e| format!("parse commit response: {e}"))?;

    // 5. Create branch
    let ref_req = serde_json::to_string(&CreateRefRequest {
        ref_name: format!("refs/heads/{branch_name}"),
        sha: commit.sha.clone(),
    }).map_err(|e| format!("serialize ref: {e}"))?;

    http("POST", &format!("{}/git/refs", api), &ref_req, &config.token)?;

    // 6. Create PR
    let pr_req = serde_json::to_string(&CreatePrRequest {
        title: pr_title.into(),
        body: pr_body.into(),
        head: branch_name.into(),
        base: config.base_branch.clone(),
    }).map_err(|e| format!("serialize PR: {e}"))?;

    let pr_resp = http("POST", &format!("{}/pulls", api), &pr_req, &config.token)?;
    let pr: CreatePrResponse = serde_json::from_str(&pr_resp)
        .map_err(|e| format!("parse PR response: {e}"))?;

    Ok(PrResult {
        pr_url: pr.html_url,
        pr_number: pr.number,
        branch: branch_name.into(),
        commit_sha: commit.sha,
        files_changed: diffs.iter().map(|d| d.path.clone()).collect(),
        diff_summary: crate::diff::format_diff(&diffs),
    })
}

/// Simple base64 encoding (no external dep needed).
/// Encodes bytes to base64 for GitHub blob API.
fn base64_encode(data: &[u8]) -> String {
    const CHARS: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut result = String::new();
    for chunk in data.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = if chunk.len() > 1 { chunk[1] as u32 } else { 0 };
        let b2 = if chunk.len() > 2 { chunk[2] as u32 } else { 0 };
        let n = (b0 << 16) | (b1 << 8) | b2;

        result.push(CHARS[((n >> 18) & 63) as usize] as char);
        result.push(CHARS[((n >> 12) & 63) as usize] as char);
        if chunk.len() > 1 {
            result.push(CHARS[((n >> 6) & 63) as usize] as char);
        } else {
            result.push('=');
        }
        if chunk.len() > 2 {
            result.push(CHARS[(n & 63) as usize] as char);
        } else {
            result.push('=');
        }
    }
    result
}
