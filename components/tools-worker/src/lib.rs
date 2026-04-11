wit_bindgen::generate!({
    path: "wit",
    world: "tools-worker",
    pub_export_macro: true,
    generate_all,
});

use exports::wasi::http::incoming_handler::Guest;
use wasi::http::types::*;

struct ToolsWorker;

/// Handles tool execution requests.
/// POST /: {tool, params, workspace_id} → {result, is_error}
impl Guest for ToolsWorker {
    fn handle(request: IncomingRequest, response_out: ResponseOutparam) {
        if matches!(request.method(), Method::Get) {
            respond_json(response_out, 200, r#"{"status":"ok","component":"tools-worker","tools":["read_file","write_file","list_files","grep","diff"]}"#);
            return;
        }

        let body = read_body(&request);
        let body_str = String::from_utf8_lossy(&body);

        let req: serde_json::Value = match serde_json::from_str(&body_str) {
            Ok(v) => v,
            Err(e) => {
                respond_json(response_out, 400, &format!(r#"{{"error":"{}"}}"#, e));
                return;
            }
        };

        let tool = req["tool"].as_str().unwrap_or("");
        let _workspace_id = req["workspace_id"].as_str().unwrap_or("default");

        // Create in-memory workspace
        let mut store = virt_git::MemoryBlobStore::new();
        let mut ws = virt_git::VirtWorkspace::new();

        // Load files from request
        if let Some(files) = req["files"].as_array() {
            for f in files {
                let path = f["path"].as_str().unwrap_or("");
                let content = f["content"].as_str().unwrap_or("");
                if !path.is_empty() {
                    ws.load_file(&mut store, path, content);
                }
            }
        }

        let result = match tool {
            "read_file" => {
                let path = req["params"]["path"].as_str().unwrap_or("");
                match ws.read_file(&store, path) {
                    Some(content) => serde_json::json!({"content": content, "is_error": false}),
                    None => serde_json::json!({"content": format!("File not found: {path}"), "is_error": true}),
                }
            }
            "write_file" => {
                let path = req["params"]["path"].as_str().unwrap_or("");
                let content = req["params"]["content"].as_str().unwrap_or("");
                ws.write_file(&mut store, path, content);
                serde_json::json!({"content": format!("Written {} bytes to {path}", content.len()), "is_error": false})
            }
            "list_files" => {
                let files = ws.list_files();
                serde_json::json!({"content": files.join("\n"), "is_error": false})
            }
            "grep" => {
                let pattern = req["params"]["pattern"].as_str().unwrap_or("");
                let mut matches = Vec::new();
                for file in ws.list_files() {
                    if let Some(content) = ws.read_file(&store, file) {
                        for (i, line) in content.lines().enumerate() {
                            if line.contains(pattern) {
                                matches.push(format!("{}:{}:{}", file, i + 1, line));
                                if matches.len() >= 100 { break; }
                            }
                        }
                    }
                }
                serde_json::json!({"content": if matches.is_empty() { "No matches".into() } else { matches.join("\n") }, "is_error": false})
            }
            "diff" => {
                let diff = ws.diff_text(&store);
                serde_json::json!({"content": if diff.is_empty() { "No changes".into() } else { diff }, "is_error": false})
            }
            _ => serde_json::json!({"content": format!("Unknown tool: {tool}"), "is_error": true}),
        };

        respond_json(response_out, 200, &result.to_string());
    }
}

fn read_body(request: &IncomingRequest) -> Vec<u8> {
    let req_body = request.consume().unwrap();
    let stream = req_body.stream().unwrap();
    let mut body = Vec::new();
    loop {
        match stream.read(65536) {
            Ok(chunk) if chunk.is_empty() => break,
            Ok(chunk) => body.extend_from_slice(&chunk),
            Err(wasi::io::streams::StreamError::Closed) => break,
            Err(_) => { stream.subscribe().block(); match stream.read(65536) { Ok(c) if c.is_empty() => break, Ok(c) => body.extend_from_slice(&c), _ => break } }
        }
    }
    drop(stream);
    let _ = IncomingBody::finish(req_body);
    body
}

fn respond_json(response_out: ResponseOutparam, status: u16, body: &str) {
    let headers = Fields::new();
    headers.append("content-type", &b"application/json"[..]).unwrap();
    let response = OutgoingResponse::new(headers);
    response.set_status_code(status).unwrap();
    let out = response.body().unwrap();
    let s = out.write().unwrap();
    s.blocking_write_and_flush(body.as_bytes()).unwrap();
    drop(s);
    OutgoingBody::finish(out, None).unwrap();
    ResponseOutparam::set(response_out, Ok(response));
}

export!(ToolsWorker);
