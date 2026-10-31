//! The engine event stream: a fullstack WebSocket that forwards every
//! BusEvent (session updates, streaming chunks) to the browser/webview.

use dioxus::{fullstack::JsonEncoding, prelude::*};
use dioxus_fullstack::{UseWebsocket, WebSocketOptions, Websocket, use_websocket};
use halny_vibe_deck_core::snapshot::BusEvent;

/// Server side: subscribe to the engine bus and stream to the client.
/// No token check: the endpoint is reachable from the local 127.0.0.1
/// listener (trusted) or through the remote HTTPS listener, whose
/// middleware already validated the session cookie on the upgrade request.
#[get("/api/events")]
async fn events_ws(
    options: WebSocketOptions,
) -> Result<Websocket<(), BusEvent, JsonEncoding>, ServerFnError> {
    let mut rx = halny_vibe_deck_engine::Engine::global().subscribe();
    Ok(options.on_upgrade(move |mut socket| async move {
        loop {
            match rx.recv().await {
                Ok(ev) => {
                    if socket.send(ev).await.is_err() {
                        break;
                    }
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                Err(_) => break,
            }
        }
    }))
}

/// Client side: reactive handle for the event stream.
pub type EngineSocket = UseWebsocket<(), BusEvent, JsonEncoding>;

pub fn use_engine_socket() -> EngineSocket {
    use_websocket(move || events_ws(WebSocketOptions::new()))
}
