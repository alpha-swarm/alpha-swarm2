//! Sandboxed filesystem tool component (Wassette).
//!
//! Ports the native `read_file` / `list_files` / `grep` tools to WASM. Reads via
//! `std::fs` (which maps to `wasi:filesystem`), so it only sees directories
//! Wassette has granted. The daemon injects `repo-path` from run context and
//! lazily grants read on that repo (see `swarm_tools::wasm_tools::WasmTool`).

wit_bindgen::generate!({
    path: "wit",
    world: "fs",
    generate_all,
});

use std::path::Path;

/// Max bytes returned by read-file (matches the native tool).
const MAX_READ_BYTES: usize = 100_000;
/// Max grep matches returned (matches the native tool).
const MAX_MATCHES: usize = 100;
/// Max entries returned by list-files.
const MAX_LIST: usize = 2_000;
/// Directories never descended into.
const IGNORE_DIRS: &[&str] = &["target", "node_modules", "dist", "build", "vendor", ".venv", "__pycache__"];

struct Component;
export!(Component);

impl Guest for Component {
    fn read_file(repo_path: String, path: String) -> Result<String, String> {
        if path.is_empty() {
            return Err("missing 'path'".into());
        }
        let full = Path::new(&repo_path).join(&path);
        match std::fs::read_to_string(&full) {
            Ok(c) if c.len() > MAX_READ_BYTES => Ok(format!("{}... (truncated)", &c[..MAX_READ_BYTES])),
            Ok(c) => Ok(c),
            Err(e) => Err(format!("cannot read {path}: {e}")),
        }
    }

    fn list_files(repo_path: String, pattern: Option<String>) -> Result<String, String> {
        let root = Path::new(&repo_path);
        let pat = pattern.unwrap_or_default();
        let mut out = Vec::new();
        walk(root, root, &mut |rel| {
            if pat.is_empty() || rel.contains(&pat) {
                out.push(rel.to_string());
            }
            out.len() >= MAX_LIST
        });
        Ok(out.join("\n"))
    }

    fn grep(
        repo_path: String,
        pattern: String,
        path: Option<String>,
        glob: Option<String>,
    ) -> Result<String, String> {
        if pattern.is_empty() {
            return Err("missing 'pattern'".into());
        }
        let root = Path::new(&repo_path);
        let prefix = path.unwrap_or_default();
        let ext = glob.unwrap_or_default();
        let mut matches = Vec::new();
        walk(root, root, &mut |rel| {
            if !prefix.is_empty() && !rel.starts_with(&prefix) {
                return false;
            }
            if !ext.is_empty() && !rel.ends_with(&format!(".{ext}")) {
                return false;
            }
            if let Ok(content) = std::fs::read_to_string(root.join(rel)) {
                for (i, line) in content.lines().enumerate() {
                    if line.contains(&pattern) {
                        matches.push(format!("{}:{}:{}", rel, i + 1, line));
                        if matches.len() >= MAX_MATCHES {
                            return true;
                        }
                    }
                }
            }
            matches.len() >= MAX_MATCHES
        });
        if matches.is_empty() {
            Ok("No matches found".into())
        } else {
            Ok(matches.join("\n"))
        }
    }
}

/// Recursively visit files under `dir`, calling `f` with each file's path
/// relative to `base`. `f` returns true to stop the whole walk early.
fn walk(dir: &Path, base: &Path, f: &mut dyn FnMut(&str) -> bool) -> bool {
    let Ok(entries) = std::fs::read_dir(dir) else { return false };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if path.is_dir() {
            if name.starts_with('.') || IGNORE_DIRS.contains(&name.as_ref()) {
                continue;
            }
            if walk(&path, base, f) {
                return true;
            }
        } else if let Ok(rel) = path.strip_prefix(base) {
            if f(&rel.to_string_lossy()) {
                return true;
            }
        }
    }
    false
}
