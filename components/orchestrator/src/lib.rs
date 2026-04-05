// Orchestrator component — decomposes goals into sub-tasks.
// Compiled to wasm32-wasip2, runs inside WasmCloud.
//
// Imports: ollama/inference, virtfs/repository, quality/gate, knowledge/base
// Exports: planner (plan, merge-results)
//
// TODO: Generate bindings from WIT and implement plan + merge_results.
