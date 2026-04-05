// Agent worker component — one-shot task executor.
// Compiled to wasm32-wasip2, runs inside WasmCloud.
//
// Imports: ollama/inference, virtfs/repository, quality/gate, knowledge/base
// Exports: handler (handle-task)
//
// TODO: Generate bindings from WIT and implement handle_task.
