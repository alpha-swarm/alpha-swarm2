#!/usr/bin/env bash
set -euo pipefail

# Check alpha-swarm infrastructure status on both machines.

echo "=== LOCAL ==="
echo -n "  NATS:      "; pgrep -f "nats-server" > /dev/null 2>&1 && echo "running" || echo "stopped"
echo -n "  SurrealDB: "; pgrep -f "surreal start" > /dev/null 2>&1 && echo "running" || echo "stopped"
echo -n "  WasmCloud: "; pgrep -f "wash host" > /dev/null 2>&1 && echo "running" || echo "stopped"
echo -n "  wadm:      "; pgrep -f "wadm" > /dev/null 2>&1 && echo "running" || echo "stopped"
echo -n "  Ollama:    "; pgrep -f "ollama" > /dev/null 2>&1 && echo "running" || echo "not running (ok — csatapaci handles inference)"

echo ""
echo "=== CSATAPACI ==="
ssh csatapaci bash <<'EOF' 2>/dev/null || echo "  SSH connection failed"
echo -n "  NATS:      "; pgrep -f "nats-server" > /dev/null 2>&1 && echo "running" || echo "stopped"
echo -n "  SurrealDB: "; pgrep -f "surreal start" > /dev/null 2>&1 && echo "running" || echo "stopped"
echo -n "  WasmCloud: "; pgrep -f "wash host" > /dev/null 2>&1 && echo "running" || echo "stopped"
echo -n "  wadm:      "; pgrep -f "wadm" > /dev/null 2>&1 && echo "running" || echo "stopped"
echo -n "  Ollama:    "; pgrep -f "ollama" > /dev/null 2>&1 && echo "running" || echo "stopped"
if pgrep -f "ollama" > /dev/null 2>&1; then
    echo "  Models:    $(curl -s http://localhost:11434/api/tags 2>/dev/null | python3 -c 'import sys,json; [print("             " + m["name"]) for m in json.load(sys.stdin).get("models",[])]' 2>/dev/null || echo "could not query")"
fi
EOF

echo ""
echo "=== DEPLOYMENT ==="
nats req "wadm.api.default.model.status.alpha-swarm" '' --server nats://127.0.0.1:4222 --timeout 3s 2>/dev/null || echo "  No wadm deployment found (is wadm running?)"
