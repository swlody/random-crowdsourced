use std::net::SocketAddr;

use axum::{
    body::Bytes,
    extract::{
        ws::{Message, WebSocket},
        ConnectInfo, State, WebSocketUpgrade,
    },
    response::IntoResponse,
    routing::get,
    Router,
};
use futures_util::FutureExt;
use tokio::time::{timeout, Duration};

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
async fn heartbeat(socket: &mut WebSocket) -> bool {
    socket
        .send(Message::Ping(Bytes::from_static(b"heartbeat")))
        .await
        .is_ok()
}

#[tracing::instrument]
async fn handle_socket(
    mut socket: WebSocket,
    who: SocketAddr,
    state: AppState,
) -> Result<(), RrgError> {
    let mut rx = state.state_updates.subscribe();

    if !heartbeat(&mut socket).await {
        return Err(anyhow::anyhow!("Failed to ping new connection").into());
    }

    // Whenever a state update occurs ("/get" or "/submit")
    loop {
        match timeout(Duration::from_secs(5), rx.recv()).await {
            Ok(Ok(update)) => {
                if socket.send(update.into()).await.is_err() {
                    break;
                }
            }
            Ok(Err(e)) => return Err(RrgError::Other(e.into())),
            Err(_) => {
                if !heartbeat(&mut socket).await {
                    break;
                }
            }
        }
    }

    tracing::debug!("Socket disconnected with: {:?}", who);
    Ok(())
}

pub fn routes() -> Router<AppState> {
    Router::new().route("/", get(ws_handler))
}
