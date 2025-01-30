use std::net::SocketAddr;

use axum::{
    extract::{ws::WebSocket, ConnectInfo, State, WebSocketUpgrade},
    response::IntoResponse,
    routing::get,
    Router,
};
use futures_util::FutureExt;

use crate::{error::RrgError, state::AppState};

#[tracing::instrument]
async fn ws_handler(
    ws: WebSocketUpgrade,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    State(state): State<AppState>,
) -> impl IntoResponse {
    ws.on_upgrade(move |socket| {
        handle_socket(socket, addr, state).map(|res| {
            if let Err(e) = res {
                tracing::error!("Error in websocket: {:?}", e);
            }
        })
    })
}

#[tracing::instrument]
async fn handle_socket(
    mut socket: WebSocket,
    who: SocketAddr,
    state: AppState,
) -> Result<(), RrgError> {
    let mut rx = state.state_updates.subscribe();

    // Whenever a state update occurs ("/get" or "/submit")
    while let Ok(update) = rx.recv().await {
        // Re-request the list of pending waiters
        // TODO single waiter updates instead of sending entire list every time
        // And re-render the HTML list
        if socket.send(update.into()).await.is_err() {
            tracing::debug!("Socket disconnected with: {:?}", who);
            break;
        }
    }

    tracing::warn!("websocket connection closed");

    Ok(())
}

pub fn routes() -> Router<AppState> {
    Router::new().route("/", get(ws_handler))
}
