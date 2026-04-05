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
                // SSE endpoint — for now return a snapshot
                api_events(response_out);
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
