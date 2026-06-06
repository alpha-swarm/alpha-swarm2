# In crates/config/src/lib.rs, add a convenience method to WassetteConfig: pub fn is_enabled(&self) -> bool { self.enabled } with a one-line doc comment, in a new impl block. Edit only that file.

- model: direct
- files: crates/config/src/lib.rs
- when: 2026-06-04T15:36:24.943163+00:00

## Agent output

```
Direct edit applied
```

## Diff

```diff
--- a/crates/config/src/lib.rs
+++ b/crates/config/src/lib.rs
@@ -1,4 +1,6 @@
-use serde::Deserialize;
+pub fn is_enabled(&self) -> bool { self.enabled }
+        /// Returns whether the config is enabled.
+    use serde::Deserialize;
 
 /// Central configuration for alpha-swarm.
 ///

```
