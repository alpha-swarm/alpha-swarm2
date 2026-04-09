wit_bindgen::generate!({
    path: "wit",
    world: "orchestrator-worker",
    pub_export_macro: true,
    generate_all,
});

use exports::wasi::http::incoming_handler::Guest;
use wasi::http::types::*;

struct OrchestratorWorker;

/// Handles goal decomposition requests.
/// POST /plan: takes {goal, files} → returns {tasks: [{id, description, files, complexity}]}
impl Guest for OrchestratorWorker {
    fn handle(request: IncomingRequest, response_out: ResponseOutparam) {
        let method = request.method();

        if matches!(method, Method::Get) {
            respond_json(response_out, 200, r#"{"status":"ok","component":"orchestrator-worker"}"#);
            return;
        }

        // Read body
        let body = read_body(&request);
        let body_str = String::from_utf8_lossy(&body);

        // Parse request
        let req: serde_json::Value = match serde_json::from_str(&body_str) {
            Ok(v) => v,
            Err(e) => {
                respond_json(response_out, 400, &format!(r#"{{"error":"{}"}}"#, e));
                return;
            }
        };

        let goal = req["goal"].as_str().unwrap_or("");
        let files: Vec<String> = req["files"].as_array()
            .map(|arr| arr.iter().filter_map(|v| v.as_str().map(String::from)).collect())
            .unwrap_or_default();

        // Use planner types to parse a plan response (if the request includes raw LLM output)
        if let Some(raw_plan) = req["raw_plan"].as_str() {
            match swarm_orchestrator::planner_types::parse_plan(raw_plan, &files) {
                Ok(tasks) => {
                    let resp = serde_json::to_string(&tasks).unwrap_or_default();
                    respond_json(response_out, 200, &format!(r#"{{"status":"ok","tasks":{resp}}}"#));
                }
                Err(e) => {
                    respond_json(response_out, 400, &format!(r#"{{"error":"plan parse: {e}"}}"#));
                }
            }
            return;
        }

        // Return the planner system prompt + goal for the caller to send to LLM
        let prompt = swarm_orchestrator::planner_types::PLANNER_SYSTEM;
        let file_list = files.join("\n");
        let user_msg = format!("GOAL: {goal}\n\nREPOSITORY FILES:\n{file_list}");

        let resp = serde_json::json!({
            "status": "ok",
            "system_prompt": prompt,
            "user_message": user_msg,
        });
        respond_json(response_out, 200, &resp.to_string());
    }
}

fn read_body(request: &IncomingRequest) -> Vec<u8> {
    let req_body = request.consume().unwrap();
    let req_stream = req_body.stream().unwrap();
    let mut body = Vec::new();
    loop {
        match req_stream.read(65536) {
            Ok(chunk) if chunk.is_empty() => break,
            Ok(chunk) => body.extend_from_slice(&chunk),
            Err(wasi::io::streams::StreamError::Closed) => break,
            Err(_) => {
                req_stream.subscribe().block();
                match req_stream.read(65536) {
                    Ok(chunk) if chunk.is_empty() => break,
                    Ok(chunk) => body.extend_from_slice(&chunk),
                    _ => break,
                }
            }
        }
    }
    drop(req_stream);
    let _ = IncomingBody::finish(req_body);
    body
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

export!(OrchestratorWorker);
