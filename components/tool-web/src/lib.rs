wit_bindgen::generate!({
    path: "wit",
    world: "tool-web",
    pub_export_macro: true,
    generate_all,
});

use exports::wasi::http::incoming_handler::Guest;
use wasi::http::outgoing_handler;
use wasi::http::types::*;

use serde::{Deserialize, Serialize};

#[derive(Deserialize)]
struct ToolRequest {
    name: String,
    #[serde(default)]
    params_json: String,
}

#[derive(Serialize)]
struct ToolResponse {
    content: String,
    is_error: bool,
    duration_ms: u64,
}

/// Max characters to return from URL fetch.
const MAX_FETCH_CHARS: usize = 10_000;
/// Max search results.
const MAX_SEARCH_RESULTS: usize = 5;

struct Component;

export!(Component);

impl Guest for Component {
    fn handle(request: IncomingRequest, response_out: ResponseOutparam) {
        let method = request.method();
        if !matches!(method, Method::Post) {
            respond(response_out, 405, r#"{"error":"method not allowed"}"#);
            return;
        }

        let body = match read_body(&request) {
            Ok(b) => b,
            Err(e) => { respond(response_out, 400, &format!(r#"{{"error":"{e}"}}"#)); return; }
        };

        let req: ToolRequest = match serde_json::from_slice(&body) {
            Ok(r) => r,
            Err(e) => { respond(response_out, 400, &format!(r#"{{"error":"invalid json: {e}"}}"#)); return; }
        };

        let result = execute_tool(&req);
        let json = serde_json::to_string(&result).unwrap_or_else(|_| r#"{"content":"serialize error","is_error":true,"duration_ms":0}"#.into());
        respond(response_out, 200, &json);
    }
}

fn execute_tool(req: &ToolRequest) -> ToolResponse {
    let params: serde_json::Value = serde_json::from_str(&req.params_json)
        .unwrap_or(serde_json::Value::Object(Default::default()));

    match req.name.as_str() {
        "web_search" => tool_web_search(&params),
        "fetch_url" => tool_fetch_url(&params),
        "search_crates" => tool_search_crates(&params),
        _ => ToolResponse { content: format!("Unknown tool: {}", req.name), is_error: true, duration_ms: 0 },
    }
}

fn tool_web_search(params: &serde_json::Value) -> ToolResponse {
    let query = params.get("query").and_then(|q| q.as_str()).unwrap_or("");
    if query.is_empty() {
        return ToolResponse { content: "Missing 'query'".into(), is_error: true, duration_ms: 0 };
    }

    let url = format!("https://html.duckduckgo.com/html/?q={}", url_encode(query));
    match http_get(&url) {
        Ok(body) => {
            let results = parse_ddg_results(&body);
            if results.is_empty() {
                ToolResponse { content: format!("No results for '{query}'"), is_error: false, duration_ms: 0 }
            } else {
                let formatted: Vec<String> = results.iter().take(MAX_SEARCH_RESULTS)
                    .map(|(title, snippet, url)| format!("- {} ({})\n  {}", title, url, snippet))
                    .collect();
                ToolResponse { content: formatted.join("\n\n"), is_error: false, duration_ms: 0 }
            }
        }
        Err(e) => ToolResponse { content: format!("Search failed: {e}"), is_error: true, duration_ms: 0 },
    }
}

fn tool_fetch_url(params: &serde_json::Value) -> ToolResponse {
    let url = params.get("url").and_then(|u| u.as_str()).unwrap_or("");
    if url.is_empty() {
        return ToolResponse { content: "Missing 'url'".into(), is_error: true, duration_ms: 0 };
    }

    match http_get(url) {
        Ok(body) => {
            let text = strip_html(&body);
            let truncated = if text.len() > MAX_FETCH_CHARS {
                format!("{}...\n(truncated, {} chars)", &text[..MAX_FETCH_CHARS], text.len())
            } else {
                text
            };
            ToolResponse { content: truncated, is_error: false, duration_ms: 0 }
        }
        Err(e) => ToolResponse { content: format!("Fetch failed: {e}"), is_error: true, duration_ms: 0 },
    }
}

fn tool_search_crates(params: &serde_json::Value) -> ToolResponse {
    let query = params.get("query").and_then(|q| q.as_str()).unwrap_or("");
    if query.is_empty() {
        return ToolResponse { content: "Missing 'query'".into(), is_error: true, duration_ms: 0 };
    }

    let url = format!("https://crates.io/api/v1/crates?q={}&per_page={}", url_encode(query), MAX_SEARCH_RESULTS);
    match http_get(&url) {
        Ok(body) => {
            let parsed: serde_json::Value = serde_json::from_str(&body).unwrap_or_default();
            let crates = parsed.get("crates").and_then(|c| c.as_array());
            match crates {
                Some(arr) => {
                    let results: Vec<String> = arr.iter().map(|c| {
                        let name = c.get("name").and_then(|n| n.as_str()).unwrap_or("?");
                        let desc = c.get("description").and_then(|d| d.as_str()).unwrap_or("");
                        let ver = c.get("newest_version").and_then(|v| v.as_str()).unwrap_or("?");
                        format!("- {} v{}\n  {}", name, ver, desc)
                    }).collect();
                    ToolResponse { content: results.join("\n"), is_error: false, duration_ms: 0 }
                }
                None => ToolResponse { content: format!("No crates for '{query}'"), is_error: false, duration_ms: 0 },
            }
        }
        Err(e) => ToolResponse { content: format!("crates.io failed: {e}"), is_error: true, duration_ms: 0 },
    }
}

// --- WASI HTTP client ---

fn http_get(url: &str) -> Result<String, String> {
    let headers = Fields::new();
    headers.append("user-agent", &b"alpha-swarm/0.1"[..]).map_err(|e| format!("header: {e:?}"))?;

    let request = OutgoingRequest::new(headers);
    request.set_method(&Method::Get).map_err(|_| "set method")?;

    // Parse URL components
    let (scheme, authority, path) = parse_url(url)?;
    request.set_scheme(Some(&scheme)).map_err(|_| "set scheme")?;
    request.set_authority(Some(&authority)).map_err(|_| "set authority")?;
    request.set_path_with_query(Some(&path)).map_err(|_| "set path")?;

    let future = outgoing_handler::handle(request, None).map_err(|e| format!("handle: {e:?}"))?;

    // Poll for response
    let pollable = future.subscribe();
    pollable.block();
    let response = future.get()
        .ok_or("no response".to_string())?
        .map_err(|e| format!("response error: {e:?}"))?
        .map_err(|e| format!("http error: {e:?}"))?;

    let status = response.status();
    if status >= 400 {
        return Err(format!("HTTP {status}"));
    }

    let body = response.consume().map_err(|_| "consume")?;
    let stream = body.stream().map_err(|_| "stream")?;
    let mut bytes = Vec::new();
    loop {
        match stream.read(65536) {
            Ok(chunk) if chunk.is_empty() => break,
            Ok(chunk) => bytes.extend_from_slice(&chunk),
            Err(_) => break,
        }
    }

    String::from_utf8(bytes).map_err(|e| format!("utf8: {e}"))
}

fn parse_url(url: &str) -> Result<(Scheme, String, String), String> {
    let (scheme, rest) = if let Some(rest) = url.strip_prefix("https://") {
        (Scheme::Https, rest)
    } else if let Some(rest) = url.strip_prefix("http://") {
        (Scheme::Http, rest)
    } else {
        return Err(format!("unsupported URL scheme: {url}"));
    };

    let (authority, path) = match rest.find('/') {
        Some(i) => (&rest[..i], &rest[i..]),
        None => (rest, "/"),
    };

    Ok((scheme, authority.to_string(), path.to_string()))
}

// --- Helpers ---

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
    let stream = out_body.write().expect("write");
    write_all(&stream, body.as_bytes());
    drop(stream);
    OutgoingBody::finish(out_body, None).expect("finish");
}

fn write_all(stream: &wasi::io::streams::OutputStream, data: &[u8]) {
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

fn url_encode(s: &str) -> String {
    s.chars().map(|c| match c {
        ' ' => '+'.to_string(),
        c if c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.' => c.to_string(),
        c => format!("%{:02X}", c as u32),
    }).collect()
}

fn strip_html(html: &str) -> String {
    let mut result = String::new();
    let mut in_tag = false;
    for c in html.chars() {
        if c == '<' { in_tag = true; }
        else if c == '>' { in_tag = false; }
        else if !in_tag { result.push(c); }
    }
    // Collapse whitespace
    let mut collapsed = String::new();
    let mut last_ws = false;
    for c in result.chars() {
        if c.is_whitespace() {
            if !last_ws { collapsed.push(' '); }
            last_ws = true;
        } else { collapsed.push(c); last_ws = false; }
    }
    collapsed.trim().to_string()
}

fn parse_ddg_results(html: &str) -> Vec<(String, String, String)> {
    let mut results = Vec::new();
    for chunk in html.split("result__a") {
        if results.len() >= MAX_SEARCH_RESULTS { break; }
        let url = extract_attr(chunk, "href=\"");
        if url.is_empty() || url.starts_with('/') { continue; }
        let title = strip_html(&extract_tag_text(chunk));
        let snippet = if let Some(idx) = chunk.find("result__snippet") {
            strip_html(&extract_tag_text(&chunk[idx..]))
        } else { String::new() };
        if !title.is_empty() { results.push((title, snippet, url)); }
    }
    results
}

fn extract_attr(s: &str, attr: &str) -> String {
    let Some(idx) = s.find(attr) else { return String::new() };
    let rest = &s[idx + attr.len()..];
    let end = rest.find('"').unwrap_or(rest.len());
    rest[..end].to_string()
}

fn extract_tag_text(s: &str) -> String {
    let Some(gt) = s.find('>') else { return String::new() };
    let rest = &s[gt + 1..];
    let end = rest.find("</").unwrap_or(rest.len());
    rest[..end].to_string()
}
