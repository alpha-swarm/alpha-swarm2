# Add a module-level //! doc comment at the top of crates/agent-daemon/src/resources.rs summarizing the resource-admission logic

- model: unknown
- files: crates/agent-daemon/src/resources.rs
- when: 2026-06-05T17:41:29.977025+00:00

## Agent output

```

```

## Diff

```diff
--- a/crates/agent-daemon/src/resources.rs
+++ b/crates/agent-daemon/src/resources.rs
@@ -4,7 +4,9 @@
 
 use swarm_config::ResourceConfig;
 
-/// Snapshot of current system resource usage for one host.
+/// This module contains the logic for checking and managing system resources.
+/// It provides functions to snapshot resource usage for both local machines and remote Ollama instances,
+/// as well as determining if there is enough available capacity to schedule new tasks based on configured limits.
 #[derive(Debug, Clone, Serialize)]
 pub struct ResourceSnapshot {
     pub host: String,
@@ -19,6 +21,8 @@
     pub ollama_models: Vec<OllamaModelStatus>,
 }
 
+// Remove the duplicate definition
+
 #[derive(Debug, Clone, Serialize, Deserialize)]
 pub struct OllamaModelStatus {
     pub name: String,

```
