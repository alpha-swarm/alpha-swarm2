# In crates/config/src/lib.rs, add a small convenience accessor to WassetteConfig: a new impl block with pub fn is_enabled(&self) -> bool { self.enabled } and a one-line doc comment. Edit only crates/config/src/lib.rs.

- model: direct
- files: crates/config/src/lib.rs
- when: 2026-06-04T14:58:24.259240+00:00

## Agent output

```
Direct edit applied
```

## Diff

```diff
--- a/crates/config/src/lib.rs
+++ b/crates/config/src/lib.rs
@@ -1,4 +1,7 @@
-use serde::Deserialize;
+impl WassetteConfig {
+    /// Returns whether the config is enabled.
+    pub fn is_enabled(&self) -> bool { self.enabled }
+}use serde::Deserialize;
 
 /// Central configuration for alpha-swarm.
 ///

```
