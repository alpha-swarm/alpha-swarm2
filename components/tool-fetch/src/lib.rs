//! Sandboxed HTTP fetch tool component (Wassette).
//!
//! Ports the native `fetch_url` tool to WASM. Makes the request via
//! `wasi:http/outgoing-handler`; Wassette enforces a per-host network
//! allowlist (deny-by-default), so this is a capability-scoped fetch.

wit_bindgen::generate!({
    path: "wit",
    world: "fetch",
    generate_all,
});

use wasi::http::outgoing_handler;
use wasi::http::types::*;

/// Max response bytes returned to the agent.
const MAX_BODY: usize = 200_000;

struct Component;
export!(Component);

impl Guest for Component {
    fn fetch_url(url: String) -> Result<String, String> {
        let (scheme, rest) = if let Some(r) = url.strip_prefix("https://") {
            (Scheme::Https, r)
        } else if let Some(r) = url.strip_prefix("http://") {
            (Scheme::Http, r)
        } else {
            return Err("url must start with http:// or https://".into());
        };
        let (authority, path) = match rest.split_once('/') {
            Some((a, p)) => (a.to_string(), format!("/{p}")),
            None => (rest.to_string(), "/".to_string()),
        };

        let request = OutgoingRequest::new(Fields::new());
        request.set_method(&Method::Get).map_err(|_| "set method")?;
        request.set_scheme(Some(&scheme)).map_err(|_| "set scheme")?;
        request.set_authority(Some(&authority)).map_err(|_| "set authority")?;
        request.set_path_with_query(Some(&path)).map_err(|_| "set path")?;

        // GET: no request body.
        let out_body = request.body().map_err(|_| "body")?;
        OutgoingBody::finish(out_body, None).map_err(|_| "finish body")?;

        let future = outgoing_handler::handle(request, None).map_err(|e| format!("send: {e:?}"))?;
        future.subscribe().block();
        let response = future
            .get()
            .ok_or("no response")?
            .map_err(|_| "response taken")?
            .map_err(|e| format!("http error: {e:?}"))?;

        let status = response.status();
        let body = response.consume().map_err(|_| "consume")?;
        let stream = body.stream().map_err(|_| "stream")?;
        let mut bytes = Vec::new();
        while let Ok(chunk) = stream.read(65536) {
            if chunk.is_empty() {
                break;
            }
            bytes.extend_from_slice(&chunk);
            if bytes.len() > MAX_BODY {
                break;
            }
        }
        drop(stream);
        let _ = IncomingBody::finish(body);

        let text = String::from_utf8_lossy(&bytes).to_string();
        if !(200..300).contains(&status) {
            return Err(format!("HTTP {status}: {}", text.chars().take(200).collect::<String>()));
        }
        Ok(if text.len() > MAX_BODY { text[..MAX_BODY].to_string() } else { text })
    }
}
