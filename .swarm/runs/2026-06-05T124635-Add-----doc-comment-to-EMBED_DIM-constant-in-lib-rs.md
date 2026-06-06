# Add /// doc comment to EMBED_DIM constant in lib.rs

- model: unknown
- files: crates/inference-client/src/lib.rs
- when: 2026-06-05T12:46:35.422487+00:00

## Agent output

```

```

## Diff

```diff
--- a/crates/inference-client/src/lib.rs
+++ b/crates/inference-client/src/lib.rs
@@ -13,6 +13,9 @@
 #[cfg(feature = "native")]
 pub mod mock;
 
+/// Dimensionality of the embedding vector
+pub const EMBED_DIM: usize = 768;
+
 pub use types::*;
 pub use backend::InferenceBackend;
 

```
