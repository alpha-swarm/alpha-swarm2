#!/usr/bin/env bash
set -euo pipefail

# Stop alpha-swarm infrastructure on LOCAL machine.

DATA_DIR="/tmp/alpha-swarm"
PID_FILE="$DATA_DIR/pids"

echo "=== Stopping alpha-swarm infrastructure (local) ==="

if [ -f "$PID_FILE" ]; then
    source "$PID_FILE"

    for name in WASMCLOUD SURREAL NATS; do
        pid_var="${name}_PID"
        pid="${!pid_var:-}"
        if [ -n "$pid" ] && kill -0 "$pid" 2>/dev/null; then
            echo "  Stopping $name (PID $pid)..."
            kill "$pid" 2>/dev/null
            wait "$pid" 2>/dev/null || true
        else
            echo "  $name not running"
        fi
    done

    rm -f "$PID_FILE"
else
    echo "  No PID file found. Killing by process name..."
    pkill -f "wasmcloud" 2>/dev/null || true
    pkill -f "surreal start" 2>/dev/null || true
    pkill -f "nats-server.*nats-local.conf" 2>/dev/null || true
fi

echo "=== Done ==="
