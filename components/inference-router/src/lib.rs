// Inference router component — multi-backend model routing.
// Compiled to wasm32-wasip2, runs inside WasmCloud.
//
// Imports: inference/backend (from each provider), ollama/provider, claude/provider
// Exports: inference/completions (what agents and orchestrator consume)
//
// Routing logic:
//   1. Check which backends are healthy
//   2. Match complexity tier to available models
//   3. Prefer local (Ollama) for simple/medium tasks
//   4. Prefer Claude for complex tasks or when local models fail
//   5. Fallback chain: preferred → next best → error
