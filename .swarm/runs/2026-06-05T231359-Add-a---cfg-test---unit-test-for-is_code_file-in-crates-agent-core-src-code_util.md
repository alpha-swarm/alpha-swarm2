# Add a #[cfg(test)] unit test for is_code_file in crates/agent-core/src/code_utils.rs

- model: unknown
- files: crates/agent-core/src/code_utils.rs
- when: 2026-06-05T23:13:59.478119+00:00

## Agent output

```

```

## Diff

```diff
--- a/crates/agent-core/src/code_utils.rs
+++ b/crates/agent-core/src/code_utils.rs
@@ -152,4 +152,12 @@
         assert_eq!(fuzzy_replace("let a = 1;\n", "nonexistent line", "x"), None);
         assert_eq!(fuzzy_replace("anything", "   ", "x"), None);
     }
+
+    #[test]
+    fn test_is_code_file() {
+        assert!(is_code_file("src/main.rs"));
+        assert!(is_code_file("lib.rs"));
+        assert!(!is_code_file("README.md"));
+        assert!(!is_code_file("config.toml"));
+    }
 }

```
