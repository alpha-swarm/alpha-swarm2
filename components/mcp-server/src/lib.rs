//! MCP Server — WASI component implementing Model Context Protocol (Streamable HTTP).
//!
//! Exposes alpha-swarm capabilities as MCP tools and resources.
//! Transport: HTTP POST /mcp with JSON-RPC 2.0 body.
//! Deployed via wasmCloud alongside other components.

wit_bindgen::generate!({
    path: "wit",
    world: "mcp-server",
    pub_export_macro: true,
    generate_all,
});

use exports::wasi::http::incoming_handler::Guest;
use wasi::http::outgoing_handler;
use wasi::http::types::*;

mod mcp;
mod tools;
mod resources;

// --- Configuration constants ---

/// SurrealDB connection.
const SURREAL_HOST: &str = "127.0.0.1:8000";
const SURREAL_PATH: &str = "/sql";
const SURREAL_NS: &str = "alpha_swarm";
const SURREAL_DB: &str = "alpha_swarm";
/// base64("root:root")
const SURREAL_AUTH: &str = "Basic cm9vdDpyb290";

/// Ollama inference server.
pub const OLLAMA_HOST: &str = "100.81.10.8:11434";

/// MCP protocol.
const MCP_CONTENT_TYPE: &str = "application/json";
const MCP_SESSION_HEADER: &str = "mcp-session-id";
const MCP_SESSION_ID: &str = "alpha-swarm-static";

struct McpServer;

impl Guest for McpServer {
    fn handle(request: IncomingRequest, response_out: ResponseOutparam) {
        let method = request.method();
        let path = request.path_with_query().unwrap_or_default();

        match (method, path.as_str()) {
            (Method::Post, "/mcp") => {
                let body = read_body(&request);
                let response = mcp::handle_jsonrpc(&body);
                respond_json(response_out, 200, &response);
            }
            (Method::Get, "/mcp") => {
                respond_json(response_out, 200, r#"{"info":"SSE notifications planned for future release"}"#);
            }
            (Method::Get, "/health") => {
                respond_json(response_out, 200, &format!(
                    r#"{{"status":"ok","protocol":"mcp","version":"{}"}}"#,
                    mcp::MCP_PROTOCOL_VERSION,
                ));
            }
            _ => {
                respond_json(response_out, 404, r#"{"error":"Not found. Use POST /mcp for MCP JSON-RPC."}"#);
            }
        }
    }
}

export!(McpServer);

// --- HTTP helpers ---

fn read_body(request: &IncomingRequest) -> String {
    let Some(body) = request.consume().ok() else {
        return String::new();
    };
    let stream = body.stream().unwrap();
    let mut buf = Vec::new();
    loop {
        match stream.blocking_read(64 * 1024) {
            Ok(chunk) if chunk.is_empty() => break,
            Ok(chunk) => buf.extend_from_slice(&chunk),
            Err(_) => break,
        }
    }
    drop(stream);
    let _ = IncomingBody::finish(body);
    String::from_utf8(buf).unwrap_or_default()
}

fn respond_json(out: ResponseOutparam, status: u16, body: &str) {
    let resp = OutgoingResponse::new(Fields::new());
    resp.set_status_code(status).ok();

    let headers = resp.headers();
    headers.append("content-type", &MCP_CONTENT_TYPE.as_bytes()[..]).ok();
    headers.append(MCP_SESSION_HEADER, &MCP_SESSION_ID.as_bytes()[..]).ok();

    let out_body = resp.body().unwrap();
    ResponseOutparam::set(out, Ok(resp));

    let stream = out_body.write().unwrap();
    stream.blocking_write_and_flush(body.as_bytes()).ok();
    drop(stream);
    OutgoingBody::finish(out_body, None).ok();
}

/// Make an HTTP request to SurrealDB REST API.
pub fn surreal_query(query: &str) -> Result<String, String> {
    let headers = Fields::new();
    headers.append("content-type", &MCP_CONTENT_TYPE.as_bytes()[..]).ok();
    headers.append("accept", &MCP_CONTENT_TYPE.as_bytes()[..]).ok();
    headers.append("ns", SURREAL_NS.as_bytes()).ok();
    headers.append("db", SURREAL_DB.as_bytes()).ok();
    headers.append("authorization", SURREAL_AUTH.as_bytes()).ok();

    let req = OutgoingRequest::new(headers);
    req.set_method(&Method::Post).map_err(|_| "set method")?;
    req.set_scheme(Some(&Scheme::Http)).map_err(|_| "set scheme")?;
    req.set_authority(Some(SURREAL_HOST)).map_err(|_| "set authority")?;
    req.set_path_with_query(Some(SURREAL_PATH)).map_err(|_| "set path")?;

    let body = req.body().map_err(|_| "get body")?;
    let stream = body.write().map_err(|_| "get write stream")?;
    stream.blocking_write_and_flush(query.as_bytes()).map_err(|e| format!("write body: {e:?}"))?;
    drop(stream);
    OutgoingBody::finish(body, None).map_err(|_| "finish body")?;

    let future_resp = outgoing_handler::handle(req, None).map_err(|_| "send request")?;
    let poll = future_resp.subscribe();
    poll.block();

    let resp = future_resp.get()
        .ok_or("no response")?
        .map_err(|_| "future error")?
        .map_err(|e| format!("http error: {e:?}"))?;

    read_response_body(resp)
}

/// Make a GET request to an external HTTP service.
pub fn http_get(host: &str, path: &str) -> Result<String, String> {
    let headers = Fields::new();
    headers.append("accept", &MCP_CONTENT_TYPE.as_bytes()[..]).ok();

    let req = OutgoingRequest::new(headers);
    req.set_method(&Method::Get).map_err(|_| "set method")?;
    req.set_scheme(Some(&Scheme::Http)).map_err(|_| "set scheme")?;
    req.set_authority(Some(host)).map_err(|_| "set authority")?;
    req.set_path_with_query(Some(path)).map_err(|_| "set path")?;

    let body = req.body().map_err(|_| "get body")?;
    OutgoingBody::finish(body, None).map_err(|_| "finish body")?;

    let future_resp = outgoing_handler::handle(req, None).map_err(|_| "send request")?;
    let poll = future_resp.subscribe();
    poll.block();

    let resp = future_resp.get()
        .ok_or("no response")?
        .map_err(|_| "future error")?
        .map_err(|e| format!("http error: {e:?}"))?;

    read_response_body(resp)
}

fn read_response_body(resp: IncomingResponse) -> Result<String, String> {
    let resp_body = resp.consume().map_err(|_| "consume response")?;
    let resp_stream = resp_body.stream().map_err(|_| "get response stream")?;
    let mut buf = Vec::new();
    loop {
        match resp_stream.blocking_read(64 * 1024) {
            Ok(chunk) if chunk.is_empty() => break,
            Ok(chunk) => buf.extend_from_slice(&chunk),
            Err(_) => break,
        }
    }
    drop(resp_stream);
    let _ = IncomingBody::finish(resp_body);

    String::from_utf8(buf).map_err(|_| "response not utf8".into())
}
