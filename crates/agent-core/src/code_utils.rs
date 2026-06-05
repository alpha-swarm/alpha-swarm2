//! Shared utilities for code analysis — signature extraction, file classification.

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

/// Apply an OLD→NEW edit to `content`, tolerating the whitespace/line-ending
/// drift local models routinely produce. Returns the updated content, or
/// `None` when the OLD block cannot be located — callers MUST treat `None` as a
/// failed edit, never as a silent no-op (a plain `str::replacen` that misses
/// rewrites the file unchanged yet looks "applied", which hides broken edits
/// from the quality gate).
///
/// Match order: exact (after line-ending normalize + trim) → single-line
/// trimmed search → multi-line trimmed line-by-line search.
pub fn fuzzy_replace(content: &str, old: &str, new: &str) -> Option<String> {
    let content_norm = content.replace("\r\n", "\n");
    let old_norm = old.replace("\r\n", "\n");
    let new_norm = new.replace("\r\n", "\n");
    let old_trimmed = old_norm.trim();
    if old_trimmed.is_empty() {
        return None;
    }
    if content_norm.contains(old_trimmed) {
        return Some(content_norm.replacen(old_trimmed, new_norm.trim(), 1));
    }
    let lines: Vec<&str> = content_norm.lines().collect();
    let old_lines: Vec<&str> = old_trimmed.lines().map(|l| l.trim()).collect();
    let new_lines = || new_norm.trim().lines().map(|l| l.to_string());

    if old_lines.len() == 1 {
        let idx = lines.iter().position(|l| l.trim() == old_lines[0])?;
        let mut result: Vec<String> = lines[..idx].iter().map(|l| l.to_string()).collect();
        result.extend(new_lines());
        result.extend(lines[idx + 1..].iter().map(|l| l.to_string()));
        return Some(result.join("\n"));
    }

    // Multi-line: find the window whose trimmed lines all match.
    let start = (0..lines.len()).find(|&i| {
        lines[i].trim() == old_lines[0]
            && old_lines
                .iter()
                .enumerate()
                .all(|(j, ol)| i + j < lines.len() && lines[i + j].trim() == *ol)
    })?;
    let mut result: Vec<String> = lines[..start].iter().map(|l| l.to_string()).collect();
    result.extend(new_lines());
    result.extend(lines[start + old_lines.len()..].iter().map(|l| l.to_string()));
    Some(result.join("\n"))
}

/// All indexable file extensions (code + config + docs).
pub const INDEXABLE_EXTENSIONS: &[&str] = &["rs", "ts", "js", "go", "py", "md", "toml", "json", "yaml", "yml"];

/// Directories to skip when walking a repo.
pub const SKIP_DIRS: &[&str] = &[".git", "target", "node_modules", "dist", ".wash"];

/// Check if a file has a code extension (triggers quality gate).
pub fn is_code_file(path: &str) -> bool {
    CODE_EXTENSIONS.iter().any(|ext| path.ends_with(&format!(".{ext}")))
}

/// Reject obviously invalid/placeholder file paths from model output.
/// Returns true if the path looks like a real project file.
pub fn is_valid_file_path(path: &str) -> bool {
    let path = path.trim();
    if path.is_empty() { return false; }

    // Reject placeholder/example paths
    const REJECT_PATTERNS: &[&str] = &[
        "path/to/",
        "example",
        "your_",
        "my_file",
        "new_file.rs",
        "old_file",
        "{",
        "<",
        "...",
        "file.ext",
        "foo.",
        "bar.",
        "baz.",
        "test_file",
    ];

    let lower = path.to_lowercase();
    for pattern in REJECT_PATTERNS {
        if lower.contains(pattern) { return false; }
    }

    // Must have a file extension
    if !path.contains('.') { return false; }

    // Must not start with / (absolute paths)
    if path.starts_with('/') { return false; }

    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fuzzy_replace_exact() {
        let got = fuzzy_replace("let a = 1;\nlet b = 2;\n", "let a = 1;", "let a = 10;");
        assert_eq!(got.as_deref(), Some("let a = 10;\nlet b = 2;\n"));
    }

    #[test]
    fn fuzzy_replace_tolerates_indentation_drift() {
        // OLD has different leading whitespace than the file — exact replacen
        // would miss; the trimmed line search must still match.
        let content = "fn main() {\n    let x = 1;\n}\n";
        let got = fuzzy_replace(content, "let x = 1;", "let x = 2;");
        assert!(got.is_some());
        assert!(got.unwrap().contains("let x = 2;"));
    }

    #[test]
    fn fuzzy_replace_multiline() {
        let content = "a\n  foo();\n  bar();\nz\n";
        let got = fuzzy_replace(content, "foo();\nbar();", "baz();").unwrap();
        assert!(got.contains("baz();"));
        assert!(!got.contains("foo();"));
    }

    #[test]
    fn fuzzy_replace_miss_returns_none() {
        // The critical contract: a non-matching OLD is None, NOT a silent no-op.
        assert_eq!(fuzzy_replace("let a = 1;\n", "nonexistent line", "x"), None);
        assert_eq!(fuzzy_replace("anything", "   ", "x"), None);
    }

    #[test]
    fn test_is_code_file() {
        assert!(is_code_file("src/main.rs"));
        assert!(is_code_file("lib.rs"));
        assert!(!is_code_file("README.md"));
        assert!(!is_code_file("config.toml"));
    }
}
