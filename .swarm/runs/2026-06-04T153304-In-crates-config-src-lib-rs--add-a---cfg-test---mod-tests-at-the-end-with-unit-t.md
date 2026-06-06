# In crates/config/src/lib.rs, add a #[cfg(test)] mod tests at the end with unit tests asserting AutopilotConfig::default().enabled is false, WassetteConfig::default().enabled is false, and OllamaConfig::default().keep_alive equals "-1". Edit only that file.

- model: direct
- files: crates/config/src/lib.rs
- when: 2026-06-04T15:33:04.705323+00:00

## Agent output

```
Direct edit applied
```

## Diff

```diff
--- a/crates/config/src/lib.rs
+++ b/crates/config/src/lib.rs
@@ -1,4 +1,20 @@
-use serde::Deserialize;
+#[cfg(test)]
+mod tests {
+    #[test]
+    fn test_autopilot_config() {
+        assert!(!AutopilotConfig::default().enabled);
+    }
+
+    #[test]
+    fn test_wassette_config() {
+        assert!(!WassetteConfig::default().enabled);
+    }
+
+    #[test]
+    fn test_ollama_config() {
+        assert_eq!(OllamaConfig::default().keep_alive, "-1");
+    }
+}use serde::Deserialize;
 
 /// Central configuration for alpha-swarm.
 ///

```
