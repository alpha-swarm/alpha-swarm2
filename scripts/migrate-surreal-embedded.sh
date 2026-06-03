#!/usr/bin/env bash
# Migrate data from the external SurrealDB server into the embedded surrealkv
# data dir owned by agent-daemon. One-shot; run on the LOCAL machine (the
# orchestrator Mac) while the external server is still up and the daemon is
# STOPPED.
#
# Usage: scripts/migrate-surreal-embedded.sh [old-server-url]
#   old-server-url defaults to http://127.0.0.1:8000 (what infra-up started).
set -euo pipefail

OLD_URL="${1:-http://127.0.0.1:8000}"
NS="alpha_swarm"
DB="swarm"
USER="root"
PASS="root"
EMBEDDED_DIR="/tmp/alpha-swarm/surrealdb/embedded"
EXPORT_FILE="/tmp/alpha-swarm/migrate-${NS}-${DB}.surql"

echo "==> Exporting ${NS}/${DB} from ${OLD_URL}"
mkdir -p "$(dirname "$EXPORT_FILE")"
surreal export \
    --conn "$OLD_URL" \
    --user "$USER" --pass "$PASS" \
    --ns "$NS" --db "$DB" \
    "$EXPORT_FILE"
echo "    exported to $EXPORT_FILE ($(wc -c < "$EXPORT_FILE") bytes)"

echo "==> Importing into embedded surrealkv at $EMBEDDED_DIR"
echo "    (daemon must NOT be running — surrealkv is single-process)"
mkdir -p "$EMBEDDED_DIR"
# Requires a surreal CLI built with surrealkv support (v2+ official builds).
surreal import \
    --conn "surrealkv://$EMBEDDED_DIR" \
    --ns "$NS" --db "$DB" \
    "$EXPORT_FILE"

echo "==> Done. Start agent-daemon (mode=embedded); init_schema rebuilds the"
echo "    HNSW indexes idempotently on first connect."
echo "    Old server can be decommissioned (scripts/infra-up-local.sh block)."
