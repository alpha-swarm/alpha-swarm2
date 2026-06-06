#!/usr/bin/env bash
# Run ON csatapaci (M2 Max, Apple Silicon). Starts one MLX server PER MODEL on
# its own port — OpenAI-compatible (/v1/chat/completions). One process per model
# means NO model swapping and NO cross-model deadlock (the failure mode that
# wedged Ollama). Embeddings live on malna (rpi); these servers are chat only.
#
# One-time install:   pip install --upgrade mlx-lm     (or: uv pip install mlx-lm)
# First run downloads the 8-bit models from Hugging Face (~16GB + ~34GB).
#
# Resident set: 14b-8bit (~16GB) + 32b-8bit (~34GB) = ~50GB of 96GB — comfortable.
set -euo pipefail

M14="${MLX_MODEL_14B:-mlx-community/Qwen2.5-Coder-14B-Instruct-8bit}"
M32="${MLX_MODEL_32B:-mlx-community/Qwen2.5-Coder-32B-Instruct-8bit}"
PORT_14B="${MLX_PORT_14B:-8101}"
PORT_32B="${MLX_PORT_32B:-8102}"
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
    nohup mlx_lm.server --model "$1" --host 0.0.0.0 --port "$2" > "$LOG/$3.log" 2>&1 &
    echo "started $3 ($1) on :$2 — log $LOG/$3.log"
  fi
}

start "$M14" "$PORT_14B" mlx-14b
start "$M32" "$PORT_32B" mlx-32b
echo "MLX chat servers launching (first run pulls models). Health: curl localhost:$PORT_14B/v1/models"
