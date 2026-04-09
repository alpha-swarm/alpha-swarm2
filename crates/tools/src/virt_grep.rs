//! WASI-portable grep using FileProvider (no Command::new, no rg/grep binary).

use serde_json::{json, Value};
use crate::{Tool, ToolContext, ToolResult};

const MAX_MATCHES: usize = 100;

pub struct VirtGrepTool;

#[async_trait::async_trait]
impl Tool for VirtGrepTool {
    fn name(&self) -> &str { "grep" }
    fn description(&self) -> &str { "Search for a pattern in files (pure Rust, no external binary)" }
    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "pattern": {"type": "string", "description": "Text pattern to search for"},
                "path": {"type": "string", "description": "File or directory to search"},
                "glob": {"type": "string", "description": "File extension filter (e.g., 'rs')"}
            },
            "required": ["pattern"]
        })
    }

    async fn execute(&self, params: Value, ctx: &ToolContext) -> ToolResult {
        let pattern = params.get("pattern").and_then(|p| p.as_str()).unwrap_or("");
        if pattern.is_empty() {
            return ToolResult::err("Missing 'pattern' parameter", 0);
        }
        let filter_ext = params.get("glob").and_then(|g| g.as_str()).unwrap_or("");
        let search_path = params.get("path").and_then(|p| p.as_str()).unwrap_or("");

        // Use FileProvider if available
        if let Some(ref fp) = ctx.file_provider {
            if let Ok(fp) = fp.lock() {
                let files = fp.list_files();
                let mut matches = Vec::new();

                for file in &files {
                    // Filter by path prefix
                    if !search_path.is_empty() && !file.starts_with(search_path) {
                        continue;
                    }
                    // Filter by extension
                    if !filter_ext.is_empty() && !file.ends_with(&format!(".{filter_ext}")) {
                        continue;
                    }

                    if let Ok(content) = fp.read_file(file) {
                        for (i, line) in content.lines().enumerate() {
                            if line.contains(pattern) {
                                matches.push(format!("{}:{}:{}", file, i + 1, line));
                                if matches.len() >= MAX_MATCHES { break; }
                            }
                        }
                    }
                    if matches.len() >= MAX_MATCHES { break; }
                }

                return if matches.is_empty() {
                    ToolResult::ok("No matches found", 0)
                } else {
                    ToolResult::ok(matches.join("\n"), 0)
                };
            }
        }

        // Disk fallback — simple file search
        let search_dir = ctx.repo_path.join(if search_path.is_empty() { "." } else { search_path });
        let mut matches = Vec::new();
        search_dir_recursive(&search_dir, &ctx.repo_path, pattern, filter_ext, &mut matches, MAX_MATCHES);

        if matches.is_empty() {
            ToolResult::ok("No matches found", 0)
        } else {
            ToolResult::ok(matches.join("\n"), 0)
        }
    }
}

fn search_dir_recursive(
    dir: &std::path::Path,
    base: &std::path::Path,
    pattern: &str,
    ext_filter: &str,
    out: &mut Vec<String>,
    max: usize,
) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        if out.len() >= max { return; }
        let path = entry.path();
        if path.is_dir() {
            let name = path.file_name().unwrap_or_default().to_string_lossy();
            if name.starts_with('.') || name == "target" || name == "node_modules" { continue; }
            search_dir_recursive(&path, base, pattern, ext_filter, out, max);
        } else {
            if !ext_filter.is_empty() {
                if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
                    if ext != ext_filter { continue; }
                } else { continue; }
            }
            if let Ok(content) = std::fs::read_to_string(&path) {
                let rel = path.strip_prefix(base).unwrap_or(&path);
                for (i, line) in content.lines().enumerate() {
                    if line.contains(pattern) {
                        out.push(format!("{}:{}:{}", rel.display(), i + 1, line));
                        if out.len() >= max { return; }
                    }
                }
            }
        }
    }
}
