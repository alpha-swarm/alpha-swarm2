/// Shared utilities for code analysis — signature extraction, file classification.

/// Check if a line is a function/struct/trait/impl signature.
pub fn is_signature_line(line: &str) -> bool {
    let t = line.trim();
    t.starts_with("pub fn ") || t.starts_with("fn ")
        || t.starts_with("pub struct ") || t.starts_with("struct ")
        || t.starts_with("pub enum ") || t.starts_with("enum ")
        || t.starts_with("pub trait ") || t.starts_with("trait ")
        || t.starts_with("impl ") || t.starts_with("pub async fn ")
        || t.starts_with("pub type ") || t.starts_with("pub const ")
}

/// Extract signature lines from source code.
pub fn extract_signatures(content: &str, max: usize) -> Vec<String> {
    content.lines()
        .filter(|l| is_signature_line(l))
        .take(max)
        .map(|l| l.trim().to_string())
        .collect()
}

/// Code file extensions that trigger quality gates.
pub const CODE_EXTENSIONS: &[&str] = &["rs", "ts", "js", "go", "py"];

/// All indexable file extensions (code + config + docs).
pub const INDEXABLE_EXTENSIONS: &[&str] = &["rs", "ts", "js", "go", "py", "md", "toml", "json", "yaml", "yml"];

/// Directories to skip when walking a repo.
pub const SKIP_DIRS: &[&str] = &[".git", "target", "node_modules", "dist", ".wash"];

/// Check if a file has a code extension (triggers quality gate).
pub fn is_code_file(path: &str) -> bool {
    CODE_EXTENSIONS.iter().any(|ext| path.ends_with(&format!(".{ext}")))
}
