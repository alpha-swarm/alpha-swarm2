# Add a Rust doc comment to the NUM_CTX_CHARS_PER_TOKEN constant in crates/inference-client/src/ollama.rs

- model: unknown
- files: crates/inference-client/src/ollama.rs
- when: 2026-06-05T15:18:10.720968+00:00

## Agent output

```

```

## Diff

```diff
--- a/crates/inference-client/src/ollama.rs
+++ b/crates/inference-client/src/ollama.rs
@@ -29,6 +29,7 @@
 /// above what the caller already asked for.
 const NUM_CTX_FLOOR: u32 = 4096;
 const NUM_CTX_OUTPUT_HEADROOM: u32 = 4096;
+/// Number of characters per token used for estimating the number of context tokens.
 const NUM_CTX_CHARS_PER_TOKEN: usize = 3;
 
 /// Size `num_ctx` from the prompt's char length, clamped to `[NUM_CTX_FLOOR,

```
