wit_bindgen::generate!({
    path: "wit",
    world: "web-ui",
    pub_export_macro: true,
    generate_all,
});

use exports::wasi::http::incoming_handler::Guest;
use wasi::http::outgoing_handler;
use wasi::http::types::*;

struct WebUi;

impl Guest for WebUi {
    fn handle(request: IncomingRequest, response_out: ResponseOutparam) {
        let method = request.method();
        let path = request.path_with_query().unwrap_or_default();

        // Route
        match (method, path.as_str()) {
            (Method::Get, "/") | (Method::Get, "/index.html") => {
                serve_dashboard(response_out);
            }
            (Method::Get, p) if p.starts_with("/api/models") => {
                api_models(response_out);
            }
            (Method::Get, p) if p.starts_with("/api/health") => {
                respond_json(response_out, 200, r#"{"status":"ok"}"#);
            }
            (Method::Get, p) if p.starts_with("/api/events") => {
                api_events(response_out);
            }
            (Method::Get, p) if p.starts_with("/api/runs/") => {
                let project = p.strip_prefix("/api/runs/").unwrap_or("default");
                api_runs(response_out, project);
            }
            (Method::Get, p) if p.starts_with("/api/metrics/") => {
                let project = p.strip_prefix("/api/metrics/").unwrap_or("default");
                api_metrics(response_out, project);
            }
            (Method::Post, "/api/run") => {
                let body = read_body(&request);
                api_submit_run(response_out, &body);
            }
            _ => {
                respond_json(response_out, 404, r#"{"error":"not found"}"#);
            }
        }
    }
}

fn read_body(request: &IncomingRequest) -> Vec<u8> {
    let body = match request.consume() {
        Ok(b) => b,
        Err(_) => return Vec::new(),
    };
    let stream = match body.stream() {
        Ok(s) => s,
        Err(_) => return Vec::new(),
    };
    let mut bytes = Vec::new();
    loop {
        match stream.read(65536) {
            Ok(chunk) if chunk.is_empty() => break,
            Ok(chunk) => bytes.extend_from_slice(&chunk),
            Err(_) => break,
        }
    }
    drop(stream);
    let _ = IncomingBody::finish(body);
    bytes
}

fn api_models(response_out: ResponseOutparam) {
    let ollama_url = "http://localhost:11434";

    // Call Ollama /api/tags
    match http_get(&format!("{}/api/tags", ollama_url)) {
        Ok(body) => {
            // Parse and reformat
            let parsed: serde_json::Value = serde_json::from_str(&body).unwrap_or_default();
            let models = parsed.get("models").cloned().unwrap_or(serde_json::Value::Array(vec![]));
            respond_json(response_out, 200, &serde_json::to_string(&models).unwrap_or_default());
        }
        Err(e) => {
            respond_json(response_out, 502, &format!(r#"{{"error":"ollama: {}"}}"#, e));
        }
    }
}

fn api_events(response_out: ResponseOutparam) {
    // SSE: return text/event-stream with current status
    // For Phase 1, return a single snapshot event then close
    // Full SSE streaming requires long-lived connections (Phase 2)
    let headers = Fields::new();
    headers.append(&"content-type".to_string(), &b"text/event-stream"[..]).unwrap();
    headers.append(&"cache-control".to_string(), &b"no-cache"[..]).unwrap();

    let response = OutgoingResponse::new(headers);
    response.set_status_code(200).unwrap();

    let body = response.body().unwrap();
    let stream = body.write().unwrap();

    let event = "event: status\ndata: {\"active_agents\":0,\"message\":\"web-ui running\"}\n\n";
    stream.blocking_write_and_flush(event.as_bytes()).unwrap();

    drop(stream);
    OutgoingBody::finish(body, None).unwrap();
    ResponseOutparam::set(response_out, Ok(response));
}

fn surreal_query(query: &str) -> Result<String, String> {
    let surreal_url = "http://127.0.0.1:8001";
    let headers = Fields::new();
    headers.append(&"content-type".to_string(), &b"application/json"[..]).map_err(|e| format!("{e:?}"))?;
    headers.append(&"accept".to_string(), &b"application/json"[..]).map_err(|e| format!("{e:?}"))?;
    headers.append(&"surreal-ns".to_string(), &b"alpha_swarm"[..]).map_err(|e| format!("{e:?}"))?;
    headers.append(&"surreal-db".to_string(), &b"swarm"[..]).map_err(|e| format!("{e:?}"))?;
    // Basic auth: root:root
    headers.append(&"authorization".to_string(), &b"Basic cm9vdDpyb290"[..]).map_err(|e| format!("{e:?}"))?;

    let request = OutgoingRequest::new(headers);
    request.set_method(&Method::Post).map_err(|_| "method")?;
    request.set_scheme(Some(&Scheme::Http)).map_err(|_| "scheme")?;
    request.set_authority(Some("127.0.0.1:8001")).map_err(|_| "authority")?;
    request.set_path_with_query(Some("/sql")).map_err(|_| "path")?;

    let out_body = request.body().map_err(|_| "body")?;
    let out_stream = out_body.write().map_err(|_| "stream")?;
    out_stream.blocking_write_and_flush(query.as_bytes()).map_err(|e| format!("write: {e:?}"))?;
    drop(out_stream);
    OutgoingBody::finish(out_body, None).map_err(|_| "finish")?;

    let future_response = outgoing_handler::handle(request, None).map_err(|e| format!("send: {e:?}"))?;
    let pollable = future_response.subscribe();
    pollable.block();

    let response = future_response.get()
        .ok_or("no response")?
        .map_err(|_| "response error")?
        .map_err(|e| format!("http: {e:?}"))?;

    let resp_body = response.consume().map_err(|_| "consume")?;
    let resp_stream = resp_body.stream().map_err(|_| "stream")?;
    let mut bytes = Vec::new();
    loop {
        match resp_stream.read(65536) {
            Ok(chunk) if chunk.is_empty() => break,
            Ok(chunk) => bytes.extend_from_slice(&chunk),
            Err(_) => break,
        }
    }
    drop(resp_stream);
    let _ = IncomingBody::finish(resp_body);

    String::from_utf8(bytes).map_err(|e| format!("utf8: {e}"))
}

fn api_runs(response_out: ResponseOutparam, project: &str) {
    let query = format!(
        "SELECT * FROM agent_run WHERE project = '{}' ORDER BY created_at DESC LIMIT 50",
        project.replace('\'', "")
    );
    match surreal_query(&query) {
        Ok(body) => {
            // SurrealDB returns [{result: [...], ...}], extract the result array
            let parsed: serde_json::Value = serde_json::from_str(&body).unwrap_or_default();
            let runs = parsed.get(0)
                .and_then(|r| r.get("result"))
                .cloned()
                .unwrap_or(serde_json::Value::Array(vec![]));
            respond_json(response_out, 200, &serde_json::to_string(&runs).unwrap_or_default());
        }
        Err(e) => respond_json(response_out, 502, &format!(r#"{{"error":"surrealdb: {}"}}"#, e)),
    }
}

fn api_metrics(response_out: ResponseOutparam, project: &str) {
    let query = format!(
        "SELECT status, model_used, tokens_input, tokens_output, duration_ms FROM agent_run WHERE project = '{}'",
        project.replace('\'', "")
    );
    match surreal_query(&query) {
        Ok(body) => {
            let parsed: serde_json::Value = serde_json::from_str(&body).unwrap_or_default();
            let runs = parsed.get(0)
                .and_then(|r| r.get("result"))
                .and_then(|r| r.as_array())
                .cloned()
                .unwrap_or_default();

            let total = runs.len();
            let passed = runs.iter().filter(|r| r.get("status").and_then(|s| s.as_str()) == Some("passed")).count();
            let failed = runs.iter().filter(|r| r.get("status").and_then(|s| s.as_str()) == Some("failed")).count();
            let pass_rate = if total > 0 { passed as f64 / total as f64 } else { 0.0 };
            let total_tokens: u64 = runs.iter()
                .filter_map(|r| r.get("tokens_output").and_then(|t| t.as_u64()))
                .sum();
            let avg_duration: u64 = if total > 0 {
                runs.iter()
                    .filter_map(|r| r.get("duration_ms").and_then(|d| d.as_u64()))
                    .sum::<u64>() / total as u64
            } else { 0 };

            let resp = format!(
                r#"{{"total_runs":{},"passed":{},"failed":{},"pass_rate":{:.2},"total_tokens_output":{},"avg_duration_ms":{}}}"#,
                total, passed, failed, pass_rate, total_tokens, avg_duration
            );
            respond_json(response_out, 200, &resp);
        }
        Err(e) => respond_json(response_out, 502, &format!(r#"{{"error":"surrealdb: {}"}}"#, e)),
    }
}

const SYSTEM_PROMPT: &str = "You are a code modification agent. You receive a task and files, then output precise edits.\n\nOUTPUT FORMAT:\n<<<EDIT path/to/file\n--- OLD\nexact lines to replace\n--- NEW\nreplacement lines\n>>>\n\nOutput ONLY edit blocks.";

fn api_submit_run(response_out: ResponseOutparam, body: &[u8]) {
    let body_str = String::from_utf8_lossy(body);
    let parsed: serde_json::Value = match serde_json::from_str(&body_str) {
        Ok(v) => v,
        Err(e) => {
            respond_json(response_out, 400, &format!(r#"{{"error":"invalid json: {}"}}"#, e));
            return;
        }
    };

    let task = parsed.get("task").and_then(|t| t.as_str()).unwrap_or("");
    let ollama_url = parsed.get("ollama_url").and_then(|u| u.as_str()).unwrap_or("http://localhost:11434");
    let model = parsed.get("model").and_then(|m| m.as_str()).unwrap_or("qwen2.5-coder:7b");
    let project = parsed.get("project").and_then(|p| p.as_str()).unwrap_or("default");
    let files = parsed.get("files").and_then(|f| f.as_array()).cloned().unwrap_or_default();

    // Build file context
    let mut file_context = String::new();
    for f in &files {
        let path = f.get("path").and_then(|p| p.as_str()).unwrap_or("");
        let content = f.get("content").and_then(|c| c.as_str()).unwrap_or("");
        file_context.push_str(&format!("=== {} ===\n{}\n\n", path, content));
    }
    let user_msg = format!("TASK: {}\n\nFILES:\n{}", task, file_context);

    // Build Ollama chat request
    let system_escaped = SYSTEM_PROMPT.replace('\\', "\\\\").replace('"', "\\\"").replace('\n', "\\n");
    let user_escaped = user_msg.replace('\\', "\\\\").replace('"', "\\\"").replace('\n', "\\n");
    let chat_body = format!(
        r#"{{"model":"{}","messages":[{{"role":"system","content":"{}"}},{{"role":"user","content":"{}"}}],"stream":false}}"#,
        model, system_escaped, user_escaped
    );

    // Call Ollama
    let start = std::time::SystemTime::now();
    let ollama_result = http_post(&format!("{}/api/chat", ollama_url), &chat_body);
    let duration_ms = start.elapsed().map(|d| d.as_millis() as u64).unwrap_or(0);

    match ollama_result {
        Ok(resp_body) => {
            let resp_json: serde_json::Value = serde_json::from_str(&resp_body).unwrap_or_default();
            let content = resp_json["message"]["content"].as_str().unwrap_or("");
            let tokens_in = resp_json["prompt_eval_count"].as_u64().unwrap_or(0);
            let tokens_out = resp_json["eval_count"].as_u64().unwrap_or(0);

            // Parse edits
            let edits = edit_parser::parse_edits(content).unwrap_or_default();
            let edit_count = edits.len();

            // Apply edits to files
            let mut modified_files = Vec::new();
            for edit in &edits {
                if let edit_parser::FileEdit::Edit { path, old, new } = edit {
                    if let Some(f) = files.iter().find(|f| f.get("path").and_then(|p| p.as_str()) == Some(path)) {
                        let orig = f.get("content").and_then(|c| c.as_str()).unwrap_or("");
                        let updated = orig.replacen(old.as_str(), new.as_str(), 1);
                        modified_files.push(serde_json::json!({"path": path, "content": updated}));
                    }
                }
            }

            let status = if edit_count > 0 { "passed" } else { "skipped" };

            // Store in SurrealDB
            let store_query = format!(
                "CREATE agent_run SET project='{}', task_description='{}', agent_id='web-ui', model_used='{}', status='{}', tokens_input={}, tokens_output={}, duration_ms={}, created_at=time::now(), files_modified=[]",
                project.replace('\'', ""),
                task.replace('\'', "").chars().take(200).collect::<String>(),
                model.replace('\'', ""),
                status,
                tokens_in,
                tokens_out,
                duration_ms,
            );
            let _ = surreal_query(&store_query);

            let resp = serde_json::json!({
                "status": status,
                "model": model,
                "edits": edit_count,
                "tokens_input": tokens_in,
                "tokens_output": tokens_out,
                "duration_ms": duration_ms,
                "modified_files": modified_files,
            });
            respond_json(response_out, 200, &resp.to_string());
        }
        Err(e) => {
            // Store failure
            let store_query = format!(
                "CREATE agent_run SET project='{}', task_description='{}', agent_id='web-ui', model_used='{}', status='failed', error_message='{}', duration_ms={}, created_at=time::now(), tokens_input=0, tokens_output=0, files_modified=[]",
                project.replace('\'', ""),
                task.replace('\'', "").chars().take(200).collect::<String>(),
                model.replace('\'', ""),
                e.replace('\'', "").chars().take(200).collect::<String>(),
                duration_ms,
            );
            let _ = surreal_query(&store_query);

            respond_json(response_out, 500, &format!(r#"{{"error":"inference failed: {}"}}"#, e.replace('"', "'")));
        }
    }
}

fn http_post(url: &str, body: &str) -> Result<String, String> {
    let headers = Fields::new();
    headers.append(&"content-type".to_string(), &b"application/json"[..]).map_err(|e| format!("{e:?}"))?;

    let request = OutgoingRequest::new(headers);
    request.set_method(&Method::Post).map_err(|_| "method")?;
    request.set_scheme(Some(&Scheme::Http)).map_err(|_| "scheme")?;

    let url_stripped = url.strip_prefix("http://").unwrap_or(url);
    let (authority, path) = url_stripped.split_once('/').unwrap_or((url_stripped, ""));
    request.set_authority(Some(authority)).map_err(|_| "authority")?;
    request.set_path_with_query(Some(&format!("/{}", path))).map_err(|_| "path")?;

    let out_body = request.body().map_err(|_| "body")?;
    let out_stream = out_body.write().map_err(|_| "stream")?;
    out_stream.blocking_write_and_flush(body.as_bytes()).map_err(|e| format!("write: {e:?}"))?;
    drop(out_stream);
    OutgoingBody::finish(out_body, None).map_err(|_| "finish")?;

    let future_response = outgoing_handler::handle(request, None).map_err(|e| format!("send: {e:?}"))?;
    let pollable = future_response.subscribe();
    pollable.block();

    let response = future_response.get()
        .ok_or("no response")?
        .map_err(|_| "response error")?
        .map_err(|e| format!("http: {e:?}"))?;

    let status = response.status();
    let resp_body = response.consume().map_err(|_| "consume")?;
    let resp_stream = resp_body.stream().map_err(|_| "stream")?;
    let mut bytes = Vec::new();
    loop {
        match resp_stream.read(65536) {
            Ok(chunk) if chunk.is_empty() => break,
            Ok(chunk) => bytes.extend_from_slice(&chunk),
            Err(_) => break,
        }
    }
    drop(resp_stream);
    let _ = IncomingBody::finish(resp_body);

    if status != 200 {
        return Err(format!("HTTP {}: {}", status, String::from_utf8_lossy(&bytes)));
    }
    String::from_utf8(bytes).map_err(|e| format!("utf8: {e}"))
}

fn http_get(url: &str) -> Result<String, String> {
    let headers = Fields::new();
    let request = OutgoingRequest::new(headers);
    request.set_method(&Method::Get).map_err(|_| "set method")?;
    request.set_scheme(Some(&Scheme::Http)).map_err(|_| "set scheme")?;

    let url_stripped = url.strip_prefix("http://").unwrap_or(url);
    let (authority, path) = url_stripped.split_once('/').unwrap_or((url_stripped, ""));
    request.set_authority(Some(authority)).map_err(|_| "set authority")?;
    request.set_path_with_query(Some(&format!("/{}", path))).map_err(|_| "set path")?;

    let out_body = request.body().map_err(|_| "get body")?;
    OutgoingBody::finish(out_body, None).map_err(|_| "finish body")?;

    let future_response = outgoing_handler::handle(request, None).map_err(|e| format!("send: {e:?}"))?;
    let pollable = future_response.subscribe();
    pollable.block();

    let response = future_response.get()
        .ok_or("no response")?
        .map_err(|_| "response error")?
        .map_err(|e| format!("http: {e:?}"))?;

    let resp_body = response.consume().map_err(|_| "consume")?;
    let resp_stream = resp_body.stream().map_err(|_| "stream")?;
    let mut bytes = Vec::new();
    loop {
        match resp_stream.read(65536) {
            Ok(chunk) if chunk.is_empty() => break,
            Ok(chunk) => bytes.extend_from_slice(&chunk),
            Err(_) => break,
        }
    }
    drop(resp_stream);
    let _ = IncomingBody::finish(resp_body);

    String::from_utf8(bytes).map_err(|e| format!("utf8: {e}"))
}

fn serve_dashboard(response_out: ResponseOutparam) {
    let html = include_str!("../static/index.html");
    let headers = Fields::new();
    headers.append(&"content-type".to_string(), &b"text/html; charset=utf-8"[..]).unwrap();
    let response = OutgoingResponse::new(headers);
    response.set_status_code(200).unwrap();
    let body = response.body().unwrap();

    // Set response FIRST (sends headers), then write body
    ResponseOutparam::set(response_out, Ok(response));

    let stream = body.write().unwrap();
    let bytes = html.as_bytes();
    let mut offset = 0;
    while offset < bytes.len() {
        let capacity = stream.check_write().unwrap_or(0) as usize;
        if capacity == 0 {
            stream.subscribe().block();
            continue;
        }
        let end = (offset + capacity).min(bytes.len());
        stream.write(&bytes[offset..end]).unwrap();
        offset = end;
    }
    stream.flush().unwrap();
    stream.subscribe().block();
    drop(stream);
    OutgoingBody::finish(body, None).unwrap();
}

fn serve_static(response_out: ResponseOutparam, content_type: &str, data: &[u8]) {
    let headers = Fields::new();
    headers.append(&"content-type".to_string(), content_type.as_bytes()).unwrap();

    let response = OutgoingResponse::new(headers);
    response.set_status_code(200).unwrap();

    let body = response.body().unwrap();
    let stream = body.write().unwrap();

    // Write in chunks to avoid exceeding buffer limits
    for chunk in data.chunks(4096) {
        stream.blocking_write_and_flush(chunk).unwrap();
    }

    drop(stream);
    OutgoingBody::finish(body, None).unwrap();
    ResponseOutparam::set(response_out, Ok(response));
}

fn respond_json(response_out: ResponseOutparam, status: u16, body: &str) {
    let headers = Fields::new();
    headers.append(&"content-type".to_string(), &b"application/json"[..]).unwrap();
    let response = OutgoingResponse::new(headers);
    response.set_status_code(status).unwrap();
    let out_body = response.body().unwrap();
    let stream = out_body.write().unwrap();
    stream.blocking_write_and_flush(body.as_bytes()).unwrap();
    drop(stream);
    OutgoingBody::finish(out_body, None).unwrap();
    ResponseOutparam::set(response_out, Ok(response));
}

export!(WebUi);
