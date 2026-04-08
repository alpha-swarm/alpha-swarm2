wit_bindgen::generate!({
    path: "wit",
    world: "agent-worker",
    pub_export_macro: true,
    generate_all,
});

use exports::wasi::http::incoming_handler::Guest;
use wasi::http::outgoing_handler;
use wasi::http::types::*;

struct AgentWorker;

// --- Ollama HTTP client via WASI ---

fn ollama_chat(ollama_url: &str, model: &str, messages: &str) -> Result<String, String> {
    let body_json = format!(
        r#"{{"model":"{}","messages":{},"stream":false}}"#,
        model, messages
    );

    let url = format!("{}/api/chat", ollama_url);

    // Create outgoing request
    let headers = Fields::new();
    headers.append("content-type", &b"application/json"[..]).map_err(|e| format!("header: {e:?}"))?;

    let request = OutgoingRequest::new(headers);
    request.set_method(&Method::Post).map_err(|_| "set method")?;
    request.set_scheme(Some(&Scheme::Http)).map_err(|_| "set scheme")?;

    // Parse URL for authority and path
    let url_stripped = url.strip_prefix("http://").unwrap_or(&url);
    let (authority, path) = url_stripped.split_once('/').unwrap_or((url_stripped, "api/chat"));
    request.set_authority(Some(authority)).map_err(|_| "set authority")?;
    request.set_path_with_query(Some(&format!("/{}", path))).map_err(|_| "set path")?;

    // Write body
    let out_body = request.body().map_err(|_| "get body")?;
    let out_stream = out_body.write().map_err(|_| "get stream")?;
    out_stream.blocking_write_and_flush(body_json.as_bytes()).map_err(|e| format!("write: {e:?}"))?;
    drop(out_stream);
    OutgoingBody::finish(out_body, None).map_err(|_| "finish body")?;

    // Send request
    let future_response = outgoing_handler::handle(request, None)
        .map_err(|e| format!("send: {e:?}"))?;

    // Poll for response
    let pollable = future_response.subscribe();
    pollable.block();

    let response = future_response.get()
        .ok_or("no response")?
        .map_err(|_| "response error")?
        .map_err(|e| format!("http error: {e:?}"))?;

    let status = response.status();
    let resp_body = response.consume().map_err(|_| "consume body")?;
    let resp_stream = resp_body.stream().map_err(|_| "get stream")?;

    // Read response body
    let mut body_bytes = Vec::new();
    while let Ok(chunk) = resp_stream.read(65536) {
        if chunk.is_empty() { break; }
        body_bytes.extend_from_slice(&chunk);
    }
    drop(resp_stream);
    let _ = IncomingBody::finish(resp_body);

    if status != 200 {
        return Err(format!("Ollama returned {}: {}", status, String::from_utf8_lossy(&body_bytes)));
    }

    let body_str = String::from_utf8(body_bytes).map_err(|e| format!("utf8: {e}"))?;

    // Parse Ollama response JSON with serde
    let parsed: serde_json::Value = serde_json::from_str(&body_str)
        .map_err(|e| format!("json parse: {e}"))?;

    parsed["message"]["content"]
        .as_str()
        .map(String::from)
        .ok_or_else(|| format!("no content in response: {}", &body_str[..body_str.len().min(200)]))
}

use edit_parser::FileEdit;

// --- Request/Response types ---

#[derive(serde::Deserialize)]
struct TaskRequest {
    task: String,
    #[serde(default = "default_model")]
    model: String,
    #[serde(default = "default_ollama_url")]
    ollama_url: String,
    #[serde(default)]
    files: Vec<FileInput>,
}

#[derive(serde::Deserialize)]
struct FileInput {
    path: String,
    content: String,
}

fn default_model() -> String { "qwen2.5-coder:7b".into() }
fn default_ollama_url() -> String { "http://localhost:11434".into() }

const SYSTEM_PROMPT: &str = r#"You are a code modification agent. You receive a task and files, then output precise edits.

OUTPUT FORMAT — for each file to modify:

<<<EDIT path/to/file
--- OLD
exact lines to replace
--- NEW
replacement lines
>>>

Output ONLY edit blocks. No explanation."#;

impl Guest for AgentWorker {
    fn handle(request: IncomingRequest, response_out: ResponseOutparam) {
        let method = request.method();

        // Health check on GET
        if matches!(method, Method::Get) {
            respond_json(response_out, 200, r#"{"status":"ok","component":"agent-worker"}"#);
            return;
        }

        // POST = run a task
        let req_body = request.consume().unwrap();
        let req_stream = req_body.stream().unwrap();
        let mut body_bytes = Vec::new();
        while let Ok(chunk) = req_stream.read(65536) {
            if chunk.is_empty() { break; }
            body_bytes.extend_from_slice(&chunk);
        }
        drop(req_stream);
        let _ = IncomingBody::finish(req_body);

        let body_str = String::from_utf8_lossy(&body_bytes);

        let task_req: TaskRequest = match serde_json::from_str(&body_str) {
            Ok(t) => t,
            Err(e) => {
                respond_json(response_out, 400, &format!(r#"{{"error":"invalid json: {}"}}"#, e));
                return;
            }
        };

        // Build prompt
        let mut file_context = String::new();
        for f in &task_req.files {
            file_context.push_str(&format!("=== {} ===\n{}\n\n", f.path, f.content));
        }
        let user_msg = format!("TASK: {}\n\nFILES:\n{}", task_req.task, file_context);

        // Build Ollama messages JSON
        let system_escaped = SYSTEM_PROMPT.replace('\\', "\\\\").replace('"', "\\\"").replace('\n', "\\n");
        let user_escaped = user_msg.replace('\\', "\\\\").replace('"', "\\\"").replace('\n', "\\n");
        let messages_json = format!(
            r#"[{{"role":"system","content":"{}"}},{{"role":"user","content":"{}"}}]"#,
            system_escaped, user_escaped
        );

        // Call Ollama
        let llm_result = ollama_chat(&task_req.ollama_url, &task_req.model, &messages_json);

        match llm_result {
            Ok(content) => {
                let edits = edit_parser::parse_edits(&content).unwrap_or_default();

                // Apply edits to files
                let mut modified_files = Vec::new();
                for edit in &edits {
                    if let FileEdit::Edit { path, old, new } = edit
                        && let Some(f) = task_req.files.iter().find(|f| f.path == *path) {
                            let updated = f.content.replacen(old.as_str(), new.as_str(), 1);
                            modified_files.push(format!(
                                r#"{{"path":"{}","content":{}}}"#,
                                path,
                                serde_json::to_string(&updated).unwrap_or_default()
                            ));
                    }
                }

                let resp = format!(
                    r#"{{"status":"ok","model":"{}","edits":{},"modified_files":[{}],"raw_response":{}}}"#,
                    task_req.model,
                    edits.len(),
                    modified_files.join(","),
                    serde_json::to_string(&content).unwrap_or_default()
                );
                respond_json(response_out, 200, &resp);
            }
            Err(e) => {
                respond_json(response_out, 500, &format!(r#"{{"error":"inference failed: {}"}}"#, e.replace('"', "'")));
            }
        }
    }
}

fn respond_json(response_out: ResponseOutparam, status: u16, body: &str) {
    let headers = Fields::new();
    headers.append("content-type", &b"application/json"[..]).unwrap();
    let response = OutgoingResponse::new(headers);
    response.set_status_code(status).unwrap();
    let out_body = response.body().unwrap();
    let stream = out_body.write().unwrap();
    stream.blocking_write_and_flush(body.as_bytes()).unwrap();
    drop(stream);
    OutgoingBody::finish(out_body, None).unwrap();
    ResponseOutparam::set(response_out, Ok(response));
}

export!(AgentWorker);
