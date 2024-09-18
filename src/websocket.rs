use std::{net::SocketAddr, sync::atomic::Ordering};

use axum::{
    extract::{ws::WebSocket, ConnectInfo, State, WebSocketUpgrade},
    response::IntoResponse,
    routing::get,
    Router,
};
use futures_util::{stream::StreamExt as _, SinkExt};

use crate::state::AppState;

#[tracing::instrument]
async fn ws_handler(
    ws: WebSocketUpgrade,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    State(state): State<AppState>,
) -> impl IntoResponse {
    ws.on_upgrade(move |socket| handle_socket(socket, addr, state))
}

#[tracing::instrument]
async fn handle_socket(socket: WebSocket, who: SocketAddr, state: AppState) {
    let (mut send, mut recv) = socket.split();

    let mut send_task = tokio::spawn(async move {
        // TODO
        loop {
            if state.waiting_calls.load(Ordering::Relaxed) == 0 {
                send.send("<div hx-swap-oob=\"innerHTML:#num_waiters\"></div>".into())
                    .await
                    .unwrap();
            } else {
                send.send(
                    "<div hx-swap-oob=\"innerHTML:#num_waiters\"><p>Someone needs a random \
                     number!</p></div>"
                        .into(),
                )
                .await
                .unwrap();
            }
            tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;
        }
    });

    let mut recv_task = tokio::spawn(async move {
        while let Some(Ok(msg)) = recv.next().await {
            tracing::debug!("Received: {:?}", msg);
            let msg = msg.into_text().unwrap();

            if let Ok(msg) = serde_json::from_str::<serde_json::Value>(&msg) {
                if let Some(Ok(random_number)) =
                    msg["random_number"].as_str().map(|s| s.parse::<i64>())
                {
                    tracing::debug!("Parsed message as i64, sending: {}", random_number);
                    state.tx.send(random_number).await.unwrap();
                } else {
                    tracing::error!("Unable to parse message as i64");
                }
            } else {
                tracing::error!("Unable to parse message as i64");
            }
        }
    });

    // If either task exits, abort the other
    tokio::select! {
        _ = (&mut send_task) => {
            recv_task.abort();
        }

        _ = (&mut recv_task) => {
            send_task.abort();
        }
    }
}

pub fn routes() -> Router<AppState> {
    Router::new().route("/", get(ws_handler))
}
