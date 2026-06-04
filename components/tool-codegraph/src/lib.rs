//! Codegraph WASM tool component (Wassette).
//!
//! Exports `extract-graph` as a single WIT function → one MCP tool. Reuses the
//! multi-language tree-sitter extraction logic that lives natively in
//! `swarm_tools::codegraph`; ported here (no native-only cfg gates) so it
//! compiles to wasm32-wasip2 and runs sandboxed under Wassette with a
//! filesystem grant for the target repo.
//!
//! NOTE: the extraction body is intentionally a copy of
//! `crates/tools/src/codegraph.rs`. A future refactor can factor the pure
//! extraction into a shared no-deps crate consumed by both.

wit_bindgen::generate!({
    path: "wit",
    world: "codegraph",
    generate_all,
});

use serde::Serialize;

struct Component;
export!(Component);

impl Guest for Component {
    fn extract_graph(repo_path: String, files: Vec<String>) -> Result<String, String> {
        let g = extract(std::path::Path::new(&repo_path), &files);
        serde_json::to_string(&g).map_err(|e| e.to_string())
    }
}

// ----------------------------------------------------------------------------
// Extraction (ported from swarm_tools::codegraph; tree-sitter, multi-language)
// ----------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
struct Entity {
    kind: String,
    name: String,
    file: String,
    line: usize,
    lang: String,
}

#[derive(Debug, Clone, Serialize)]
struct Relation {
    from: String,
    kind: String,
    to: String,
    file: String,
}

#[derive(Debug, Clone, Default, Serialize)]
struct CodeGraph {
    entities: Vec<Entity>,
    relations: Vec<Relation>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Lang {
    Rust,
    Python,
    JavaScript,
    TypeScript,
    Tsx,
    Go,
}

impl Lang {
    fn detect(file: &str) -> Option<Lang> {
        let ext = file.rsplit('.').next().unwrap_or("");
        match ext {
            "rs" => Some(Lang::Rust),
            "py" | "pyi" => Some(Lang::Python),
            "js" | "jsx" | "mjs" | "cjs" => Some(Lang::JavaScript),
            "ts" | "mts" | "cts" => Some(Lang::TypeScript),
            "tsx" => Some(Lang::Tsx),
            "go" => Some(Lang::Go),
            _ => None,
        }
    }

    fn tag(self) -> &'static str {
        match self {
            Lang::Rust => "rust",
            Lang::Python => "python",
            Lang::JavaScript => "javascript",
            Lang::TypeScript | Lang::Tsx => "typescript",
            Lang::Go => "go",
        }
    }

    fn language(self) -> tree_sitter::Language {
        match self {
            Lang::Rust => tree_sitter_rust::LANGUAGE.into(),
            Lang::Python => tree_sitter_python::LANGUAGE.into(),
            Lang::JavaScript => tree_sitter_javascript::LANGUAGE.into(),
            Lang::TypeScript => tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
            Lang::Tsx => tree_sitter_typescript::LANGUAGE_TSX.into(),
            Lang::Go => tree_sitter_go::LANGUAGE.into(),
        }
    }

    fn defs(self) -> &'static [(&'static str, &'static str)] {
        match self {
            Lang::Rust => &[
                ("function_item", "function"),
                ("struct_item", "struct"),
                ("enum_item", "enum"),
                ("trait_item", "trait"),
                ("type_item", "type"),
                ("const_item", "const"),
                ("static_item", "static"),
                ("mod_item", "mod"),
            ],
            Lang::Python => &[
                ("function_definition", "function"),
                ("class_definition", "class"),
            ],
            Lang::JavaScript => &[
                ("function_declaration", "function"),
                ("generator_function_declaration", "function"),
                ("class_declaration", "class"),
                ("method_definition", "method"),
            ],
            Lang::TypeScript | Lang::Tsx => &[
                ("function_declaration", "function"),
                ("generator_function_declaration", "function"),
                ("class_declaration", "class"),
                ("interface_declaration", "interface"),
                ("type_alias_declaration", "type"),
                ("enum_declaration", "enum"),
                ("method_definition", "method"),
            ],
            Lang::Go => &[
                ("function_declaration", "function"),
                ("method_declaration", "method"),
                ("type_spec", "type"),
                ("type_alias", "type"),
            ],
        }
    }

    fn import_kinds(self) -> &'static [&'static str] {
        match self {
            Lang::Rust => &["use_declaration"],
            Lang::Python => &["import_statement", "import_from_statement"],
            Lang::JavaScript | Lang::TypeScript | Lang::Tsx => &["import_statement"],
            Lang::Go => &["import_declaration"],
        }
    }
}

impl PartialOrd for Lang {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}
impl Ord for Lang {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        (*self as u8).cmp(&(*other as u8))
    }
}

fn extract(repo_path: &std::path::Path, files: &[String]) -> CodeGraph {
    let mut g = CodeGraph::default();
    let mut parser = tree_sitter::Parser::new();
    let mut by_lang: std::collections::BTreeMap<Lang, Vec<&String>> = std::collections::BTreeMap::new();
    for file in files {
        if let Some(lang) = Lang::detect(file) {
            by_lang.entry(lang).or_default().push(file);
        }
    }
    for (lang, lang_files) in by_lang {
        if parser.set_language(&lang.language()).is_err() {
            continue;
        }
        for file in lang_files {
            let Ok(src) = std::fs::read_to_string(repo_path.join(file)) else { continue };
            let Some(tree) = parser.parse(&src, None) else { continue };
            walk(lang, &src, tree.root_node(), file, &mut g);
        }
    }
    g
}

fn node_text(src: &str, node: tree_sitter::Node) -> String {
    src[node.start_byte()..node.end_byte()].to_string()
}

fn node_name(src: &str, node: tree_sitter::Node) -> Option<String> {
    let n = node.child_by_field_name("name")?;
    Some(node_text(src, n))
}

fn import_targets(lang: Lang, src: &str, node: tree_sitter::Node) -> Vec<String> {
    let mut out = Vec::new();
    match lang {
        Lang::Rust => {
            if let Some(c) = node.named_child(0) {
                out.push(node_text(src, c));
            }
        }
        Lang::Python => {
            collect_kinds(src, node, &["dotted_name", "relative_import"], &mut out);
        }
        Lang::JavaScript | Lang::TypeScript | Lang::Tsx => {
            if let Some(s) = node.child_by_field_name("source") {
                out.push(strip_quotes(&node_text(src, s)));
            }
        }
        Lang::Go => {
            collect_kinds(src, node, &["interpreted_string_literal", "raw_string_literal"], &mut out);
            out = out.into_iter().map(|s| strip_quotes(&s)).collect();
        }
    }
    if out.is_empty() {
        if let Some(line) = node_text(src, node).lines().next() {
            out.push(line.trim().to_string());
        }
    }
    out.retain(|s| !s.is_empty());
    out
}

fn collect_kinds(src: &str, node: tree_sitter::Node, kinds: &[&str], out: &mut Vec<String>) {
    let mut cursor = node.walk();
    let mut stack = vec![node];
    while let Some(n) = stack.pop() {
        if kinds.contains(&n.kind()) {
            out.push(node_text(src, n));
            continue;
        }
        for child in n.named_children(&mut cursor) {
            stack.push(child);
        }
    }
}

fn strip_quotes(s: &str) -> String {
    s.trim_matches(|c| c == '"' || c == '`' || c == '\'').to_string()
}

fn parents(lang: Lang, src: &str, node: tree_sitter::Node) -> Vec<String> {
    let mut out = Vec::new();
    match lang {
        Lang::Python => {
            if let Some(sc) = node.child_by_field_name("superclasses") {
                collect_kinds(src, sc, &["identifier", "attribute"], &mut out);
            }
        }
        Lang::JavaScript | Lang::TypeScript | Lang::Tsx => {
            let mut cursor = node.walk();
            for child in node.named_children(&mut cursor) {
                if child.kind() == "class_heritage" {
                    collect_kinds(src, child, &["identifier", "type_identifier"], &mut out);
                }
            }
        }
        Lang::Rust | Lang::Go => {}
    }
    out
}

fn walk(lang: Lang, src: &str, node: tree_sitter::Node, file: &str, g: &mut CodeGraph) {
    let kind = node.kind();
    let line = node.start_position().row + 1;

    if let Some((_, label)) = lang.defs().iter().find(|(k, _)| *k == kind) {
        if let Some(name) = node_name(src, node) {
            g.entities.push(Entity {
                kind: (*label).to_string(),
                name: name.clone(),
                file: file.into(),
                line,
                lang: lang.tag().into(),
            });
            g.relations.push(Relation {
                from: file.into(),
                kind: "defines".into(),
                to: name.clone(),
                file: file.into(),
            });
            for parent in parents(lang, src, node) {
                g.relations.push(Relation {
                    from: name.clone(),
                    kind: "extends".into(),
                    to: parent,
                    file: file.into(),
                });
            }
        }
    }

    if lang == Lang::Rust && kind == "impl_item" {
        let ty = node.child_by_field_name("type").map(|n| node_text(src, n));
        let tr = node.child_by_field_name("trait").map(|n| node_text(src, n));
        if let (Some(ty), Some(tr)) = (ty.clone(), tr) {
            g.relations.push(Relation {
                from: ty,
                kind: "implements".into(),
                to: tr,
                file: file.into(),
            });
        }
        if let Some(ty) = ty {
            g.entities.push(Entity {
                kind: "impl".into(),
                name: ty,
                file: file.into(),
                line,
                lang: lang.tag().into(),
            });
        }
    }

    if lang.import_kinds().contains(&kind) {
        for path in import_targets(lang, src, node) {
            g.relations.push(Relation {
                from: file.into(),
                kind: "imports".into(),
                to: path,
                file: file.into(),
            });
        }
    }

    let mut cursor = node.walk();
    if cursor.goto_first_child() {
        loop {
            walk(lang, src, cursor.node(), file, g);
            if !cursor.goto_next_sibling() {
                break;
            }
        }
    }
}
