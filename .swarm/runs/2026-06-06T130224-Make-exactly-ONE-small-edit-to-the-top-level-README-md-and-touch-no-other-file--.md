# Make exactly ONE small edit to the top-level README.md and touch no other file. In the Inference section, replace the bullet line that reads "- Models: qwen2.5:72b (planning), deepseek-coder:33b (code gen), qwen2.5-coder:7b (fast tasks)" with two bullet lines: "- Chat: MLX (mlx_lm.server) on csatapaci — qwen2.5-coder 14B (planner/worker) + 32B (refactor escalation)" and "- Embeddings: malna (RPi 5) via Ollama nomic-embed-text (768-dim)". Do not edit any Rust file or any file other than README.md.

- model: direct
- files: README.md
- when: 2026-06-06T13:02:24.329010+00:00

## Agent output

```
Direct edit applied
```

## Diff

```diff
--- a/README.md
+++ b/README.md
@@ -22,7 +22,8 @@
 
 ### Inference
 - **inference-client** — Multi-backend router (Ollama + Claude). Model selection by complexity tier. Native tool calling support.
-- Models: qwen2.5:72b (planning), deepseek-coder:33b (code gen), qwen2.5-coder:7b (fast tasks)
+- Chat: MLX (mlx_lm.server) on csatapaci — qwen2.5-coder 14B (planner/worker) + 32B (refactor escalation)
+- Embeddings: malna (RPi 5) via Ollama nomic-embed-text (768-dim)
 
 ### Tools (15 built-in)
 - **Filesystem**: read_file, write_file, delete_file, list_files

```
