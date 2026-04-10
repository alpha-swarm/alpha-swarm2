//! MCP JSON-RPC 2.0 protocol handler.
//!
//! Implements the MCP spec (2025-11-25) Streamable HTTP transport.
//! Methods: initialize, tools/list, tools/call, resources/list, resources/read.

use serde_json::{json, Value};

/// Protocol version we implement.
pub const MCP_PROTOCOL_VERSION: &str = "2025-11-25";
const SERVER_NAME: &str = "alpha-swarm";
const SERVER_VERSION: &str = "0.1.0";

/// Handle a JSON-RPC 2.0 request and return a JSON-RPC response.
pub fn handle_jsonrpc(body: &str) -> String {
    let req: Value = match serde_json::from_str(body) {
        Ok(v) => v,
        Err(e) => return jsonrpc_error(Value::Null, -32700, &format!("Parse error: {e}")),
    };

    let id = req.get("id").cloned().unwrap_or(Value::Null);
    let method = req.get("method").and_then(|m| m.as_str()).unwrap_or("");
    let params = req.get("params").cloned().unwrap_or(json!({}));

    let result = match method {
        "initialize" => handle_initialize(&params),
        "notifications/initialized" => return String::new(), // notification, no response
        "ping" => Ok(json!({})),
        "tools/list" => handle_tools_list(),
        "tools/call" => handle_tools_call(&params),
        "resources/list" => handle_resources_list(),
        "resources/read" => handle_resources_read(&params),
        "prompts/list" => Ok(json!({"prompts": []})),
        _ => Err(format!("Method not found: {method}")),
    };

    match result {
        Ok(result) => jsonrpc_ok(id, result),
        Err(e) => jsonrpc_error(id, -32601, &e),
    }
}

fn handle_initialize(_params: &Value) -> Result<Value, String> {
    Ok(json!({
        "protocolVersion": MCP_PROTOCOL_VERSION,
        "serverInfo": {
            "name": SERVER_NAME,
            "version": SERVER_VERSION,
        },
        "capabilities": {
            "tools": { "listChanged": false },
            "resources": { "subscribe": false, "listChanged": false },
        }
    }))
}

fn handle_tools_list() -> Result<Value, String> {
    Ok(json!({
        "tools": crate::tools::list_tools()
    }))
}

fn handle_tools_call(params: &Value) -> Result<Value, String> {
    let name = params.get("name").and_then(|n| n.as_str()).unwrap_or("");
    let args = params.get("arguments").cloned().unwrap_or(json!({}));
    crate::tools::call_tool(name, &args)
}

fn handle_resources_list() -> Result<Value, String> {
    Ok(json!({
        "resources": crate::resources::list_resources()
    }))
}

fn handle_resources_read(params: &Value) -> Result<Value, String> {
    let uri = params.get("uri").and_then(|u| u.as_str()).unwrap_or("");
    crate::resources::read_resource(uri)
}

// --- JSON-RPC helpers ---

fn jsonrpc_ok(id: Value, result: Value) -> String {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "result": result,
    }).to_string()
}

fn jsonrpc_error(id: Value, code: i32, message: &str) -> String {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "error": {
            "code": code,
            "message": message,
        }
    }).to_string()
}
