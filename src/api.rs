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
use uuid::Uuid;

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

    // TODO we "need" to remove the callback if the request is cancelled by the user

    let random_number = redis::Msg::from_push_info(rx.recv().await.unwrap()).unwrap();

    (StatusCode::OK, random_number.get_payload_bytes().to_owned())
}

pub fn routes() -> Router<redis::Client> {
    Router::new()
        .route("/get_random", get(get_random))
        .route("/submit_random", post(submit_random))
}
