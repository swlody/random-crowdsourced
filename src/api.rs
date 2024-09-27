use std::time::{Duration, Instant};

use anyhow::Context as _;
use axum::{
    body::Body,
    extract::State,
    http::{HeaderMap, StatusCode},
    response::{IntoResponse as _, Response},
    routing::{get, post},
    Json, Router,
};
use redis::AsyncCommands as _;
use serde::Deserialize;
use tokio::time::timeout;
use uuid::Uuid;

use crate::{error::RrgError, message::StateUpdate};

#[derive(Deserialize, Debug)]
struct SubmitParams {
    random_number: String,
}

#[tracing::instrument]
async fn submit_random(
    State(redis): State<redis::Client>,
    Json(SubmitParams { random_number }): Json<SubmitParams>,
) -> Result<Response, RrgError> {
    let mut conn = redis.get_multiplexed_async_connection().await?;

    let guid: Option<Uuid> = conn.rpop("callbacks", None).await?;
    if let Some(guid) = guid {
        conn.publish::<_, _, ()>(guid, &random_number).await?;

        conn.publish::<_, _, ()>(
            "state_updates",
            serde_json::to_string(&StateUpdate::Removed(guid)).unwrap(),
        )
        .await?;

        // conn.incr::<_, _, ()>(&random_number, 1).await?;
    }

    Ok((StatusCode::OK, Body::empty()).into_response())
}

#[tracing::instrument]
async fn get_random(
    headers: HeaderMap,
    State(redis): State<redis::Client>,
) -> Result<Response, RrgError> {
    let guid = Uuid::parse_str(headers["x-request-id"].to_str().unwrap()).unwrap();

    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    let config = redis::AsyncConnectionConfig::new().set_push_sender(tx);
    let mut conn = redis
        .get_multiplexed_async_connection_with_config(&config)
        .await?;

    conn.subscribe(guid).await?;

    // TODO proper error handling - map most everything to internal server error
    conn.lpush::<_, _, ()>("callbacks", guid).await?;
    conn.publish::<_, _, ()>(
        "state_updates",
        serde_json::to_string(&StateUpdate::Added(guid))?,
    )
    .await?;

    let start_time = Instant::now();
    loop {
        if start_time.elapsed() > Duration::from_secs(30) {
            return Ok((StatusCode::REQUEST_TIMEOUT, String::new()).into_response());
        }

        if let Ok(Some(res)) = timeout(Duration::from_secs(30), rx.recv()).await {
            if res.kind == redis::PushKind::Message {
                let random_number = redis::Msg::from_push_info(res)
                    .context("Cannot convert push info to message")?;
                return Ok((
                    StatusCode::OK,
                    format!(
                        "{}\n",
                        std::str::from_utf8(random_number.get_payload_bytes())?
                    ),
                )
                    .into_response());
            }
        } else {
            conn.lrem::<_, _, ()>("callbacks", 1, guid).await?;

            conn.publish::<_, _, ()>(
                "state_updates",
                serde_json::to_string(&StateUpdate::Removed(guid)).unwrap(),
            )
            .await?;

            // TODO return with Connection::close header in response
            return Ok((StatusCode::REQUEST_TIMEOUT, String::new()).into_response());
        }
    }
}

pub fn routes() -> Router<redis::Client> {
    Router::new()
        .route("/get", get(get_random))
        .route("/submit", post(submit_random))
}
