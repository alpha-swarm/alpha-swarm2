wit_bindgen::generate!({
    path: "wit",
    world: "knowledge-store",
    pub_export_macro: true,
    generate_all,
});

use exports::wasi::http::incoming_handler::Guest;
use wasi::http::types::*;
use wasi::http::outgoing_handler;

struct KnowledgeStoreWorker;

/// Knowledge store: CRUD for agent runs via SurrealDB REST API.
/// POST /query: {sql} → SurrealDB response
/// POST /run: {project, task, agent_id, ...} → create AgentRun
/// GET /runs?project=X → list runs
impl Guest for KnowledgeStoreWorker {
    fn handle(request: IncomingRequest, response_out: ResponseOutparam) {
        if matches!(request.method(), Method::Get) {
            respond_json(response_out, 200, r#"{"status":"ok","component":"knowledge-store","types":["AgentRun","RunStatus","GoalPlan","ToolCallRecord"]}"#);
            return;
        }

        let body = read_body(&request);
        let body_str = String::from_utf8_lossy(&body);

        let req: serde_json::Value = match serde_json::from_str(&body_str) {
            Ok(v) => v,
            Err(e) => { respond_json(response_out, 400, &format!(r#"{{"error":"{}"}}"#, e)); return; }
        };

        let action = req["action"].as_str().unwrap_or("query");
        let surrealdb_url = req["surrealdb_url"].as_str().unwrap_or("http://127.0.0.1:8001");

        match action {
            "create_run" => {
                let project = req["project"].as_str().unwrap_or("default");
                let task = req["task"].as_str().unwrap_or("");
                let agent_id = req["agent_id"].as_str().unwrap_or("agent");
                let run = knowledge_base::AgentRun::new(project, task, agent_id, "pending");
                let json = serde_json::to_string(&run).unwrap_or_default();
                respond_json(response_out, 200, &format!(r#"{{"status":"ok","run":{json}}}"#));
            }
            "query" => {
                let sql = req["sql"].as_str().unwrap_or("");
                match surreal_query(surrealdb_url, sql) {
                    Ok(result) => respond_json(response_out, 200, &result),
                    Err(e) => respond_json(response_out, 500, &format!(r#"{{"error":"{}"}}"#, e)),
                }
            }
            _ => respond_json(response_out, 400, &format!(r#"{{"error":"unknown action: {action}"}}"#)),
        }
    }
}

/// Query SurrealDB via wasi:http REST API.
fn surreal_query(url: &str, sql: &str) -> Result<String, String> {
    let headers = Fields::new();
    headers.append("content-type", &b"text/plain"[..]).map_err(|e| format!("{e:?}"))?;
    headers.append("accept", &b"application/json"[..]).map_err(|e| format!("{e:?}"))?;
    headers.append("surreal-ns", &b"alpha_swarm"[..]).map_err(|e| format!("{e:?}"))?;
    headers.append("surreal-db", &b"swarm"[..]).map_err(|e| format!("{e:?}"))?;
    headers.append("authorization", &b"Basic cm9vdDpyb290"[..]).map_err(|e| format!("{e:?}"))?; // root:root base64

    let request = OutgoingRequest::new(headers);
    request.set_method(&Method::Post).map_err(|_| "method")?;
    request.set_scheme(Some(&Scheme::Http)).map_err(|_| "scheme")?;

    let stripped = url.strip_prefix("http://").unwrap_or(url);
    let (authority, _) = stripped.split_once('/').unwrap_or((stripped, ""));
    request.set_authority(Some(authority)).map_err(|_| "authority")?;
    request.set_path_with_query(Some("/sql")).map_err(|_| "path")?;

    let out_body = request.body().map_err(|_| "body")?;
    let stream = out_body.write().map_err(|_| "stream")?;
    stream.blocking_write_and_flush(sql.as_bytes()).map_err(|e| format!("{e:?}"))?;
    drop(stream);
    OutgoingBody::finish(out_body, None).map_err(|_| "finish")?;

    let future = outgoing_handler::handle(request, None).map_err(|e| format!("{e:?}"))?;
    future.subscribe().block();

    let response = future.get().ok_or("no response")?.map_err(|_| "error")?.map_err(|e| format!("{e:?}"))?;
    let resp_body = response.consume().map_err(|_| "consume")?;
    let resp_stream = resp_body.stream().map_err(|_| "stream")?;

    let mut bytes = Vec::new();
    loop {
        match resp_stream.read(65536) {
            Ok(chunk) if chunk.is_empty() => break,
            Ok(chunk) => bytes.extend_from_slice(&chunk),
            _ => break,
        }
    }
    drop(resp_stream);
    let _ = IncomingBody::finish(resp_body);

    String::from_utf8(bytes).map_err(|e| format!("{e}"))
}

fn read_body(request: &IncomingRequest) -> Vec<u8> {
    let b = request.consume().unwrap();
    let s = b.stream().unwrap();
    let mut body = Vec::new();
    loop { match s.read(65536) { Ok(c) if c.is_empty() => break, Ok(c) => body.extend_from_slice(&c), Err(wasi::io::streams::StreamError::Closed) => break, Err(_) => { s.subscribe().block(); match s.read(65536) { Ok(c) if c.is_empty() => break, Ok(c) => body.extend_from_slice(&c), _ => break } } } }
    drop(s); let _ = IncomingBody::finish(b); body
}

fn respond_json(o: ResponseOutparam, status: u16, body: &str) {
    let h = Fields::new(); h.append("content-type", &b"application/json"[..]).unwrap();
    let r = OutgoingResponse::new(h); r.set_status_code(status).unwrap();
    let b = r.body().unwrap(); let s = b.write().unwrap();
    s.blocking_write_and_flush(body.as_bytes()).unwrap(); drop(s);
    OutgoingBody::finish(b, None).unwrap(); ResponseOutparam::set(o, Ok(r));
}

export!(KnowledgeStoreWorker);
