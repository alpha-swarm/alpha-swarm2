#!/usr/bin/env bash
set -euo pipefail

# Start alpha-swarm infrastructure on CSATAPACI via SSH.
# Run this from the local machine.
# Usage: ./scripts/infra-up-csatapaci.sh

REMOTE="csatapaci"
DATA_DIR="/tmp/alpha-swarm"

echo "=== alpha-swarm infrastructure (csatapaci) ==="

# --- Get local machine's Tailscale IP ---
LOCAL_IP=$(ssh "$REMOTE" "echo dummy" 2>/dev/null; tailscale ip -4 2>/dev/null || echo "UNKNOWN")
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

# --- SurrealDB ---
echo "[2/4] Starting SurrealDB..."
ssh "$REMOTE" "mkdir -p $DATA_DIR/surrealdb && nohup surreal start --bind 0.0.0.0:8000 --user root --pass root 'file://$DATA_DIR/surrealdb/data.db' > $DATA_DIR/surreal.log 2>&1 &"
sleep 2
ssh "$REMOTE" "pgrep -f 'surreal start' > /dev/null && echo '  SurrealDB running' || echo '  ERROR: SurrealDB failed'"

# --- Ollama ---
echo "[3/4] Ensuring Ollama is running..."
ssh "$REMOTE" "pgrep -f 'ollama serve' > /dev/null 2>&1 && echo '  Ollama already running' || (nohup ollama serve > $DATA_DIR/ollama.log 2>&1 & sleep 2 && echo '  Ollama started')"

# --- WasmCloud Host ---
echo "[4/4] Starting WasmCloud host..."
ssh "$REMOTE" "nohup wash host --scheduler-nats-url nats://127.0.0.1:4222 --data-nats-url nats://127.0.0.1:4222 --host-name alpha-swarm-csatapaci --host-group alpha-swarm --non-interactive > $DATA_DIR/wasmcloud.log 2>&1 &"
sleep 2
ssh "$REMOTE" "pgrep -f 'wash host' > /dev/null && echo '  WasmCloud running' || echo '  ERROR: WasmCloud failed'"

echo ""
echo "=== csatapaci infrastructure running ==="
echo "  NATS:      nats://csatapaci:4222"
echo "  SurrealDB: ws://csatapaci:8000"
echo "  Ollama:    http://csatapaci:11434"
echo "  WasmCloud: running (wash host)"
echo ""
echo "  Stop with: ./scripts/infra-down-csatapaci.sh"
