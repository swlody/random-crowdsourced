use core::panic;

use axum::{
    body::Body,
    extract::State,
    http::StatusCode,
    response::IntoResponse,
    routing::{get, post},
    Json, Router,
};
use redis::AsyncCommands;
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
    tracing::info!("Getting connection");
    let mut conn = redis.get_multiplexed_async_connection().await.unwrap();

    tracing::info!("Getting guid");
    let guid: Option<Uuid> = conn.rpop("callbacks", None).await.unwrap();

    if let Some(guid) = guid {
        tracing::info!("Got guid {guid}, publishing number");
        let _: () = conn.publish(guid, random_number).await.unwrap();
    } else {
        tracing::info!("No guid found");
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

    let confirmation = format!("{:?}", rx.recv().await.unwrap().data);
    tracing::info!("Got confirmation: {confirmation:?}");

    let _: () = conn.lpush("callbacks", guid).await.unwrap();

    let random_number = rx.recv().await.unwrap().data;
    tracing::info!("Got random number: {random_number:?}");
    let redis::Value::BulkString(random_number) = &random_number[1] else {
        panic!("Not a bulk string")
    };

    (
        StatusCode::OK,
        std::str::from_utf8(random_number).unwrap().to_owned(),
    )
}

pub fn routes() -> Router<redis::Client> {
    Router::new()
        .route("/get", get(get_random))
        .route("/submit", post(submit_random))
}
