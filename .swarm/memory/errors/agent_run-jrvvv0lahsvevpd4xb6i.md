---
key: agent_run:jrvvv0lahsvevpd4xb6i
project: alpha-swarm2
namespace: errors
use_count: 5
---

GOAL: Add a module-level //! doc comment at the very top of crates/inference-client/src/router.rs explaining the routing strategy: requests carry an optional preferred_model and preferred_backend; when a preferred model is set and multiple backends exist, the router routes to the backend that actually hosts that model (via list_models_indexed); when no preference is given, complexity tiers (Simple/Medium/Complex) choose the model size; escalation bumps to a larger model on refactor or complex tasks. Keep it to one concise paragraph. Only edit router.rs.
FAILED PLAN:
[passed] task-1: Add a module-level doc comment at the top of crates/inference-client/src/router.rs explaining the routing strategy.
