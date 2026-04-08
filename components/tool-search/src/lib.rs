wit_bindgen::generate!({
    path: "wit",
    world: "tool-search",
    pub_export_macro: true,
    generate_all,
});

use exports::wasi::http::incoming_handler::Guest;
use wasi::http::types::*;
use wasi::io::streams::OutputStream;

use serde::{Deserialize, Serialize};

#[derive(Deserialize)]
struct ToolRequest {
    name: String,
    #[serde(default)]
    params_json: String,
    #[serde(default)]
    repo_path: String,
    #[serde(default)]
    #[allow(dead_code)]
    project: String,
    #[serde(default)]
    #[allow(dead_code)]
    timeout_ms: u64,
}

#[derive(Serialize)]
struct ToolResponse {
    content: String,
    is_error: bool,
    duration_ms: u64,
}

struct Component;

export!(Component);

impl Guest for Component {
    fn handle(request: IncomingRequest, response_out: ResponseOutparam) {
        let (method, _path) = {
            let m = request.method();
            let p = request.path_with_query().unwrap_or_default();
            (m, p)
        };

        // Only accept POST
        if !matches!(method, Method::Post) {
            respond(response_out, 405, r#"{"error":"method not allowed"}"#);
            return;
        }

        // Read body
        let body = match read_body(&request) {
            Ok(b) => b,
            Err(e) => {
                respond(response_out, 400, &format!(r#"{{"error":"read body: {e}"}}"#));
                return;
            }
        };

        // Parse request
        let req: ToolRequest = match serde_json::from_slice(&body) {
            Ok(r) => r,
            Err(e) => {
                respond(response_out, 400, &format!(r#"{{"error":"invalid json: {e}"}}"#));
                return;
            }
        };

        // Dispatch tool
        let result = execute_tool(&req);
        let json = serde_json::to_string(&result).unwrap_or_else(|_| r#"{"content":"serialize error","is_error":true,"duration_ms":0}"#.into());
        respond(response_out, 200, &json);
    }
}

fn execute_tool(req: &ToolRequest) -> ToolResponse {
    let params: serde_json::Value = serde_json::from_str(&req.params_json)
        .unwrap_or(serde_json::Value::Object(Default::default()));

    match req.name.as_str() {
        "grep" => tool_grep(&params, &req.repo_path),
        "ts_find" => tool_ts_find(&params, &req.repo_path),
        "ts_signatures" => tool_ts_signatures(&params, &req.repo_path),
        _ => ToolResponse {
            content: format!("Unknown tool: {}", req.name),
            is_error: true,
            duration_ms: 0,
        },
    }
}

fn tool_grep(params: &serde_json::Value, _repo_path: &str) -> ToolResponse {
    let pattern = params.get("pattern").and_then(|p| p.as_str()).unwrap_or("");
    let content = params.get("content").and_then(|c| c.as_str()).unwrap_or("");

    if pattern.is_empty() {
        return ToolResponse { content: "Missing 'pattern'".into(), is_error: true, duration_ms: 0 };
    }

    // Simple in-memory grep (no filesystem access needed if content is provided)
    let matches: Vec<String> = content.lines()
        .enumerate()
        .filter(|(_, line)| line.contains(pattern))
        .map(|(i, line)| format!("{}:{}", i + 1, line))
        .take(100)
        .collect();

    if matches.is_empty() {
        ToolResponse { content: "No matches found".into(), is_error: false, duration_ms: 0 }
    } else {
        ToolResponse { content: matches.join("\n"), is_error: false, duration_ms: 0 }
    }
}

fn tool_ts_find(params: &serde_json::Value, _repo_path: &str) -> ToolResponse {
    let symbol = params.get("symbol").and_then(|s| s.as_str()).unwrap_or("");
    let content = params.get("content").and_then(|c| c.as_str()).unwrap_or("");

    if symbol.is_empty() {
        return ToolResponse { content: "Missing 'symbol'".into(), is_error: true, duration_ms: 0 };
    }

    // Simple text-based find (tree-sitter would be added when it compiles to WASI)
    let matches: Vec<String> = content.lines()
        .enumerate()
        .filter(|(_, line)| line.contains(symbol))
        .map(|(i, line)| format!("{}:{}", i + 1, line.trim()))
        .take(50)
        .collect();

    ToolResponse {
        content: format!("{} occurrences of '{symbol}':\n{}", matches.len(), matches.join("\n")),
        is_error: false,
        duration_ms: 0,
    }
}

fn tool_ts_signatures(params: &serde_json::Value, _repo_path: &str) -> ToolResponse {
    let content = params.get("content").and_then(|c| c.as_str()).unwrap_or("");

    // Extract function/struct signatures via simple pattern matching
    // (tree-sitter AST version would be more accurate)
    let sigs: Vec<String> = content.lines()
        .enumerate()
        .filter(|(_, line)| {
            let trimmed = line.trim();
            trimmed.starts_with("pub fn ")
                || trimmed.starts_with("fn ")
                || trimmed.starts_with("pub struct ")
                || trimmed.starts_with("struct ")
                || trimmed.starts_with("pub enum ")
                || trimmed.starts_with("enum ")
                || trimmed.starts_with("pub trait ")
                || trimmed.starts_with("impl ")
        })
        .map(|(i, line)| format!("{}:{}", i + 1, line.trim().trim_end_matches('{'). trim()))
        .take(200)
        .collect();

    ToolResponse {
        content: sigs.join("\n"),
        is_error: false,
        duration_ms: 0,
    }
}

fn read_body(request: &IncomingRequest) -> Result<Vec<u8>, String> {
    let body = request.consume().map_err(|_| "consume failed")?;
    let stream = body.stream().map_err(|_| "stream failed")?;
    let mut bytes = Vec::new();
    loop {
        match stream.read(65536) {
            Ok(chunk) if chunk.is_empty() => break,
            Ok(chunk) => bytes.extend_from_slice(&chunk),
            Err(_) => break,
        }
    }
    Ok(bytes)
}

fn respond(response_out: ResponseOutparam, status: u16, body: &str) {
    let response = OutgoingResponse::new(Headers::new());
    response.set_status_code(status).ok();

    let out_body = response.body().expect("body");
    ResponseOutparam::set(response_out, Ok(response));

    let stream = out_body.write().expect("write stream");
    write_all(&stream, body.as_bytes());
    drop(stream);
    OutgoingBody::finish(out_body, None).expect("finish");
}

fn write_all(stream: &OutputStream, data: &[u8]) {
    let mut offset = 0;
    while offset < data.len() {
        let chunk_size = (data.len() - offset).min(4096);
        match stream.check_write() {
            Ok(n) => {
                let n = n.min(chunk_size as u64) as usize;
                if n == 0 { continue; }
                let _ = stream.write(&data[offset..offset + n]);
                let _ = stream.flush();
                offset += n;
            }
            Err(_) => break,
        }
    }
}
