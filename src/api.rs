use std::time::Duration;

use axum::{
    body::Body,
    extract::State,
    http::StatusCode,
    response::IntoResponse,
    routing::{get, post},
    Json, Router,
};
use redis::AsyncCommands as _;
use serde::Deserialize;
use tokio::time::timeout;
use uuid::Uuid;

use crate::message::StateUpdate;

#[derive(Deserialize, Debug)]
struct SubmitParams {
    random_number: String,
}

#[tracing::instrument]
async fn submit_random(
    State(redis): State<redis::Client>,
    Json(SubmitParams { random_number }): Json<SubmitParams>,
) -> impl IntoResponse {
    let mut conn = redis.get_multiplexed_async_connection().await.unwrap();

    let guid: Option<Uuid> = conn.rpop("callbacks", None).await.unwrap();
    if let Some(guid) = guid {
        let _: () = conn.publish(guid, random_number).await.unwrap();

        let _: () = conn
            .publish(
                "state_updates",
                serde_json::to_string(&StateUpdate::Removed(guid)).unwrap(),
            )
            .await
            .unwrap();
    }

    (StatusCode::OK, Body::empty())
}

#[tracing::instrument]
async fn get_random(State(redis): State<redis::Client>) -> impl IntoResponse {
    let guid = Uuid::now_v7();

    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    let config = redis::AsyncConnectionConfig::new().set_push_sender(tx);
    let mut conn = redis
        .get_multiplexed_async_connection_with_config(&config)
        .await
        .unwrap();

    conn.subscribe(guid).await.unwrap();
    rx.recv().await.unwrap();

    // TODO proper error handling
    let _: () = conn.lpush("callbacks", guid).await.unwrap();
    let _: () = conn
        .publish(
            "state_updates",
            serde_json::to_string(&StateUpdate::Added(guid)).unwrap(),
        )
        .await
        .unwrap();

    loop {
        if let Ok(res) = timeout(Duration::from_secs(30), rx.recv()).await {
            let res = res.unwrap();

            if res.kind == redis::PushKind::Message {
                let random_number = redis::Msg::from_push_info(res).unwrap();
                return (StatusCode::OK, random_number.get_payload_bytes().to_owned());
            }
        } else {
            let _: () = conn.lrem("callbacks", 1, guid).await.unwrap();

            let _: () = conn
                .publish(
                    "state_updates",
                    serde_json::to_string(&StateUpdate::Removed(guid)).unwrap(),
                )
                .await
                .unwrap();
            // TODO return with Connection::close header in response
            return (StatusCode::REQUEST_TIMEOUT, vec![]);
        }
    }
}

pub fn routes() -> Router<redis::Client> {
    Router::new()
        .route("/get_random", get(get_random))
        .route("/submit_random", post(submit_random))
}
