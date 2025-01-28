use std::net::SocketAddr;

use askama::Template;
use axum::{
    extract::{ws::WebSocket, ConnectInfo, State, WebSocketUpgrade},
    response::IntoResponse,
    routing::get,
    Router,
};
use futures_util::FutureExt;
use redis::AsyncCommands as _;
use uuid::Uuid;

use crate::{
    error::RrgError,
    state::{AppState, StateUpdate},
};

#[derive(Template)]
#[template(path = "index.html", block = "waitlist")]
struct ListFragment {
    pending_requests: Vec<Uuid>,
}

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
    let mut conn = state.redis.clone();
    let mut rx = state.state_updates.subscribe();

    while let Ok(update) = rx.recv().await {
        match update {
            // TODO do we still need separate added/removed?
            StateUpdate::Added(_guid) | StateUpdate::Removed(_guid) => {
                // TODO single waiter updates instead of sending entire list every time
                let pending_requests = conn.lrange("pending_callbacks", 0, -1).await?;
                if socket
                    .send(ListFragment { pending_requests }.render().unwrap().into())
                    .await
                    .is_err()
                {
                    tracing::debug!("Socket disconnected with: {:?}", who);
                    break;
                }
            }
        }
    }

    Ok(())
}

pub fn routes() -> Router<AppState> {
    Router::new().route("/", get(ws_handler))
}
