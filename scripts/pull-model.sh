#!/usr/bin/env bash
set -euo pipefail

# Pull an Ollama model with auto-restart on stall.
# Usage: ./scripts/pull-model.sh <model> [host]
# Example: ./scripts/pull-model.sh qwen3:72b csatapaci

MODEL="${1:?Usage: pull-model.sh <model> [host]}"
HOST="${2:-csatapaci}"
CHECK_INTERVAL=120  # 2 minutes
STALL_THRESHOLD=1048576  # 1MB — if less than this transferred in 2 min, restart
MAX_RESTARTS=20

echo "=== Pulling $MODEL on $HOST ==="
echo "  Check interval: ${CHECK_INTERVAL}s"
echo "  Stall threshold: $((STALL_THRESHOLD / 1024))KB in ${CHECK_INTERVAL}s"
echo ""

restart_count=0
last_size=0

pull_model() {
    echo "[$(date +%H:%M:%S)] Starting ollama pull $MODEL (attempt $((restart_count + 1)))"
    ssh "$HOST" "nohup ollama pull $MODEL > /tmp/ollama-pull.log 2>&1 &"
    sleep 5
}

get_pull_progress() {
    # Check the latest blob being downloaded
    ssh "$HOST" "tail -1 /tmp/ollama-pull.log 2>/dev/null" || echo ""
}

is_pull_running() {
    ssh "$HOST" "pgrep -f 'ollama pull' > /dev/null 2>&1" && return 0 || return 1
}

get_blob_size() {
    # Get total size of blobs directory to detect progress
    ssh "$HOST" "du -sb ~/.ollama/models/ 2>/dev/null | cut -f1" || echo "0"
}

check_model_exists() {
    ssh "$HOST" "ollama list 2>/dev/null | grep -q '$MODEL'" && return 0 || return 1
}

# Start the pull
pull_model

while true; do
    sleep "$CHECK_INTERVAL"

    # Check if model is already fully downloaded
    if check_model_exists; then
        echo ""
        echo "[$(date +%H:%M:%S)] Model $MODEL is ready!"
        ssh "$HOST" "ollama list | grep '$MODEL'"
        echo "=== Done (${restart_count} restarts) ==="
        exit 0
    fi

    # Check if pull process is still running
    if ! is_pull_running; then
        echo "[$(date +%H:%M:%S)] Pull process died. Checking if complete..."
        if check_model_exists; then
            echo "Model $MODEL is ready!"
            exit 0
        fi

        restart_count=$((restart_count + 1))
        if [ "$restart_count" -ge "$MAX_RESTARTS" ]; then
            echo "ERROR: Max restarts ($MAX_RESTARTS) reached. Giving up."
            exit 1
        fi

        echo "[$(date +%H:%M:%S)] Restarting pull..."
        pull_model
        last_size=0
        continue
    fi

    # Check progress by blob directory size
    current_size=$(get_blob_size)
    delta=$((current_size - last_size))
    progress=$(get_pull_progress)

    echo "[$(date +%H:%M:%S)] Progress: +$((delta / 1024 / 1024))MB (total: $((current_size / 1024 / 1024))MB) | $progress"

    if [ "$delta" -lt "$STALL_THRESHOLD" ] && [ "$last_size" -gt 0 ]; then
        echo "[$(date +%H:%M:%S)] STALL DETECTED (<$((STALL_THRESHOLD / 1024))KB in ${CHECK_INTERVAL}s). Restarting..."

        # Kill the stalled pull
        ssh "$HOST" "pkill -f 'ollama pull' 2>/dev/null" || true
        sleep 3

        restart_count=$((restart_count + 1))
        if [ "$restart_count" -ge "$MAX_RESTARTS" ]; then
            echo "ERROR: Max restarts ($MAX_RESTARTS) reached. Giving up."
            exit 1
        fi

        pull_model
        last_size=0
        continue
    fi

    last_size=$current_size
done
