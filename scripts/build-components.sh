#!/usr/bin/env bash
set -euo pipefail

# Build all WASI components for wasmCloud deployment.
# Usage: ./scripts/build-components.sh

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_DIR="$(dirname "$SCRIPT_DIR")"

echo "=== Building WASI components ==="

COMPONENTS=(
    "web-ui"
    "agent-worker"
    "tool-search"
    "tool-web"
    "mcp-server"
)

for comp in "${COMPONENTS[@]}"; do
    echo "[$comp] Building..."
    (cd "$PROJECT_DIR/components/$comp" && wash build 2>&1) || {
        echo "  WARN: wash build failed for $comp, trying cargo..."
        cargo build --release --target wasm32-wasip2 -p "$comp" 2>&1
    }
    echo "  OK"
done

echo ""
echo "=== Build complete ==="
echo "Artifacts in: $PROJECT_DIR/target/wasm32-wasip2/release/"
ls -lh "$PROJECT_DIR/target/wasm32-wasip2/release/"*.wasm 2>/dev/null || echo "  (no .wasm files found)"
