# Refactor crates/inference-client/src/ollama.rs: extract the num_ctx bucketing math in sized_num_ctx into a small pure helper fn bucket_ctx(want: u32, ceiling: u32) -> u32 with the same behavior

- model: unknown
- files: crates/inference-client/src/ollama.rs
- when: 2026-06-05T21:10:17.250819+00:00

## Agent output

```

```

## Diff

```diff
--- a/crates/inference-client/src/ollama.rs
+++ b/crates/inference-client/src/ollama.rs
@@ -32,9 +32,7 @@
 /// Number of characters per token used for estimating the number of context tokens.
 const NUM_CTX_CHARS_PER_TOKEN: usize = 3;
 
-/// Size `num_ctx` from the prompt's char length, clamped to `[NUM_CTX_FLOOR,
-/// ceiling]`. `ceiling` is the tier's max_tokens (None = let Ollama default).
-fn sized_num_ctx(prompt_chars: usize, ceiling: Option<u32>) -> Option<u32> {
+fn bucket_ctx(prompt_chars: usize, ceiling: Option<u32>) -> Option<u32> {
     ceiling.map(|c| {
         let est_prompt_tokens = (prompt_chars / NUM_CTX_CHARS_PER_TOKEN) as u32;
         let lo = NUM_CTX_FLOOR.min(c);
@@ -42,6 +40,12 @@
     })
 }
 
+/// Size `num_ctx` from the prompt's char length, clamped to `[NUM_CTX_FLOOR,
+/// ceiling]`. `ceiling` is the tier's max_tokens (None = let Ollama default).
+fn sized_num_ctx(prompt_chars: usize, ceiling: Option<u32>) -> Option<u32> {
+    bucket_ctx(prompt_chars, ceiling)
+}
+
 pub struct OllamaBackend {
     client: Client,
     base_url: String,

```
