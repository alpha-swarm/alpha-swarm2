// Gateway component — exposes swarm control to external clients.
// Compiled to wasm32-wasip2, runs inside WasmCloud.
//
// Imports: project/manager, swarm/control, events/bus
// Exports: wasi:http/incoming-handler (later)
//
// Initially bridges WIT calls + NATS event subscriptions.
// HTTP/MCP/SSE transport added in a later phase.
