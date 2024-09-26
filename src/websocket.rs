use std::net::SocketAddr;

use askama::Template;
use axum::{
    extract::{ws::WebSocket, ConnectInfo, State, WebSocketUpgrade},
    response::IntoResponse,
    routing::get,
    Router,
};
use redis::Commands as _;
use uuid::Uuid;

use crate::message::StateUpdate;

#[derive(Template)]
#[template(path = "index.html", block = "list")]
struct ListFragment {
    pending_requests: Vec<Uuid>,
}

#[tracing::instrument]
async fn ws_handler(
    ws: WebSocketUpgrade,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    State(redis): State<redis::Client>,
) -> impl IntoResponse {
    ws.on_upgrade(move |socket| handle_socket(socket, addr, redis))
}

#[tracing::instrument]
async fn handle_socket(mut socket: WebSocket, who: SocketAddr, mut redis: redis::Client) {
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    let config = redis::AsyncConnectionConfig::new().set_push_sender(tx);
    let mut conn = redis
        .get_multiplexed_async_connection_with_config(&config)
        .await
        .unwrap();

    conn.subscribe("state_updates").await.unwrap();

    while let Some(msg) = rx.recv().await {
        if msg.kind == redis::PushKind::Message {
            let msg = redis::Msg::from_push_info(msg).unwrap();
            let msg = msg.get_payload_bytes();
            let update: StateUpdate = serde_json::from_slice(msg).unwrap();

            match update {
                // TODO do we still need separate added/removed?
                StateUpdate::Added(_guid) | StateUpdate::Removed(_guid) => {
                    // TODO using client directly vs getting (using existing?) connection
                    // TODO single waiter updates instead of sending entire list every time
                    let pending_requests: Vec<Uuid> = redis.lrange("callbacks", 0, -1).unwrap();
                    let res = socket
                        .send(ListFragment { pending_requests }.render().unwrap().into())
                        .await;
                    if let Err(e) = res {
                        tracing::debug!("Error sending websocket message: {e:?}");
                    }
                }
            }
        }
    }
}

pub fn routes() -> Router<redis::Client> {
    Router::new().route("/", get(ws_handler))
}
