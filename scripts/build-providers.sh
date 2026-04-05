#!/usr/bin/env bash
set -euo pipefail

# Build all providers and create .par.gz archives.
# Usage: ./scripts/build-providers.sh

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_DIR="$(dirname "$SCRIPT_DIR")"

cd "$PROJECT_DIR"

echo "=== Building providers ==="

cargo build --release \
    -p ollama-provider \
    -p virtfs-provider \
    -p quality-gate-provider

mkdir -p build

for provider in ollama-provider virtfs-provider quality-gate-provider; do
    echo "  Packaging $provider..."
    wash par create \
        --name "$provider" \
        --vendor alpha-swarm \
        --binary "target/release/$provider" \
        --destination "build/$provider.par.gz" \
        --compress \
        --version 0.1.0
done

echo ""
echo "=== Provider archives ==="
ls -lh build/*.par.gz

echo ""
echo "=== Done ==="
