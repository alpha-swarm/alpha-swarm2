#!/usr/bin/env bash
set -euo pipefail

# Deploy alpha-swarm to wasmCloud via wadm (standalone).
#
# Prerequisites:
#   - NATS running (nats://127.0.0.1:4222)
#   - wash host running (wash host --scheduler-nats-url ...)
#   - WASI components built (./scripts/build-components.sh)
#
# Usage: ./scripts/deploy.sh [undeploy]

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_DIR="$(dirname "$SCRIPT_DIR")"
NATS_URL="${NATS_URL:-nats://127.0.0.1:4222}"
LATTICE="${LATTICE:-default}"
APP_NAME="alpha-swarm"
WADM_BIN="${WADM_BIN:-wadm}"

# --- Check prerequisites ---
check_prereqs() {
    if ! command -v nats &>/dev/null; then
        echo "ERROR: nats CLI not found. Install: brew install nats-io/nats-tools/nats"
        exit 1
    fi

    if ! nats server check connection --server "$NATS_URL" &>/dev/null; then
        echo "ERROR: Cannot connect to NATS at $NATS_URL"
        echo "  Start with: ./scripts/infra-up-local.sh"
        exit 1
    fi
}

# --- Install wadm if not present ---
ensure_wadm() {
    if command -v "$WADM_BIN" &>/dev/null; then
        echo "wadm found: $($WADM_BIN --version 2>/dev/null || echo 'unknown version')"
        return
    fi

    echo "wadm not found. Installing via cargo..."
    cargo install wadm
    WADM_BIN="wadm"
}

# --- Start wadm if not running ---
start_wadm() {
    if pgrep -f "wadm.*nats" > /dev/null 2>&1; then
        echo "wadm already running"
        return
    fi

    echo "Starting wadm..."
    nohup "$WADM_BIN" \
        --nats-server "$NATS_URL" \
        > /tmp/alpha-swarm/wadm.log 2>&1 &
    WADM_PID=$!
    echo "  wadm PID: $WADM_PID"
    sleep 2

    if ! kill -0 "$WADM_PID" 2>/dev/null; then
        echo "ERROR: wadm failed to start. Check /tmp/alpha-swarm/wadm.log"
        exit 1
    fi
}

# --- Deploy ---
deploy() {
    echo "Deploying $APP_NAME..."

    # Put the manifest
    echo "  Storing manifest..."
    nats req "wadm.api.$LATTICE.model.put" \
        --server "$NATS_URL" \
        "$(cat "$PROJECT_DIR/wadm.yaml")" \
        --timeout 10s

    echo ""

    # Deploy
    echo "  Deploying..."
    nats req "wadm.api.$LATTICE.model.deploy.$APP_NAME" \
        --server "$NATS_URL" \
        '{"version": "v0.1.0"}' \
        --timeout 10s

    echo ""
    echo "=== Deployed ==="
    echo "  Check status: nats req 'wadm.api.$LATTICE.model.status.$APP_NAME' '' --server $NATS_URL"
}

# --- Undeploy ---
undeploy() {
    echo "Undeploying $APP_NAME..."
    nats req "wadm.api.$LATTICE.model.undeploy.$APP_NAME" \
        --server "$NATS_URL" \
        '{}' \
        --timeout 10s || true
    echo "  Done"
}

# --- Status ---
status() {
    echo "Checking $APP_NAME status..."
    nats req "wadm.api.$LATTICE.model.status.$APP_NAME" \
        --server "$NATS_URL" \
        '' \
        --timeout 5s 2>/dev/null || echo "  No deployment found"
}

# --- Main ---
mkdir -p /tmp/alpha-swarm

case "${1:-deploy}" in
    undeploy)
        check_prereqs
        undeploy
        ;;
    status)
        check_prereqs
        status
        ;;
    deploy|*)
        check_prereqs
        ensure_wadm
        start_wadm
        deploy
        status
        ;;
esac
