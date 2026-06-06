# Add a Rust /// doc comment to the sanitize_key function in crates/events/src/scheduler.rs explaining it makes a string safe as a NATS KV key

- model: unknown
- files: crates/events/src/scheduler.rs
- when: 2026-06-05T17:39:24.434373+00:00

## Agent output

```

```

## Diff

```diff
--- a/crates/events/src/scheduler.rs
+++ b/crates/events/src/scheduler.rs
@@ -307,6 +307,10 @@
 }
 
 /// Sanitize a key for NATS KV — replace colons with dots, remove invalid chars.
+/// Makes a string safe as a NATS KV key.
+///
+/// This function ensures that the input string can be used as a valid key in a NATS KeyValue (KV)
+/// store by replacing any characters that are not allowed in KV keys with underscores (`_`).
 fn sanitize_key(s: &str) -> String {
     s.chars().map(|c| match c {
         ':' => '.',

```
