#!/usr/bin/env bash
set -euo pipefail

# Setup malna (RPi) as 3rd alpha-swarm lattice node.
#
# Prerequisites:
#   1. SSH key auth working: ssh malna 'echo ok'
#      If not, run on malna:
#        mkdir -p ~/.ssh && chmod 700 ~/.ssh
#        echo 'ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIL8dk2b+EdpX6uank/hVFxHM7rsvPx/1NaUvNaDY8jpi kovarimarkofficial@gmail.com' >> ~/.ssh/authorized_keys
#        chmod 600 ~/.ssh/authorized_keys
#        chmod 755 ~
#
#   2. malna is reachable via Tailscale hostname 'malna'
#
# Usage: ./scripts/setup-malna.sh

REMOTE="malna"
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_DIR="$(dirname "$SCRIPT_DIR")"

echo "=== alpha-swarm: setting up malna (RPi) ==="

# --- Test SSH ---
echo "[0/5] Testing SSH connection..."
if ! ssh -o ConnectTimeout=5 "$REMOTE" 'echo ok' >/dev/null 2>&1; then
    echo "ERROR: Cannot SSH to malna. Set up key auth first (see script header)."
    exit 1
fi
echo "  SSH connection OK"

# --- Get Tailscale IP ---
MALNA_IP=$(ssh "$REMOTE" 'tailscale ip -4 2>/dev/null || ip -4 addr show tailscale0 2>/dev/null | grep inet | awk "{print \$2}" | cut -d/ -f1' 2>/dev/null)
echo "  Tailscale IP: $MALNA_IP"

# --- System info ---
echo ""
ssh "$REMOTE" 'echo "  Arch: $(uname -m)" && echo "  OS: $(cat /etc/os-release 2>/dev/null | grep PRETTY_NAME | cut -d= -f2 | tr -d \")" && echo "  RAM: $(free -h | awk "/Mem:/{print \$2}")" && echo "  Disk: $(df -h / | tail -1 | awk "{print \$2, \$4, \"free\"}")"'

# --- Install NATS ---
echo ""
echo "[1/5] Installing NATS server..."
ssh "$REMOTE" bash <<'INSTALL_NATS'
if command -v nats-server >/dev/null 2>&1; then
    echo "  NATS already installed: $(nats-server --version)"
else
    echo "  Downloading nats-server for ARM64..."
    NATS_VERSION="2.12.6"
    curl -sL "https://github.com/nats-io/nats-server/releases/download/v${NATS_VERSION}/nats-server-v${NATS_VERSION}-linux-arm64.tar.gz" -o /tmp/nats.tar.gz
    tar xzf /tmp/nats.tar.gz -C /tmp/
    sudo mv /tmp/nats-server-v${NATS_VERSION}-linux-arm64/nats-server /usr/local/bin/
    rm -rf /tmp/nats*
    echo "  Installed: $(nats-server --version)"
fi
INSTALL_NATS

# --- Install NATS CLI ---
echo "[2/5] Installing NATS CLI..."
ssh "$REMOTE" bash <<'INSTALL_NATS_CLI'
if command -v nats >/dev/null 2>&1; then
    echo "  NATS CLI already installed"
else
    echo "  Downloading nats CLI for ARM64..."
    NATS_CLI_VERSION="0.3.2"
    curl -sL "https://github.com/nats-io/natscli/releases/download/v${NATS_CLI_VERSION}/nats-${NATS_CLI_VERSION}-linux-arm64.tar.gz" -o /tmp/natscli.tar.gz
    tar xzf /tmp/natscli.tar.gz -C /tmp/
    sudo mv /tmp/nats-${NATS_CLI_VERSION}-linux-arm64/nats /usr/local/bin/
    rm -rf /tmp/nats*
    echo "  Installed"
fi
INSTALL_NATS_CLI

# --- Install SurrealDB ---
echo "[3/5] Installing SurrealDB..."
ssh "$REMOTE" bash <<'INSTALL_SURREAL'
if command -v surreal >/dev/null 2>&1; then
    echo "  SurrealDB already installed: $(surreal version 2>/dev/null || echo unknown)"
else
    echo "  Installing SurrealDB via install script..."
    curl -sSf https://install.surrealdb.com | sh -s -- --nightly 2>/dev/null || {
        echo "  Install script failed. Trying manual download..."
        SURREAL_VERSION="v2.2.0"
        curl -sL "https://github.com/surrealdb/surrealdb/releases/download/${SURREAL_VERSION}/surreal-${SURREAL_VERSION}.linux-arm64.tgz" -o /tmp/surreal.tgz
        tar xzf /tmp/surreal.tgz -C /tmp/
        sudo mv /tmp/surreal /usr/local/bin/
        rm -f /tmp/surreal.tgz
    }
    echo "  Installed: $(surreal version 2>/dev/null || echo unknown)"
fi
INSTALL_SURREAL

# --- Install wash ---
echo "[4/5] Installing wash (wasmCloud)..."
ssh "$REMOTE" bash <<'INSTALL_WASH'
if command -v wash >/dev/null 2>&1; then
    echo "  wash already installed: $(wash --version 2>/dev/null | head -1)"
else
    echo "  Installing wash..."
    curl -s https://packagecloud.io/install/repositories/wasmcloud/core/script.deb.sh | sudo bash 2>/dev/null || true
    sudo apt-get install -y wash 2>/dev/null || {
        echo "  apt install failed. Try: cargo install wash-cli"
    }
fi
INSTALL_WASH

# --- Copy NATS config ---
echo "[5/5] Deploying NATS cluster config..."
# Update malna config with actual Tailscale IP
MALNA_IP_ACTUAL=${MALNA_IP:-TBD}
ssh "$REMOTE" "mkdir -p /tmp/alpha-swarm/nats/jetstream"

# Send the config, substituting the IP
sed "s|TBD|$MALNA_IP_ACTUAL|g" "$PROJECT_DIR/infra/nats-malna.conf" | \
    ssh "$REMOTE" "cat > /tmp/alpha-swarm/nats.conf"
echo "  NATS config deployed to /tmp/alpha-swarm/nats.conf"

# --- Verify ---
echo ""
echo "=== Verification ==="
ssh "$REMOTE" bash <<'VERIFY'
echo "  nats-server: $(command -v nats-server >/dev/null 2>&1 && nats-server --version || echo 'NOT INSTALLED')"
echo "  nats:        $(command -v nats >/dev/null 2>&1 && echo 'installed' || echo 'NOT INSTALLED')"
echo "  surreal:     $(command -v surreal >/dev/null 2>&1 && surreal version 2>/dev/null || echo 'NOT INSTALLED')"
echo "  wash:        $(command -v wash >/dev/null 2>&1 && wash --version 2>/dev/null | head -1 || echo 'NOT INSTALLED')"
VERIFY

echo ""
echo "=== Setup complete ==="
echo "To start malna as a lattice node:"
echo "  ssh malna 'nats-server -c /tmp/alpha-swarm/nats.conf &'"
echo "  ssh malna 'wash host --scheduler-nats-url nats://127.0.0.1:4222 --data-nats-url nats://127.0.0.1:4222 --host-name malna-infra --non-interactive &'"
echo ""
echo "Update infra/nats-malna.conf with Tailscale IP: $MALNA_IP_ACTUAL"
