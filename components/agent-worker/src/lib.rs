wit_bindgen::generate!({
    path: "wit",
    world: "agent-worker",
    pub_export_macro: true,
    generate_all,
});

use exports::wasi::http::incoming_handler::Guest;
use wasi::http::types::*;

struct AgentWorker;

impl Guest for AgentWorker {
    fn handle(request: IncomingRequest, response_out: ResponseOutparam) {
        let response = OutgoingResponse::new(Fields::new());
        response.set_status_code(200).unwrap();

        let body = response.body().unwrap();
        let stream = body.write().unwrap();

        let msg = b"{\"status\": \"agent-worker on wasmCloud 2.0\"}";
        stream.blocking_write_and_flush(msg).unwrap();
        drop(stream);
        OutgoingBody::finish(body, None).unwrap();

        ResponseOutparam::set(response_out, Ok(response));
    }
}

export!(AgentWorker);
