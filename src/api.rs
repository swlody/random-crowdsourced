use std::time::Duration;

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

use crate::{
    error::RrgError,
    state::{AppState, StateUpdate},
};

#[derive(Deserialize, Debug)]
struct SubmitParams {
    random_number: String,
}

#[tracing::instrument]
async fn submit_random(
    State(state): State<AppState>,
    Json(SubmitParams { random_number }): Json<SubmitParams>,
) -> Result<Response, RrgError> {
    let mut conn = state.redis.get().await?;

    if let Some(guid) = conn.rpop("pending_callbacks", None).await? {
        conn.publish::<_, _, ()>(
            "callbacks",
            serde_json::to_string(&(guid, &random_number)).unwrap(),
        )
        .await?;

        conn.publish::<_, _, ()>(
            "state_updates",
            serde_json::to_string(&StateUpdate::Removed(guid)).unwrap(),
        )
        .await?;

        conn.zincr::<_, _, _, ()>("counts", &random_number, 1)
            .await?;
    }

    Ok((StatusCode::OK, Body::empty()).into_response())
}

#[tracing::instrument]
async fn get_random(
    headers: HeaderMap,
    State(state): State<AppState>,
) -> Result<Response, RrgError> {
    let guid = Uuid::parse_str(headers["x-request-id"].to_str().unwrap()).unwrap();

    let mut conn = state.redis.get().await?;

    let (tx, rx) = tokio::sync::oneshot::channel();
    state.callback_map.lock().unwrap().insert(guid, tx);
    conn.lpush::<_, _, ()>("pending_callbacks", guid).await?;
    conn.publish::<_, _, ()>(
        "state_updates",
        serde_json::to_string(&StateUpdate::Added(guid))?,
    )
    .await?;

    let callback_result = timeout(Duration::from_secs(30), rx).await;

    conn.lrem::<_, _, ()>("pending_callbacks", 1, guid).await?;
    conn.publish::<_, _, ()>(
        "state_updates",
        serde_json::to_string(&StateUpdate::Removed(guid)).unwrap(),
    )
    .await?;

    if let Ok(random_number) = callback_result {
        let random_number = random_number.unwrap();
        return Ok((StatusCode::OK, format!("{random_number}\n",)).into_response());
    } else {
        // TODO return with Connection::close header in response
        state.callback_map.lock().unwrap().remove(&guid);
        return Ok((StatusCode::REQUEST_TIMEOUT, String::new()).into_response());
    }
}

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/get", get(get_random))
        .route("/submit", post(submit_random))
}
