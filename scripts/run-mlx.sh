#!/usr/bin/env bash
# Run ON csatapaci (M2 Max, Apple Silicon). Starts one MLX server PER MODEL on
# its own port — OpenAI-compatible (/v1/chat/completions). One process per model
# means NO model swapping and NO cross-model deadlock (the failure mode that
# wedged Ollama). Embeddings live on malna (rpi); these servers are chat only.
#
# One-time install:   pip install --upgrade mlx-lm     (or: uv pip install mlx-lm)
#
# Serves the PRE-DOWNLOADED local model dirs (~/mlx-models/qwen{14b,32b}-4bit) with
# HF_HUB_OFFLINE=1. This is critical: mlx_lm.server's /v1/models advertises the
# canonical HF repo-id alongside the local path, so a request carrying that repo-id
# makes mlx_lm try to LOAD it from Hugging Face — an unauthenticated, rate-limited
# re-download that hangs the server ~17min and wedges every run. Offline mode makes
# such a request fail fast instead. To download models the FIRST time, run with
# MLX_ALLOW_DOWNLOAD=1 and point MLX_MODEL_* at a HF repo name.
#
# Resident set: 14b-4bit (~8GB) + 32b-4bit (~17GB) = ~25GB of 96GB — comfortable.
set -euo pipefail

M14="${MLX_MODEL_14B:-$HOME/mlx-models/qwen14b-4bit}"
M32="${MLX_MODEL_32B:-$HOME/mlx-models/qwen32b-4bit}"
PORT_14B="${MLX_PORT_14B:-8101}"
PORT_32B="${MLX_PORT_32B:-8102}"
# Serve offline by default (see header). MLX_ALLOW_DOWNLOAD=1 lets the first-run
# download through (then drop it on subsequent runs).
OFFLINE_ENV="HF_HUB_OFFLINE=1"
[ "${MLX_ALLOW_DOWNLOAD:-0}" = "1" ] && OFFLINE_ENV=""
# Bound the prompt/KV prefix cache RAM. mlx_lm.server keeps an automatic LRU
# prompt cache (reused across /v1/chat/completions calls — this is what makes
# the agent tool loop skip re-prefilling its stable prefix), but it only trims
# memory when --prompt-cache-bytes is set; the default is effectively unbounded,
# so a 32B model's KV can grow without limit. Cap it (bytes; default 6 GiB).
CACHE_BYTES="${MLX_PROMPT_CACHE_BYTES:-6442450944}"
LOG=/tmp/mlx-logs
mkdir -p "$LOG"

if ! command -v mlx_lm.server >/dev/null 2>&1; then
  echo "mlx_lm not found — install with:  pip install --upgrade mlx-lm" >&2
  exit 1
fi

start() { # model port logname
  if lsof -nP -iTCP:"$2" -sTCP:LISTEN >/dev/null 2>&1; then
    echo "port $2 already in use — skipping $3"
  else
    nohup env $OFFLINE_ENV mlx_lm.server --model "$1" --host 0.0.0.0 --port "$2" \
      --prompt-cache-bytes "$CACHE_BYTES" > "$LOG/$3.log" 2>&1 &
    echo "started $3 ($1) on :$2 — log $LOG/$3.log ${OFFLINE_ENV:+(offline)}"
  fi
}

start "$M14" "$PORT_14B" mlx-14b
start "$M32" "$PORT_32B" mlx-32b
echo "MLX chat servers launching${OFFLINE_ENV:+ (HF offline)}. Health: curl localhost:$PORT_14B/v1/models"
