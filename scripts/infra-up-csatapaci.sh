#!/usr/bin/env bash
set -euo pipefail

# Start alpha-swarm infrastructure on CSATAPACI via SSH.
# Run this from the local machine.
# Usage: ./scripts/infra-up-csatapaci.sh

REMOTE="csatapaci"
DATA_DIR="/tmp/alpha-swarm"

echo "=== alpha-swarm infrastructure (csatapaci) ==="

# --- Verify SSH connectivity, then get local machine's Tailscale IP ---
ssh -o ConnectTimeout=10 "$REMOTE" "true" || { echo "ERROR: cannot SSH to $REMOTE"; exit 1; }
LOCAL_IP=$(tailscale ip -4 2>/dev/null | head -1 || echo "UNKNOWN")
echo "  Local Tailscale IP: $LOCAL_IP"

# --- Copy NATS config ---
echo "[0/4] Preparing NATS config..."
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_DIR="$(dirname "$SCRIPT_DIR")"

# Create config with local IP substituted
sed "s|LOCALIP|$LOCAL_IP|g" "$PROJECT_DIR/infra/nats-csatapaci.conf" | \
    ssh "$REMOTE" "mkdir -p $DATA_DIR && cat > $DATA_DIR/nats.conf"

# --- NATS ---
echo "[1/4] Starting NATS server..."
ssh "$REMOTE" "mkdir -p $DATA_DIR/nats/jetstream && nohup nats-server -c $DATA_DIR/nats.conf > $DATA_DIR/nats.log 2>&1 &"
sleep 1
ssh "$REMOTE" "pgrep -f 'nats-server.*nats.conf' > /dev/null && echo '  NATS running' || echo '  ERROR: NATS failed'"

# NOTE: SurrealDB no longer runs on csatapaci — the local agent-daemon embeds
# the database (kv-surrealkv) and serves all consumers via the NATS DB bridge
# (swarm.db.>). csatapaci is inference-only: NATS + Ollama + wash.

# --- Ollama ---
# csatapaci's login shell is fish — run remote snippets under bash explicitly.
echo "[2/3] Ensuring Ollama is running..."
ssh "$REMOTE" bash -s <<EOF
if pgrep -f 'ollama serve' > /dev/null 2>&1; then
    echo '  Ollama already running'
else
    nohup ollama serve > $DATA_DIR/ollama.log 2>&1 &
    sleep 2
    echo '  Ollama started'
fi
EOF

# --- WasmCloud Host ---
echo "[3/3] Starting WasmCloud host..."
ssh "$REMOTE" bash -s <<EOF
if pgrep -f 'wash host' > /dev/null 2>&1; then
    echo '  WasmCloud already running'
else
    nohup wash host --scheduler-nats-url nats://127.0.0.1:4222 --data-nats-url nats://127.0.0.1:4222 --host-name alpha-swarm-csatapaci --host-group alpha-swarm --non-interactive > $DATA_DIR/wasmcloud.log 2>&1 &
    sleep 2
fi
pgrep -f 'wash host' > /dev/null && echo '  WasmCloud running' || echo '  ERROR: WasmCloud failed'
EOF

echo ""
echo "=== csatapaci infrastructure running ==="
echo "  NATS:      nats://csatapaci:4222"
echo "  Ollama:    http://csatapaci:11434"
echo "  WasmCloud: running (wash host)"
echo "  (DB is embedded in the local agent-daemon; bridge: swarm.db.>)"
echo ""
echo "  Stop with: ./scripts/infra-down-csatapaci.sh"
