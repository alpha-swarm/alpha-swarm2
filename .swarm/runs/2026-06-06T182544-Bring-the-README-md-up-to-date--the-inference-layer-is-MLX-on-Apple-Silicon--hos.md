# Bring the README.md up to date: the inference layer is MLX on Apple Silicon (host csatapaci) serving qwen2.5-coder 14B plus 32B, with embeddings on a Raspberry Pi (malna) through Ollama nomic-embed-text; fix the architecture diagram to match; add a short paragraph on the gated autonomous loop; and reference the dashboard at localhost:8001 with its 3D graph. Preserve every other section of the file. Edit README.md only.

- model: unknown
- files: README.md
- when: 2026-06-06T18:25:44.590045+00:00

## Agent output

```

```

## Diff

```diff
--- a/README.md
+++ b/README.md
@@ -4,6 +4,8 @@
 
 ## Architecture
 
+The architecture of alpha-swarm has been updated to include a gated autonomous loop, which ensures that all actions are validated and approved before execution. This loop involves the swarm-orchestrator, agent-daemon, and inference-client, working together to manage tasks and ensure system integrity.
+
 ```mermaid
 graph TD;
   picur[Mac Mini\npicur\nSurrealDB\nWeb UI\nDaemon\nNATS] -- NATS --> csatapaci[M2 Max 96GB\ncsatapaci\nOllama 72B/33B/7B\nDaemon\nNATS];
@@ -11,8 +13,25 @@
   picur --- Tailscale_mesh[Tailscale mesh];
   csatapaci --- Tailscale_mesh;
   malna --- Tailscale_mesh;
+  swarm[swarm-orchestrator] -- Validate --> agent[agent-daemon];
+  agent -- Execute --> inference[inference-client];
+  inference -- Approve --> swarm;
 ```
 
+### Gated Autonomous Loop
+
+The gated autonomous loop is a critical component of alpha-swarm's architecture. It ensures that all tasks are validated and approved before execution, enhancing the system's reliability and security. The loop involves the following steps:
+
+1. **Task Decomposition**: The swarm-orchestrator decomposes high-level goals into parallel sub-tasks.
+2. **Task Validation**: The agent-daemon validates each task to ensure it meets the system's requirements.
+3. **Task Execution**: The agent-daemon executes the validated tasks.
+4. **Result Approval**: The inference-client approves the results of the executed tasks, ensuring they are correct and secure.
+5. **Feedback Loop**: The approved results are fed back into the swarm-orchestrator for further task decomposition and execution.
+
+## Dashboard
+
+The dashboard is accessible at `http://localhost:8001` and features a 3D graph that visualizes the system's architecture and task flow.
+
 ## Components
 
 ### Agent System

```
