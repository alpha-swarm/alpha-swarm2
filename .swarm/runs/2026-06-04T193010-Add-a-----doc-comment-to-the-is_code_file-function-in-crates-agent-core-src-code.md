# Add a /// doc comment to the is_code_file function in crates/agent-core/src/code_utils.rs.

- model: llama3.3:70b
- files: crates/agent-core/src/code_utils.rs
- when: 2026-06-04T19:30:10.975864+00:00

## Agent output

```
<<<EDIT crates/agent-core/src/code_utils.rs
--- OLD
pub fn is_code_file(path: &str) -> bool {
    CODE_EXTENSIONS.iter().any(|ext| path.ends_with(&format!(".{ext}")))
}

--- NEW
/// Check if a file has a code extension that triggers quality gates.
pub fn is_code_file(path: &str) -> bool {
    CODE_EXTENSIONS.iter().any(|ext| path.ends_with(&format!(".{ext}")))
}
>>>
```

## Diff

```diff
--- a/crates/agent-core/src/code_utils.rs
+++ b/crates/agent-core/src/code_utils.rs
@@ -30,6 +30,7 @@
 pub const SKIP_DIRS: &[&str] = &[".git", "target", "node_modules", "dist", ".wash"];
 
 /// Check if a file has a code extension (triggers quality gate).
+/// Check if a file has a code extension that triggers quality gates.
 pub fn is_code_file(path: &str) -> bool {
     CODE_EXTENSIONS.iter().any(|ext| path.ends_with(&format!(".{ext}")))
 }

```
