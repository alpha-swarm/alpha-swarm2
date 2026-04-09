wit_bindgen::generate!({
    path: "wit",
    world: "quality-gate-worker",
    pub_export_macro: true,
    generate_all,
});

use exports::wasi::http::incoming_handler::Guest;
use wasi::http::types::*;

struct QualityGateWorker;

/// Quality gate: detect toolchain and return check config.
/// POST /detect: {has_cargo_toml, has_package_json, has_go_mod} → ToolchainConfig
impl Guest for QualityGateWorker {
    fn handle(request: IncomingRequest, response_out: ResponseOutparam) {
        if matches!(request.method(), Method::Get) {
            respond_json(response_out, 200, r#"{"status":"ok","component":"quality-gate-worker"}"#);
            return;
        }

        let body = read_body(&request);
        let body_str = String::from_utf8_lossy(&body);

        let req: serde_json::Value = match serde_json::from_str(&body_str) {
            Ok(v) => v,
            Err(e) => { respond_json(response_out, 400, &format!(r#"{{"error":"{}"}}"#, e)); return; }
        };

        // Detect toolchain from file presence flags
        let config = if req["has_cargo_toml"].as_bool().unwrap_or(false) {
            quality_gate_lib::detect_toolchain(std::path::Path::new("/cargo"))
        } else if req["has_package_json"].as_bool().unwrap_or(false) {
            quality_gate_lib::detect_toolchain(std::path::Path::new("/node"))
        } else {
            quality_gate_lib::ToolchainConfig {
                build_cmd: None, fmt_cmd: None, lint_cmd: None,
                unit_test_cmd: None, integration_test_cmd: None, e2e_test_cmd: None,
            }
        };

        let resp = serde_json::to_string(&config).unwrap_or_default();
        respond_json(response_out, 200, &format!(r#"{{"status":"ok","config":{resp}}}"#));
    }
}

fn read_body(request: &IncomingRequest) -> Vec<u8> {
    let b = request.consume().unwrap(); let s = b.stream().unwrap();
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

export!(QualityGateWorker);
