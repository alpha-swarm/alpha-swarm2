#!/usr/bin/env bash
set -euo pipefail

# Stop alpha-swarm infrastructure on CSATAPACI via SSH.

REMOTE="csatapaci"

echo "=== Stopping alpha-swarm infrastructure (csatapaci) ==="

ssh "$REMOTE" bash <<'EOF'
echo "  Stopping WasmCloud..."
pkill -f "wasmcloud" 2>/dev/null || echo "    not running"

echo "  Stopping SurrealDB..."
pkill -f "surreal start" 2>/dev/null || echo "    not running"

echo "  Stopping NATS..."
pkill -f "nats-server" 2>/dev/null || echo "    not running"

echo "  (Leaving Ollama running)"
EOF

echo "=== Done ==="
