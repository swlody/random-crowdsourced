use std::net::SocketAddr;

use anyhow::Context as _;
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

use crate::{error::RrgError, message::StateUpdate, state::AppState};

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
                tracing::error!("Error in websocket: {}", e.0);
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
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    let config = redis::AsyncConnectionConfig::new().set_push_sender(tx);
    let mut conn = state
        .redis
        .get_multiplexed_async_connection_with_config(&config)
        .await?;

    conn.subscribe("state_updates").await?;

    while let Some(msg) = rx.recv().await {
        if msg.kind == redis::PushKind::Message {
            let msg = redis::Msg::from_push_info(msg)
                .context("Unable to convert push info to message")?;
            let msg = msg.get_payload_bytes();
            let update = serde_json::from_slice(msg).unwrap();

            match update {
                // TODO do we still need separate added/removed?
                StateUpdate::Added(_guid) | StateUpdate::Removed(_guid) => {
                    // TODO using client directly vs getting (using existing?) connection
                    // TODO single waiter updates instead of sending entire list every time
                    let pending_requests = conn.lrange("callbacks", 0, -1).await?;
                    socket
                        .send(ListFragment { pending_requests }.render().unwrap().into())
                        .await?;
                }
            }
        }
    }

    Ok(())
}

pub fn routes() -> Router<AppState> {
    Router::new().route("/", get(ws_handler))
}
