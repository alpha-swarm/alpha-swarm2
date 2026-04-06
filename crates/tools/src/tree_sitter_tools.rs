use async_trait::async_trait;
use serde_json::{json, Value};

use crate::{Tool, ToolContext, ToolResult};

/// Max symbols to return from extract_signatures
const MAX_SIGNATURES: usize = 200;

pub struct TreeSitterRenameTool;

#[async_trait]
impl Tool for TreeSitterRenameTool {
    fn name(&self) -> &str { "ts_rename" }
    fn description(&self) -> &str { "Rename a symbol in a file using tree-sitter AST (instant, precise)" }
    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": {"type": "string", "description": "File path"},
                "old_name": {"type": "string", "description": "Current symbol name"},
                "new_name": {"type": "string", "description": "New symbol name"}
            },
            "required": ["path", "old_name", "new_name"]
        })
    }

    async fn execute(&self, params: Value, ctx: &ToolContext) -> ToolResult {
        let path = params.get("path").and_then(|p| p.as_str()).unwrap_or("");
        let old_name = params.get("old_name").and_then(|p| p.as_str()).unwrap_or("");
        let new_name = params.get("new_name").and_then(|p| p.as_str()).unwrap_or("");

        if path.is_empty() || old_name.is_empty() || new_name.is_empty() {
            return ToolResult::err("Missing required parameters: path, old_name, new_name", 0);
        }

        let full = ctx.repo_path.join(path);
        let content = match std::fs::read_to_string(&full) {
            Ok(c) => c,
            Err(e) => return ToolResult::err(format!("Cannot read {path}: {e}"), 0),
        };

        let mut parser = tree_sitter::Parser::new();
        if parser.set_language(&tree_sitter_rust::LANGUAGE.into()).is_err() {
            // Fallback to text replace for non-Rust files
            let new_content = content.replace(old_name, new_name);
            let count = content.matches(old_name).count();
            if count == 0 {
                return ToolResult::err(format!("Symbol '{old_name}' not found in {path}"), 0);
            }
            if let Err(e) = std::fs::write(&full, &new_content) {
                return ToolResult::err(format!("Cannot write {path}: {e}"), 0);
            }
            return ToolResult::ok(format!("Renamed {count} occurrences of '{old_name}' to '{new_name}' in {path} (text replace, non-Rust file)"), 0);
        }

        let Some(tree) = parser.parse(&content, None) else {
            return ToolResult::err(format!("Failed to parse {path}"), 0);
        };

        // Find all identifier nodes matching old_name
        let mut cursor = tree.walk();
        let mut replacements = Vec::new();
        collect_identifiers(&content, cursor.node(), old_name, &mut replacements);

        if replacements.is_empty() {
            return ToolResult::err(format!("Symbol '{old_name}' not found in AST of {path}"), 0);
        }

        // Apply replacements in reverse order to preserve positions
        let mut new_content = content.clone();
        replacements.sort_by(|a, b| b.0.cmp(&a.0));
        for (start, end) in &replacements {
            new_content.replace_range(*start..*end, new_name);
        }

        if let Err(e) = std::fs::write(&full, &new_content) {
            return ToolResult::err(format!("Cannot write {path}: {e}"), 0);
        }

        ToolResult::ok(format!("Renamed {} occurrences of '{old_name}' to '{new_name}' in {path} (AST-aware)", replacements.len()), 0)
    }
}

fn collect_identifiers(source: &str, node: tree_sitter::Node, target: &str, out: &mut Vec<(usize, usize)>) {
    if node.kind() == "identifier" || node.kind() == "type_identifier" {
        let text = &source[node.start_byte()..node.end_byte()];
        if text == target {
            out.push((node.start_byte(), node.end_byte()));
        }
    }
    let mut cursor = node.walk();
    if cursor.goto_first_child() {
        loop {
            collect_identifiers(source, cursor.node(), target, out);
            if !cursor.goto_next_sibling() { break; }
        }
    }
}

pub struct TreeSitterFindTool;

#[async_trait]
impl Tool for TreeSitterFindTool {
    fn name(&self) -> &str { "ts_find" }
    fn description(&self) -> &str { "Find all occurrences of a symbol in a file using tree-sitter AST" }
    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": {"type": "string"},
                "symbol": {"type": "string"}
            },
            "required": ["path", "symbol"]
        })
    }

    async fn execute(&self, params: Value, ctx: &ToolContext) -> ToolResult {
        let path = params.get("path").and_then(|p| p.as_str()).unwrap_or("");
        let symbol = params.get("symbol").and_then(|p| p.as_str()).unwrap_or("");

        let full = ctx.repo_path.join(path);
        let content = match std::fs::read_to_string(&full) {
            Ok(c) => c,
            Err(e) => return ToolResult::err(format!("Cannot read {path}: {e}"), 0),
        };

        let mut parser = tree_sitter::Parser::new();
        let _ = parser.set_language(&tree_sitter_rust::LANGUAGE.into());
        let Some(tree) = parser.parse(&content, None) else {
            return ToolResult::err(format!("Failed to parse {path}"), 0);
        };

        let mut locations = Vec::new();
        collect_identifiers(&content, tree.root_node(), symbol, &mut locations);

        if locations.is_empty() {
            return ToolResult::ok(format!("No occurrences of '{symbol}' found in {path}"), 0);
        }

        let lines: Vec<&str> = content.lines().collect();
        let mut results = Vec::new();
        for (start, _end) in &locations {
            let line_num = content[..*start].matches('\n').count() + 1;
            let line_text = lines.get(line_num - 1).unwrap_or(&"");
            results.push(format!("  {}:{}: {}", path, line_num, line_text.trim()));
        }

        ToolResult::ok(format!("{} occurrences of '{symbol}':\n{}", locations.len(), results.join("\n")), 0)
    }
}

pub struct TreeSitterSignaturesTool;

#[async_trait]
impl Tool for TreeSitterSignaturesTool {
    fn name(&self) -> &str { "ts_signatures" }
    fn description(&self) -> &str { "Extract all function/struct/impl signatures from a Rust file" }
    fn parameters_schema(&self) -> Value {
        json!({"type": "object", "properties": {"path": {"type": "string"}}, "required": ["path"]})
    }

    async fn execute(&self, params: Value, ctx: &ToolContext) -> ToolResult {
        let path = params.get("path").and_then(|p| p.as_str()).unwrap_or("");
        let full = ctx.repo_path.join(path);
        let content = match std::fs::read_to_string(&full) {
            Ok(c) => c,
            Err(e) => return ToolResult::err(format!("Cannot read {path}: {e}"), 0),
        };

        let mut parser = tree_sitter::Parser::new();
        let _ = parser.set_language(&tree_sitter_rust::LANGUAGE.into());
        let Some(tree) = parser.parse(&content, None) else {
            return ToolResult::err(format!("Failed to parse {path}"), 0);
        };

        let mut sigs = Vec::new();
        extract_sigs(&content, tree.root_node(), &mut sigs);

        if sigs.len() > MAX_SIGNATURES {
            sigs.truncate(MAX_SIGNATURES);
            sigs.push("... (truncated)".to_string());
        }

        ToolResult::ok(sigs.join("\n"), 0)
    }
}

fn extract_sigs(source: &str, node: tree_sitter::Node, out: &mut Vec<String>) {
    let kind = node.kind();
    match kind {
        "function_item" | "struct_item" | "enum_item" | "impl_item" | "trait_item" | "type_item" | "const_item" | "static_item" => {
            let line = node.start_position().row + 1;
            // Get first line of the node as signature
            let start = node.start_byte();
            let text = &source[start..];
            let first_line = text.lines().next().unwrap_or("");
            let sig = first_line.trim_end_matches('{').trim();
            out.push(format!("  {}:{} {}", kind.replace("_item", ""), line, sig));
        }
        _ => {}
    }
    let mut cursor = node.walk();
    if cursor.goto_first_child() {
        loop {
            extract_sigs(source, cursor.node(), out);
            if !cursor.goto_next_sibling() { break; }
        }
    }
}
