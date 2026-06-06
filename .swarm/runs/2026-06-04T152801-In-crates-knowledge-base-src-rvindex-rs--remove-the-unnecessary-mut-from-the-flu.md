# In crates/knowledge-base/src/rvindex.rs, remove the unnecessary mut from the flush closure let-binding (around line 32) to clear the unused_mut warning. Edit only that file; keep behavior identical.

- model: direct
- files: crates/knowledge-base/src/rvindex.rs
- when: 2026-06-04T15:28:01.101597+00:00

## Agent output

```
Direct edit applied
```

## Diff

```diff
--- a/crates/knowledge-base/src/rvindex.rs
+++ b/crates/knowledge-base/src/rvindex.rs
@@ -27,7 +27,7 @@
 /// tokens, FNV-1a-hashed to u32 term ids, weight = sqrt(tf)). A pragmatic
 /// BM25-lite signal for hybrid retrieval.
 fn tokenize_sparse(text: &str) -> SparseVector {
-    let mut tf: HashMap<u32, f32> = HashMap::new();
+    let tf: HashMap<u32, f32> = HashMap::new();
     let mut tok = String::new();
     let mut flush = |t: &mut String, tf: &mut HashMap<u32, f32>| {
         if t.len() >= 2 {

```
