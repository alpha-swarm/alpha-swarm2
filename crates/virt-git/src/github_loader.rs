//! Load repository files from GitHub API into a VirtWorkspace.
//! No git clone, no disk — files fetched via HTTP, stored in blobstore.
//!
//! Uses GitHub Trees API to list files, Contents API to fetch them.

use crate::store::BlobStore;
use crate::workspace::VirtWorkspace;
use serde::Deserialize;

/// Fetch a repository's file tree from GitHub and load into a VirtWorkspace.
/// All via HTTP — works with wasi:http/outgoing-handler.
pub fn load_repo_into_workspace(
    owner: &str,
    repo: &str,
    branch: &str,
    store: &mut dyn BlobStore,
    http: &dyn Fn(&str, &str) -> Result<String, String>, // (url, token) -> response body
    token: &str,
    file_extensions: &[&str],
) -> Result<VirtWorkspace, String> {
    let api = format!("https://api.github.com/repos/{owner}/{repo}");

    // 1. Get the tree SHA for the branch
    let ref_resp = http(
        &format!("{api}/git/ref/heads/{branch}"),
        token,
    )?;
    let ref_data: GitRef = serde_json::from_str(&ref_resp)
        .map_err(|e| format!("parse ref: {e}"))?;

    // 2. Get the full tree (recursive)
    let tree_resp = http(
        &format!("{api}/git/trees/{}?recursive=1", ref_data.object.sha),
        token,
    )?;
    let tree: GitTree = serde_json::from_str(&tree_resp)
        .map_err(|e| format!("parse tree: {e}"))?;

    // 3. Filter files by extension and load each
    let mut ws = VirtWorkspace::new();
    let mut loaded = 0;

    for entry in &tree.tree {
        if entry.entry_type != "blob" {
            continue;
        }

        // Filter by extension
        let matches_ext = file_extensions.is_empty() || file_extensions.iter().any(|ext| {
            entry.path.ends_with(&format!(".{ext}"))
        });
        if !matches_ext {
            continue;
        }

        // Skip common non-source dirs
        if entry.path.starts_with("target/")
            || entry.path.starts_with("node_modules/")
            || entry.path.starts_with(".git/")
            || entry.path.contains("/target/")
        {
            continue;
        }

        // Fetch file content via Contents API
        match http(&format!("{api}/contents/{}?ref={branch}", entry.path), token) {
            Ok(content_resp) => {
                let content_data: GitContent = match serde_json::from_str(&content_resp) {
                    Ok(c) => c,
                    Err(_) => continue, // Skip binary or unparseable
                };

                if content_data.encoding == "base64" {
                    if let Ok(decoded) = base64_decode(&content_data.content.replace('\n', "")) {
                        if let Ok(text) = String::from_utf8(decoded) {
                            ws.load_file(store, &entry.path, &text);
                            loaded += 1;
                        }
                    }
                }
            }
            Err(_) => continue, // Skip files that fail to fetch
        }
    }

    if loaded == 0 {
        return Err("No files loaded from repository".into());
    }

    Ok(ws)
}

/// Load a single file from GitHub into a workspace.
pub fn load_file_from_github(
    owner: &str,
    repo: &str,
    branch: &str,
    path: &str,
    store: &mut dyn BlobStore,
    ws: &mut VirtWorkspace,
    http: &dyn Fn(&str, &str) -> Result<String, String>,
    token: &str,
) -> Result<(), String> {
    let api = format!("https://api.github.com/repos/{owner}/{repo}");
    let resp = http(&format!("{api}/contents/{path}?ref={branch}"), token)?;
    let content: GitContent = serde_json::from_str(&resp)
        .map_err(|e| format!("parse content: {e}"))?;

    if content.encoding == "base64" {
        let decoded = base64_decode(&content.content.replace('\n', ""))
            .map_err(|e| format!("decode: {e}"))?;
        let text = String::from_utf8(decoded)
            .map_err(|e| format!("utf8: {e}"))?;
        ws.load_file(store, path, &text);
        Ok(())
    } else {
        Err(format!("Unsupported encoding: {}", content.encoding))
    }
}

// --- GitHub API types ---

#[derive(Deserialize)]
struct GitRef {
    object: GitRefObject,
}

#[derive(Deserialize)]
struct GitRefObject {
    sha: String,
}

#[derive(Deserialize)]
struct GitTree {
    tree: Vec<GitTreeEntry>,
}

#[derive(Deserialize)]
struct GitTreeEntry {
    path: String,
    #[serde(rename = "type")]
    entry_type: String,
}

#[derive(Deserialize)]
struct GitContent {
    content: String,
    encoding: String,
}

fn base64_decode(input: &str) -> Result<Vec<u8>, String> {
    const CHARS: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut result = Vec::new();
    let bytes: Vec<u8> = input.bytes().filter(|b| *b != b'=' && *b != b'\n' && *b != b'\r').collect();

    for chunk in bytes.chunks(4) {
        let vals: Vec<u8> = chunk.iter().map(|&b| {
            CHARS.iter().position(|&c| c == b).unwrap_or(0) as u8
        }).collect();

        if vals.len() >= 2 {
            result.push((vals[0] << 2) | (vals[1] >> 4));
        }
        if vals.len() >= 3 {
            result.push((vals[1] << 4) | (vals[2] >> 2));
        }
        if vals.len() >= 4 {
            result.push((vals[2] << 6) | vals[3]);
        }
    }

    Ok(result)
}
