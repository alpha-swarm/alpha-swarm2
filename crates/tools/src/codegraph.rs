//! Code knowledge-graph extraction (multi-language, tree-sitter).
//!
//! Pure extraction: walks source files into entities (functions, types,
//! classes, traits, interfaces, modules…) and relations between them
//! (`defines` file→entity, `implements`/`extends` type→parent, `imports`
//! file→module-path). Storage + traversal live in `knowledge-base::graph`;
//! this crate just owns the tree-sitter dependency.
//!
//! ## Language coverage
//!
//! tree-sitter is a parser framework — each language ships its own grammar
//! crate, and every grammar names its AST nodes differently (Rust
//! `function_item`, Python `function_definition`, TS `function_declaration`,
//! Go `function_declaration`…). So extraction is NOT one generic walk: each
//! supported language gets a `LangSpec` mapping its node kinds to our entity
//! labels plus a per-language import extractor. Adding a language = add a
//! grammar dep + one `LangSpec`. The storage/BFS layer is language-agnostic.
//!
//! Supported today: Rust, Python, JavaScript, TypeScript (+TSX), Go.
//! Unknown extensions are skipped (no partial/garbage graphs).

use serde::{Deserialize, Serialize};

/// A code entity (node in the knowledge graph).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Entity {
    pub kind: String, // function | struct | class | enum | trait | interface | type | impl | mod | method | const | static
    pub name: String,
    pub file: String,
    pub line: usize,
    /// Source language tag (rust | python | javascript | typescript | go).
    pub lang: String,
}

/// A directed relation (edge): `from` --kind--> `to`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Relation {
    pub from: String, // entity name or file path
    pub kind: String, // defines | implements | extends | imports
    pub to: String,   // entity name / trait name / parent class / module path
    pub file: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CodeGraph {
    pub entities: Vec<Entity>,
    pub relations: Vec<Relation>,
}

/// Supported source languages. Each maps to exactly one tree-sitter grammar.
#[cfg(feature = "native")]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Lang {
    Rust,
    Python,
    JavaScript,
    TypeScript,
    Tsx,
    Go,
}

#[cfg(feature = "native")]
impl Lang {
    /// Map a file path to a language by extension (None = skip the file).
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

    /// Short stable tag stored on every entity.
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

    /// Definition node kinds → our entity label. Each listed node MUST expose
    /// a `name` field (verified per grammar). The recursive walk descends into
    /// containers, so e.g. Go `type_spec` (under `type_declaration`) and
    /// methods inside classes are picked up generically.
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

    /// Top-level import statement node kinds for this language.
    fn import_kinds(self) -> &'static [&'static str] {
        match self {
            Lang::Rust => &["use_declaration"],
            Lang::Python => &["import_statement", "import_from_statement"],
            Lang::JavaScript | Lang::TypeScript | Lang::Tsx => &["import_statement"],
            Lang::Go => &["import_declaration"],
        }
    }
}

/// Whether a path's extension maps to a supported grammar. Callers enumerating
/// a repo use this to pre-filter before handing files to `extract`.
#[cfg(feature = "native")]
pub fn is_supported_file(path: &str) -> bool {
    Lang::detect(path).is_some()
}

/// Extract a code graph from the given files under `repo_path`. Files whose
/// extension maps to no supported grammar are skipped. Files are grouped by
/// language so each grammar is loaded into the parser at most once per group.
#[cfg(feature = "native")]
pub fn extract(repo_path: &std::path::Path, files: &[String]) -> CodeGraph {
    let mut g = CodeGraph::default();
    let mut parser = tree_sitter::Parser::new();
    // Sort files by language so we minimize parser.set_language() churn.
    let mut by_lang: std::collections::BTreeMap<Lang, Vec<&String>> = std::collections::BTreeMap::new();
    for file in files {
        if let Some(lang) = Lang::detect(file) {
            by_lang.entry(lang).or_default().push(file);
        }
    }
    for (lang, lang_files) in by_lang {
        if parser.set_language(&lang.language()).is_err() {
            continue; // grammar/ABI mismatch — skip this language, keep others
        }
        for file in lang_files {
            let Ok(src) = std::fs::read_to_string(repo_path.join(file)) else { continue };
            let Some(tree) = parser.parse(&src, None) else { continue };
            walk(lang, &src, tree.root_node(), file, &mut g);
        }
    }
    g
}

// --- ordering glue so Lang can key a BTreeMap (cheap, no Hash dep) ---
#[cfg(feature = "native")]
impl PartialOrd for Lang {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}
#[cfg(feature = "native")]
impl Ord for Lang {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        (*self as u8).cmp(&(*other as u8))
    }
}

#[cfg(feature = "native")]
fn node_text(src: &str, node: tree_sitter::Node) -> String {
    src[node.start_byte()..node.end_byte()].to_string()
}

#[cfg(feature = "native")]
fn node_name(src: &str, node: tree_sitter::Node) -> Option<String> {
    let n = node.child_by_field_name("name")?;
    Some(node_text(src, n))
}

/// Extract import targets from a single import statement, per language.
/// Best-effort: returns one entry per imported module/path where the grammar
/// exposes it cleanly; falls back to the first line of source text.
#[cfg(feature = "native")]
fn import_targets(lang: Lang, src: &str, node: tree_sitter::Node) -> Vec<String> {
    let mut out = Vec::new();
    match lang {
        // `use a::b::c;` → first named child is the path/tree.
        Lang::Rust => {
            if let Some(c) = node.named_child(0) {
                out.push(node_text(src, c));
            }
        }
        // `import a.b` / `from a.b import c` → the dotted module name(s).
        Lang::Python => {
            collect_kinds(src, node, &["dotted_name", "relative_import"], &mut out);
        }
        // `import x from "mod"` → the `source` string literal.
        Lang::JavaScript | Lang::TypeScript | Lang::Tsx => {
            if let Some(s) = node.child_by_field_name("source") {
                out.push(strip_quotes(&node_text(src, s)));
            }
        }
        // `import ( "fmt"; "x/y" )` or `import "fmt"` → string literal paths.
        Lang::Go => {
            collect_kinds(src, node, &["interpreted_string_literal", "raw_string_literal"], &mut out);
            out = out.into_iter().map(|s| strip_quotes(&s)).collect();
        }
    }
    if out.is_empty() {
        // Fallback: first non-empty line of the statement, capped.
        if let Some(line) = node_text(src, node).lines().next() {
            out.push(line.trim().to_string());
        }
    }
    out.retain(|s| !s.is_empty());
    out
}

/// Collect text of all descendant nodes whose kind is in `kinds`.
#[cfg(feature = "native")]
fn collect_kinds(src: &str, node: tree_sitter::Node, kinds: &[&str], out: &mut Vec<String>) {
    let mut cursor = node.walk();
    let mut stack = vec![node];
    while let Some(n) = stack.pop() {
        if kinds.contains(&n.kind()) {
            out.push(node_text(src, n));
            continue; // don't descend into a matched node
        }
        for child in n.named_children(&mut cursor) {
            stack.push(child);
        }
    }
}

#[cfg(feature = "native")]
fn strip_quotes(s: &str) -> String {
    s.trim_matches(|c| c == '"' || c == '`' || c == '\'').to_string()
}

/// Parent type names a class/struct inherits from (best-effort, per language).
#[cfg(feature = "native")]
fn parents(lang: Lang, src: &str, node: tree_sitter::Node) -> Vec<String> {
    let mut out = Vec::new();
    match lang {
        // class C(Base1, Base2): → superclasses argument_list.
        Lang::Python => {
            if let Some(sc) = node.child_by_field_name("superclasses") {
                collect_kinds(src, sc, &["identifier", "attribute"], &mut out);
            }
        }
        // class C extends B implements I → class_heritage subtree.
        Lang::JavaScript | Lang::TypeScript | Lang::Tsx => {
            let mut cursor = node.walk();
            for child in node.named_children(&mut cursor) {
                if child.kind() == "class_heritage" {
                    collect_kinds(src, child, &["identifier", "type_identifier"], &mut out);
                }
            }
        }
        Lang::Rust | Lang::Go => {} // Rust impl handled separately; Go is structural
    }
    out
}

#[cfg(feature = "native")]
fn walk(lang: Lang, src: &str, node: tree_sitter::Node, file: &str, g: &mut CodeGraph) {
    let kind = node.kind();
    let line = node.start_position().row + 1;

    // Generic definition handling (name-field nodes).
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
            // Class/struct inheritance (extends) for OO languages.
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

    // Rust `impl Trait for Type` → implements edge Type -> Trait + impl anchor.
    if lang == Lang::Rust && kind == "impl_item" {
        let ty = node
            .child_by_field_name("type")
            .map(|n| node_text(src, n));
        let tr = node
            .child_by_field_name("trait")
            .map(|n| node_text(src, n));
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

    // Imports.
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

#[cfg(all(test, feature = "native"))]
mod tests {
    use super::*;

    fn write(dir: &std::path::Path, name: &str, body: &str) {
        std::fs::write(dir.join(name), body).unwrap();
    }

    #[test]
    fn extracts_rust() {
        let dir = std::env::temp_dir().join(format!("cg-rs-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        write(&dir, "a.rs", r#"
use std::sync::Arc;
pub struct Widget { n: u32 }
pub trait Render { fn render(&self); }
impl Render for Widget { fn render(&self) {} }
pub fn build() -> Widget { Widget { n: 0 } }
"#);
        let g = extract(&dir, &["a.rs".to_string()]);
        let names: Vec<&str> = g.entities.iter().map(|e| e.name.as_str()).collect();
        assert!(names.contains(&"Widget"));
        assert!(names.contains(&"Render"));
        assert!(names.contains(&"build"));
        assert!(g.relations.iter().any(|r| r.kind == "implements" && r.from == "Widget" && r.to == "Render"));
        assert!(g.relations.iter().any(|r| r.kind == "imports" && r.to.contains("Arc")));
        assert!(g.relations.iter().any(|r| r.kind == "defines" && r.to == "build"));
        assert!(g.entities.iter().all(|e| e.lang == "rust"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn extracts_python() {
        let dir = std::env::temp_dir().join(format!("cg-py-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        write(&dir, "a.py", "import os\nfrom collections import OrderedDict\nclass Foo(Base):\n    def method(self):\n        pass\ndef top():\n    pass\n");
        let g = extract(&dir, &["a.py".to_string()]);
        let names: Vec<&str> = g.entities.iter().map(|e| e.name.as_str()).collect();
        assert!(names.contains(&"Foo"), "entities: {names:?}");
        assert!(names.contains(&"top"));
        assert!(g.relations.iter().any(|r| r.kind == "extends" && r.from == "Foo" && r.to == "Base"),
            "missing extends: {:?}", g.relations);
        assert!(g.relations.iter().any(|r| r.kind == "imports" && r.to.contains("os")));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn extracts_typescript() {
        let dir = std::env::temp_dir().join(format!("cg-ts-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        write(&dir, "a.ts", "import { Thing } from \"./thing\";\ninterface Shape { area(): number; }\nclass Circle extends Base implements Shape { area() { return 1; } }\nfunction make(): Circle { return new Circle(); }\n");
        let g = extract(&dir, &["a.ts".to_string()]);
        let names: Vec<&str> = g.entities.iter().map(|e| e.name.as_str()).collect();
        assert!(names.contains(&"Circle"), "entities: {names:?}");
        assert!(names.contains(&"Shape"));
        assert!(names.contains(&"make"));
        assert!(g.relations.iter().any(|r| r.kind == "extends" && r.from == "Circle"),
            "missing extends: {:?}", g.relations);
        assert!(g.relations.iter().any(|r| r.kind == "imports" && r.to.contains("thing")));
        assert!(g.entities.iter().any(|e| e.lang == "typescript"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn extracts_go() {
        let dir = std::env::temp_dir().join(format!("cg-go-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        write(&dir, "a.go", "package main\nimport \"fmt\"\ntype Widget struct { n int }\nfunc (w Widget) Render() {}\nfunc Build() Widget { return Widget{} }\n");
        let g = extract(&dir, &["a.go".to_string()]);
        let names: Vec<&str> = g.entities.iter().map(|e| e.name.as_str()).collect();
        assert!(names.contains(&"Widget"), "entities: {names:?}");
        assert!(names.contains(&"Build"));
        assert!(names.contains(&"Render"));
        assert!(g.relations.iter().any(|r| r.kind == "imports" && r.to.contains("fmt")));
        assert!(g.entities.iter().any(|e| e.lang == "go"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn skips_unknown_extensions() {
        let dir = std::env::temp_dir().join(format!("cg-skip-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        write(&dir, "a.txt", "this is not code");
        let g = extract(&dir, &["a.txt".to_string()]);
        assert!(g.entities.is_empty());
        assert!(g.relations.is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
