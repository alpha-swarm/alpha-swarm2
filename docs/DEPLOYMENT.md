# alpha-swarm Deployment Guide

## Overview

alpha-swarm runs as a distributed system across multiple machines coordinated by NATS. Each machine runs an agent-daemon that picks up tasks via NATS KV, executes them using local Ollama models, and stores results in SurrealDB.

```
                        NATS Cluster (JetStream + KV)
                    ┌──────────┬───────────┬──────────┐
                    │          │           │          │
              ┌─────▼────┐ ┌──▼──────┐ ┌──▼──────┐
              │  picur    │ │csatapaci│ │  malna   │
              │  Mac Mini │ │ M2 Max  │ │  RPi 5   │
              │           │ │ 96GB    │ │  8GB     │
              │ SurrealDB │ │ Ollama  │ │ NATS     │
              │ Web UI    │ │ 72B/33B │ │ quorum   │
              │ Daemon    │ │ Daemon  │ │ (future: │
              │           │ │         │ │  WASI    │
              │           │ │         │ │  tools)  │
              └───────────┘ └─────────┘ └─────────┘
                Tailscale mesh: 100.x.x.x
```

## Machine Roles

| Machine | Role | Services | Tailscale IP |
|---------|------|----------|-------------|
| **picur** | Orchestrator | SurrealDB, NATS, Web UI, Daemon | 100.79.38.122 |
| **csatapaci** | Inference worker | Ollama (72B/33B/7B), NATS, Daemon | 100.81.10.8 |
| **malna** | NATS quorum + tools | NATS (3rd node for quorum) | 100.111.200.86 |

## Prerequisites

All machines need:
- **Tailscale** — mesh VPN for connectivity (all machines must be on the same tailnet)
- **SSH access** — key-based auth between machines

Per-role:
- **Orchestrator**: SurrealDB, NATS server, Rust toolchain (for building)
- **Inference worker**: Ollama with models pulled, NATS server
- **Quorum node**: NATS server only (minimal requirements)

## 1. Tailscale Setup

Install Tailscale on each machine:

```bash
# macOS
brew install tailscale

# Linux (Debian/Ubuntu/RPi)
curl -fsSL https://tailscale.com/install.sh | sh
sudo tailscale up
```

Verify connectivity between all nodes:
```bash
tailscale ping csatapaci
tailscale ping malna
```

## 2. NATS Cluster Setup

NATS provides the coordination backbone: task queue (KV), pub/sub events, and distributed locking via leases.

### 2.1 Install NATS

**macOS (Homebrew):**
```bash
brew install nats-server
```

**Linux (binary download):**
```bash
# aarch64
curl -L https://github.com/nats-io/nats-server/releases/latest/download/nats-server-v2.12.6-linux-arm64.tar.gz -o /tmp/nats.tar.gz
sudo tar xzf /tmp/nats.tar.gz --strip-components=1 -C /usr/sbin/ nats-server-v2.12.6-linux-arm64/nats-server

# x86_64
curl -L https://github.com/nats-io/nats-server/releases/latest/download/nats-server-v2.12.6-linux-amd64.tar.gz -o /tmp/nats.tar.gz
sudo tar xzf /tmp/nats.tar.gz --strip-components=1 -C /usr/sbin/ nats-server-v2.12.6-linux-amd64/nats-server
```

### 2.2 Configuration

Each node needs a config file. Replace Tailscale IPs with your own.

**Node 1 (orchestrator) — e.g., `/Users/you/Library/NATS/nats-server.conf`:**
```
listen: 0.0.0.0:4223
server_name: orchestrator
http: 0.0.0.0:8223

jetstream {
    store_dir: /tmp/nats/jetstream
    max_mem: 1G
    max_file: 10G
}

cluster {
    name: alpha_swarm
    listen: 0.0.0.0:4248
    routes = [
        nats-route://<WORKER_TAILSCALE_IP>:6222
        nats-route://<QUORUM_TAILSCALE_IP>:6222
    ]
}
```

**Node 2 (worker) — e.g., `~/Library/NATS/nats-server.conf`:**
```
listen: 0.0.0.0:4222
server_name: worker
http: 0.0.0.0:8222

jetstream {
    store_dir: /tmp/nats/jetstream
    max_mem: 1G
    max_file: 4G
}

cluster {
    name: alpha_swarm
    listen: 0.0.0.0:6222
    routes = [
        nats-route://<ORCHESTRATOR_TAILSCALE_IP>:4248
        nats-route://<QUORUM_TAILSCALE_IP>:6222
    ]
}
```

**Node 3 (quorum) — e.g., `/etc/nats/nats-server.conf`:**
```
listen: 0.0.0.0:4222
server_name: quorum
http_port: 8222

jetstream {
    store_dir: /var/lib/nats/jetstream
    max_mem: 512MB
    max_file: 100GB
}

cluster {
    name: alpha_swarm
    listen: 0.0.0.0:6222
    routes = [
        nats-route://<ORCHESTRATOR_TAILSCALE_IP>:4248
        nats-route://<WORKER_TAILSCALE_IP>:6222
    ]
}
```

**Key rules:**
- All nodes must use the same `cluster.name` (e.g., `alpha_swarm`)
- Each node routes to the OTHER two nodes (not itself)
- JetStream must be enabled on all nodes for KV to replicate
- `listen: 0.0.0.0` so Tailscale traffic is accepted

### 2.3 Service Management

**macOS (launchd):**

Create `/Library/LaunchDaemons/nats.server.plist`:
```xml
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN"
  "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
  <dict>
    <key>Label</key>
    <string>io.nats.server</string>
    <key>ProgramArguments</key>
    <array>
      <string>/opt/homebrew/bin/nats-server</string>
      <string>-c</string>
      <string>/Users/YOU/Library/NATS/nats-server.conf</string>
    </array>
    <key>RunAtLoad</key>
    <true/>
    <key>KeepAlive</key>
    <true/>
    <key>StandardOutPath</key>
    <string>/var/log/nats-server.log</string>
    <key>StandardErrorPath</key>
    <string>/var/log/nats-server.err.log</string>
  </dict>
</plist>
```

Load it:
```bash
sudo launchctl load /Library/LaunchDaemons/nats.server.plist
```

**Linux (systemd):**

Create `/etc/systemd/system/nats-server.service`:
```ini
[Unit]
Description=NATS Server
After=network.target

[Service]
Type=simple
ExecStart=/usr/sbin/nats-server -c /etc/nats/nats-server.conf
Restart=on-failure
User=nats
Group=nats

[Install]
WantedBy=multi-user.target
```

Enable and start:
```bash
sudo mkdir -p /var/lib/nats/jetstream
sudo chown nats:nats /var/lib/nats/jetstream
sudo systemctl enable nats-server
sudo systemctl start nats-server
```

### 2.4 Verify Cluster

Check routes from any node:
```bash
curl -s http://localhost:8223/routez | python3 -c "import sys,json; d=json.load(sys.stdin); print(f'Routes: {d[\"num_routes\"]}')"
```

Should show routes to the other two nodes. If 0, check:
- Tailscale connectivity: `tailscale ping <other_node>`
- Firewall: ensure cluster port (4248/6222) is open
- Config: cluster name must match on all nodes

## 3. SurrealDB Setup

Only needed on the orchestrator machine.

```bash
# macOS
brew install surrealdb

# Linux
curl -sSf https://install.surrealdb.com | sh
```

Start with network binding (so workers can reach it):
```bash
surreal start --bind 0.0.0.0:8001 --user root --pass root file:/var/lib/surrealdb
```

**Important:** Use `file:` storage for persistence, not `memory`. With `memory`, data is lost on restart.

For production, use systemd/launchd like NATS.

## 4. Ollama Setup (Inference Workers)

Install on machines that will run LLM inference:

```bash
# macOS
brew install ollama

# Linux
curl -fsSL https://ollama.com/install.sh | sh
```

Pull models based on available RAM:

| RAM | Recommended Models |
|-----|-------------------|
| 96GB+ | `qwen2.5:72b` (orchestrator), `deepseek-coder:33b` (agent), `qwen2.5-coder:7b` (worker) |
| 32-64GB | `deepseek-coder:33b` (agent), `qwen2.5-coder:7b` (worker) |
| 8-16GB | `qwen2.5-coder:7b` (worker only) |

```bash
ollama pull qwen2.5:72b
ollama pull deepseek-coder:33b
ollama pull qwen2.5-coder:7b
```

## 5. Agent Daemon Setup

### 5.1 Build

On the build machine (must have Rust nightly + target):

```bash
cargo build -p agent-daemon --release
```

The binary is at `target/release/agent-daemon`.

### 5.2 Configuration

Each daemon needs `alpha-swarm.toml` in its working directory:

**Orchestrator daemon (runs SurrealDB locally):**
```toml
[ollama]
url = "http://<WORKER_TAILSCALE_IP>:11434"   # remote Ollama

[surrealdb]
url = "127.0.0.1:8001"                        # local SurrealDB
namespace = "alpha_swarm"
database = "swarm"
username = "root"
password = "root"

[nats]
url = "nats://127.0.0.1:4223"                 # local NATS

[tiers.orchestrator]
model = "qwen2.5:72b"

[tiers.agent]
model = "deepseek-coder:33b"

[tiers.worker]
model = "qwen2.5-coder:7b"

[resources]
max_cpu_percent = 80.0
max_ram_percent = 80.0
max_concurrent_agents = 2
```

**Worker daemon (Ollama local, SurrealDB + NATS remote):**
```toml
[ollama]
url = "http://127.0.0.1:11434"                # local Ollama

[surrealdb]
url = "<ORCHESTRATOR_TAILSCALE_IP>:8001"       # remote SurrealDB
namespace = "alpha_swarm"
database = "swarm"
username = "root"
password = "root"

[nats]
url = "nats://<ORCHESTRATOR_TAILSCALE_IP>:4223"  # remote NATS

# Same tier config as orchestrator
```

### 5.3 Deploy to Remote Machine

```bash
scp target/release/agent-daemon <remote>:~/alpha-swarm/agent-daemon
scp alpha-swarm.toml <remote>:~/alpha-swarm/alpha-swarm.toml
# Edit the remote config to point to correct IPs
```

### 5.4 Run

```bash
cd ~/alpha-swarm
./agent-daemon >> /tmp/agent-daemon.log 2>&1 &
```

For production, create a systemd/launchd service like NATS.

Check logs:
```bash
tail -f /tmp/agent-daemon.log | grep "NATS KV\|claimed\|task"
```

Should show: `NATS KV scheduler connected` and `Primary mode: watching NATS KV for tasks`.

## 6. Web UI (Trunk + Leptos)

Only on the orchestrator:

```bash
cd frontend
trunk serve --address 0.0.0.0 --port 3000
```

Accessible at `http://<ORCHESTRATOR_TAILSCALE_IP>:3000` from any machine on the tailnet.

The web UI proxies API calls to the WASI web-ui component on port 8000 (configured in `frontend/Trunk.toml`).

## 7. Verification Checklist

After setup, verify each component:

```bash
# 1. NATS cluster (from any node)
curl -s http://localhost:8223/routez | python3 -c "import sys,json; print(json.load(sys.stdin)['num_routes'])"
# Expected: > 0 (routes to other nodes)

# 2. SurrealDB (from orchestrator)
curl -s http://localhost:8001/health
# Expected: 200 OK

# 3. Ollama (from worker)
curl -s http://localhost:11434/api/tags | python3 -c "import sys,json; [print(m['name']) for m in json.load(sys.stdin)['models']]"
# Expected: list of pulled models

# 4. Daemon (check logs)
grep "NATS KV scheduler connected" /tmp/agent-daemon.log
# Expected: connected message

# 5. Web UI
curl -s http://localhost:3000/ | head -1
# Expected: <!DOCTYPE html>
```

## 8. Adding a New Machine

To add a 4th (or Nth) machine to the swarm:

1. Install Tailscale, join the tailnet
2. Install NATS, configure with `cluster.routes` pointing to existing nodes
3. Update existing nodes' NATS configs to include the new node's route
4. Restart NATS on all nodes (cluster auto-discovers)
5. Copy `agent-daemon` binary + `alpha-swarm.toml` (adjust IPs)
6. Start the daemon — it will connect to NATS KV and start competing for tasks

No code changes needed. The daemon auto-discovers work via NATS KV watch.

## 9. Troubleshooting

**NATS routes show 0:**
- Check Tailscale: `tailscale ping <other_node>`
- Check ports: `curl http://<other_node_ip>:8222/varz`
- Check cluster name matches on all nodes
- Check firewall allows cluster port (4248/6222)

**Daemon says "NATS KV unavailable, will use SurrealDB polling":**
- NATS not reachable at configured URL
- Check `[nats] url` in `alpha-swarm.toml` matches actual NATS port

**Daemon says "Failed to connect to SurrealDB":**
- SurrealDB not listening on 0.0.0.0 (check `--bind` flag)
- Wrong port in config

**"Waiting for routing to be established":**
- Normal during startup — NATS retries route connections
- If persistent: other nodes not running or unreachable

**Two NATS processes running (macOS):**
- Check for both launchd and brew services: `ps aux | grep nats`
- Stop one: `brew services stop nats-server` or `sudo launchctl unload /Library/LaunchDaemons/nats.server.plist`
- Only use ONE service manager per machine

## 10. Recommendations

- **Use file storage for SurrealDB**, not `memory` — data survives restarts
- **Use systemd/launchd** for all services — auto-restart on crash, start on boot
- **3 NATS nodes minimum** for JetStream quorum — if one node dies, the other two maintain consensus
- **Pin Ollama to specific models** in `alpha-swarm.toml` — don't let the router auto-select codellama (it doesn't follow structured output)
- **Prefer deepseek-coder and qwen2.5-coder** over codellama for code generation
- **Use qwen2.5:72b for planning only** (orchestrator tier) — it's slow but smart
- **Lease TTL is 10 minutes** — tasks running longer than 10min need heartbeats (the daemon does this automatically)
- **Monitor disk on all nodes** — JetStream stores data on disk, git clones take space
- **Tailscale is required** — all inter-machine communication goes over the tailnet
