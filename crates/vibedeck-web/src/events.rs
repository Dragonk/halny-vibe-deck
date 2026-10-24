//! The engine event stream: a fullstack WebSocket that forwards every
//! BusEvent (session updates, streaming chunks) to the browser/webview.

use dioxus::{fullstack::JsonEncoding, prelude::*};
use dioxus_fullstack::{UseWebsocket, WebSocketOptions, Websocket, use_websocket};
use vibedeck_core::snapshot::BusEvent;

/// Server side: subscribe to the engine bus and stream to the client.
#[get("/api/events?token")]
async fn events_ws(
    token: String,
    options: WebSocketOptions,
) -> Result<Websocket<(), BusEvent, JsonEncoding>, ServerFnError> {
    let engine = vibedeck_engine::Engine::global();
    if engine.auth_enabled() && !engine.check_token(&token) {
        return Err(ServerFnError::new("unauthorized"));
    }
    let mut rx = engine.subscribe();
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

pub fn use_engine_socket(token: String) -> EngineSocket {
    use_websocket(move || events_ws(token.clone(), WebSocketOptions::new()))
}
