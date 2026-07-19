---
key: agent_run:v8m9bqn16n2tdg3qihzj
project: alpha-swarm2
namespace: errors
use_count: 10
---

GOAL: Update the top-level README.md to match the current system. (1) In the Inference section replace the outdated models: chat inference now runs on MLX (mlx_lm.server) on csatapaci serving qwen2.5-coder 14B (planner + worker) and 32B (refactor/complex escalation); embeddings run on the malna Raspberry Pi 5 via Ollama nomic-embed-text (768-dim). Remove the qwen2.5:72b, deepseek-coder:33b and 7b references. (2) Update the mermaid diagram so the csatapaci node says MLX 14B/32B and the malna node says Ollama nomic embeddings. (3) Add a short Autonomous loop subsection: the daemon drains an autopilot goal backlog one run at a time, every change is quality-gated with cargo check and test before landing on the swarm/auto branch with an auto-opened PR, and the SONA loop learns reusable patterns from passing runs. (4) Mention the Leptos dashboard served at :8001 with a 3D knowledge graph. Keep the existing Components and Tools sections; only edit README.md.
FAILED PLAN:
[passed] task-1: Update the Inference section in README.md to replace outdated models and update the mermaid diagram.
[passed] task-2: Add a short Autonomous loop subsection to README.md.
[passed] task-3: Mention the Leptos dashboard served at :8001 with a 3D knowledge graph in README.md.
