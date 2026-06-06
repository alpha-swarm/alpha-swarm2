# In crates/agent-core/src/code_utils.rs, add a #[cfg(test)] unit test named fuzzy_replace_multiline_to_single that asserts fuzzy_replace replacing a two-line OLD block with a one-line NEW works. Edit only that file.

- model: qwen2.5-coder:14b
- files: crates/agent-core/src/code_utils.rs
- when: 2026-06-05T13:59:23.823609+00:00

## Agent output

```
```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fuzzy_replace_multiline_to_single() {
        let content = "a\n  foo();\n  bar();\nz\n";
        let got = fuzzy_replace(content, "foo();\nbar();", "baz();").unwrap();
        assert!(got.contains("baz();"));
        assert!(!got.contains("foo();"));
        assert!(!got.contains("bar();"));
    }
}
```
```

## Diff

```diff
--- a/Cargo.lock
+++ b/Cargo.lock
@@ -88,6 +88,7 @@
  "sysinfo 0.33.1",
  "tokio",
  "tool-host",
+ "tower-http",
  "tracing",
  "tracing-subscriber",
  "uuid",
@@ -2697,6 +2698,12 @@
 ]
 
 [[package]]
+name = "http-range-header"
+version = "0.4.2"
+source = "registry+https://github.com/rust-lang/crates.io-index"
+checksum = "9171a2ea8a68358193d15dd5d70c1c10a2afc3e7e4c5bc92bc9f025cebd7359c"
+
+[[package]]
 name = "httparse"
 version = "1.10.1"
 source = "registry+https://github.com/rust-lang/crates.io-index"
@@ -7012,14 +7019,24 @@
 dependencies = [
  "bitflags",
  "bytes",
+ "futures-core",
  "futures-util",
  "http",
  "http-body",
+ "http-body-util",
+ "http-range-header",
+ "httpdate",
  "iri-string",
+ "mime",
+ "mime_guess",
+ "percent-encoding",
  "pin-project-lite",
+ "tokio",
+ "tokio-util",
  "tower",
  "tower-layer",
  "tower-service",
+ "tracing",
 ]
 
 [[package]]

```
