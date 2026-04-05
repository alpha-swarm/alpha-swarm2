#!/usr/bin/env bash
set -euo pipefail

# Start alpha-swarm infrastructure on LOCAL machine.
# Usage: ./scripts/infra-up-local.sh

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_DIR="$(dirname "$SCRIPT_DIR")"
DATA_DIR="/tmp/alpha-swarm"

echo "=== alpha-swarm infrastructure (local) ==="

# --- NATS ---
echo "[1/3] Starting NATS server..."
mkdir -p "$DATA_DIR/nats/jetstream"
nats-server -c "$PROJECT_DIR/infra/nats-local.conf" &
NATS_PID=$!
echo "  NATS PID: $NATS_PID"
sleep 1

if ! kill -0 "$NATS_PID" 2>/dev/null; then
    echo "  ERROR: NATS failed to start"
    exit 1
fi
echo "  NATS listening on :4222, cluster on :6222, monitoring on :8222"

# --- SurrealDB ---
echo "[2/3] Starting SurrealDB..."
mkdir -p "$DATA_DIR/surrealdb"
surreal start \
    --bind 0.0.0.0:8000 \
    --user root --pass root \
    "file://$DATA_DIR/surrealdb/data.db" &
SURREAL_PID=$!
echo "  SurrealDB PID: $SURREAL_PID"
sleep 2

if ! kill -0 "$SURREAL_PID" 2>/dev/null; then
    echo "  ERROR: SurrealDB failed to start"
    kill "$NATS_PID" 2>/dev/null
    exit 1
fi
echo "  SurrealDB listening on :8000"

# --- WasmCloud Host ---
echo "[3/3] Starting WasmCloud host..."
wasmcloud \
    --nats-host 127.0.0.1 \
    --nats-port 4222 \
    --label alpha-swarm-role=orchestrator \
    --label alpha-swarm-gpu=false &
WASMCLOUD_PID=$!
echo "  WasmCloud PID: $WASMCLOUD_PID"
sleep 2

if ! kill -0 "$WASMCLOUD_PID" 2>/dev/null; then
    echo "  ERROR: WasmCloud failed to start"
    kill "$NATS_PID" "$SURREAL_PID" 2>/dev/null
    exit 1
fi

# --- Write PID file ---
cat > "$DATA_DIR/pids" <<EOF
NATS_PID=$NATS_PID
SURREAL_PID=$SURREAL_PID
WASMCLOUD_PID=$WASMCLOUD_PID
EOF

echo ""
echo "=== Local infrastructure running ==="
echo "  NATS:      nats://127.0.0.1:4222"
echo "  SurrealDB: ws://127.0.0.1:8000"
echo "  WasmCloud: wash get hosts"
echo ""
echo "  Stop with: ./scripts/infra-down-local.sh"

# Wait for all background processes
wait
