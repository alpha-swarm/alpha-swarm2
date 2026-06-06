# In crates/config/src/lib.rs, add a #[cfg(test)] unit test asserting SecurityConfig::default().rules_enabled is true and fail_severity equals the string high. Edit only that file.

- model: unknown
- files: crates/config/src/lib.rs
- when: 2026-06-05T13:58:35.710567+00:00

## Agent output

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
