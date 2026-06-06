# In crates/knowledge-base/src/rvindex.rs, remove the unnecessary mut from the flush closure let-binding (around line 32) to clear the unused_mut warning. Edit only that file; keep behavior identical.

- model: direct
- files: crates/knowledge-base/src/rvindex.rs
- when: 2026-06-04T16:01:33.704043+00:00

## Agent output

```
Direct edit applied
```

## Diff

```diff
--- a/crates/knowledge-base/src/rvindex.rs
+++ b/crates/knowledge-base/src/rvindex.rs
@@ -29,7 +29,7 @@
 fn tokenize_sparse(text: &str) -> SparseVector {
     let mut tf: HashMap<u32, f32> = HashMap::new();
     let mut tok = String::new();
-    let mut flush = |t: &mut String, tf: &mut HashMap<u32, f32>| {
+    let flush = |t: &mut String, tf: &mut HashMap<u32, f32>| {
         if t.len() >= 2 {
             *tf.entry(fnv1a32(t)).or_insert(0.0) += 1.0;
         }

```
