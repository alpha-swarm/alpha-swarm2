#!/usr/bin/env bash
# Resilient `ollama pull` on csatapaci over SSH.
#
# Why not aria2: Ollama models aren't plain HTTP files — they're fetched via
# Ollama's own registry/blob protocol, so aria2 can't pull them. But `ollama
# pull` is itself resumable (re-running continues a partial download), so the
# resilient recipe is: run it DETACHED (survives the SSH session dropping),
# keep the Mac AWAKE for the duration, and RETRY until it succeeds.
#
#   detached  -> tmux session on csatapaci (persists after SSH disconnect)
#   awake     -> caffeinate -i (no idle sleep mid-pull)
#   retried   -> until-loop around `ollama pull` (resumes partial blobs)
#
# Usage:  scripts/pull-models-csatapaci.sh qwen2.5-coder:32b llama3.3:70b
# Host override:  CSATAPACI_SSH=user@100.81.10.8 scripts/pull-models-csatapaci.sh ...
# Monitor:  ssh csatapaci 'tmux ls'
#           ssh csatapaci 'tmux capture-pane -pt <session>'
#           ssh csatapaci 'ollama list'
set -euo pipefail

HOST="${CSATAPACI_SSH:-csatapaci}"
if [ "$#" -eq 0 ]; then
  echo "usage: $0 <model>...   e.g. $0 qwen2.5-coder:32b" >&2
  exit 1
fi

# Install the remote runner (plain file → no quoting hell). $1 = model.
ssh "$HOST" 'cat > /tmp/ollama-resilient-pull.sh && chmod +x /tmp/ollama-resilient-pull.sh' <<'REMOTE'
#!/usr/bin/env bash
set -u
m="$1"
echo "=== pull '$m' started $(date) ==="
until ollama pull "$m"; do
  echo "[retry $(date)] pull interrupted; resuming in 15s..."
  sleep 15
done
echo "=== PULL_COMPLETE '$m' $(date) ==="
REMOTE

for m in "$@"; do
  session="pull_$(printf '%s' "$m" | tr -c 'a-zA-Z0-9' '_')"
  if ssh "$HOST" "tmux has-session -t '$session' 2>/dev/null"; then
    echo ">> '$m': session '$session' already running — skipping"
    continue
  fi
  ssh "$HOST" "tmux new-session -d -s '$session' \"caffeinate -i /tmp/ollama-resilient-pull.sh '$m'\""
  echo ">> '$m': launched in tmux session '$session' on $HOST"
done

echo
echo "monitor: ssh $HOST 'tmux ls'"
echo "         ssh $HOST 'tmux capture-pane -pt <session>'"
echo "verify : ssh $HOST 'ollama list'"
