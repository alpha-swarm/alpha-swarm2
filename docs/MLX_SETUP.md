# Local inference: MLX (chat) + rpi Ollama (embeddings)

Replaces the single wedge-prone Ollama on csatapaci with a clean split:

```
picur (Mac)         daemon + NATS + surrealdb        (orchestration)
csatapaci (M2 96G)  MLX — one server per model:      (chat)
                      qwen2.5-coder-14b  :8101
                      qwen2.5-coder-32b  :8102
malna (rpi5 8G)     Ollama — nomic-embed-text :11434 (embeddings)  ✅ already live
```

**Why**: one process per model = no model swapping, no cross-model deadlock (the
exact failure that wedged Ollama all session). Embeddings on a tiny dedicated
host never contend with chat. MLX is the fastest engine on Apple Silicon.

`malna` is already set up: Ollama runs as a systemd service (`OLLAMA_HOST=0.0.0.0`,
auto-restart, survives reboot) serving `nomic-embed-text` (768-dim). Nothing more
to do there.

## Activate (after rebooting csatapaci to clear the wedged Ollama)

1. **csatapaci** — install MLX + start the chat servers:
   ```sh
   pip install --upgrade mlx-lm        # one-time
   bash scripts/run-mlx.sh             # first run downloads 8-bit models (~16GB+34GB)
   # health: curl localhost:8101/v1/models ; curl localhost:8102/v1/models
   ```
   Optionally stop the old Ollama on csatapaci entirely (`brew services stop ollama`
   + quit the desktop app) — it's no longer used for anything.

2. **picur** — point the daemon at MLX (chat) + malna (embeddings). In
   `alpha-swarm.toml`:
   ```toml
   [ollama]
   url = "http://malna:11434"          # embeddings → rpi
   keep_alive = "-1"

   [defaults]
   embed_model = "nomic-embed-text"

   [tiers.orchestrator]
   model = "mlx-community/Qwen2.5-Coder-14B-Instruct-8bit"
   # ...keep the rest of the tier fields...
   [tiers.agent]
   model = "mlx-community/Qwen2.5-Coder-32B-Instruct-8bit"
   [tiers.worker]
   model = "mlx-community/Qwen2.5-Coder-14B-Instruct-8bit"

   # chat → csatapaci MLX, one provider per model (router routes preferred_model
   # to the backend that hosts it — see PR #63)
   [[providers]]
   type = "openai-compat"
   url = "http://100.81.10.8:8101"     # csatapaci, 14b
   model = "mlx-community/Qwen2.5-Coder-14B-Instruct-8bit"
   priority = 1
   [[providers]]
   type = "openai-compat"
   url = "http://100.81.10.8:8102"     # csatapaci, 32b
   model = "mlx-community/Qwen2.5-Coder-32B-Instruct-8bit"
   priority = 1
   ```
   Remove the old `type = "ollama"` provider (chat no longer uses Ollama). Then
   restart the daemon (`launchctl kickstart -k gui/501/io.alphaswarm.agent-daemon`).

3. Verify: submit a goal; planner runs on `:8101` (14b), refactors escalate to
   `:8102` (32b), embeddings hit malna. No model swapping anywhere.

## Prompt / KV prefix cache (free, already on)
`mlx_lm.server` keeps an automatic LRU prompt cache and reuses the longest token
prefix **across separate `/v1/chat/completions` calls** — no API field or flag
to enable it. The agent tool loop already builds a stable, append-only prompt
(system + read-only `FILES:` context first, only new tool-result turns appended),
so step 2+ of every run skips re-prefilling that prefix. Two operational notes:

- **Bound the RAM.** The server only trims the cache when `--prompt-cache-bytes`
  is set (default is effectively unbounded); a 32B model's KV can otherwise grow
  without limit. `scripts/run-mlx.sh` now passes `--prompt-cache-bytes`
  (`MLX_PROMPT_CACHE_BYTES`, default 6 GiB).
- **Observe hits.** The daemon now reads `usage.cached_tokens` from each MLX
  response into `InferenceResponse.cached_tokens` and logs a `prompt-cache reuse`
  line (`hit_pct`) at debug. A low/zero hit ratio means something is busting the
  prefix — never inject timestamps/uuids/run-ids into the system or first user
  message.
- **Caveat:** caching only removes *prefill*. Long runs are *decode*-bound;
  `max_tokens` is now capped (`MAX_OUTPUT_TOKENS`, agent-core) to bound runaway
  generation, and speculative decoding (`--draft-model`) is the bigger decode-side
  lever if needed.

## Known rough edge (harmless)
The model warmer is Ollama-specific; with chat on MLX it will log a few "warm
ping failed" warnings (it tries to warm the MLX model names against malna's
nomic-only Ollama). Harmless — MLX keeps its single model resident always, so no
warming is needed. Follow-up: skip the warmer when chat isn't on the Ollama
backend. (Addressed separately: the warmer now skips absolute-path MLX models.)
