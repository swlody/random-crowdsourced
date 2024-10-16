use std::time::Duration;

use axum::{
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

    // If there is someone waiting for a random number...
    if let Some(guid) = conn.rpop("pending_callbacks", None).await? {
        tracing::debug!("Random number submitted: {random_number}, returning to client: {guid}");
        // Send the random number over the response channel
        conn.publish::<_, _, ()>(
            "callbacks",
            serde_json::to_string(&(guid, &random_number)).unwrap(),
        )
        .await?;

        // Indicate to any open provider portals that the user no longer needs a number
        conn.publish::<_, _, ()>(
            "state_updates",
            serde_json::to_string(&StateUpdate::Removed(guid)).unwrap(),
        )
        .await?;

        conn.zincr::<_, _, _, ()>("counts", &random_number, 1)
            .await?;
    } else {
        tracing::debug!("Random number submitted for no active waiters: {random_number}");
    }

    Ok(StatusCode::OK.into_response())
}

#[tracing::instrument]
async fn get_random(
    headers: HeaderMap,
    State(state): State<AppState>,
) -> Result<Response, RrgError> {
    // Grab the request-id from request headers.
    // This is a header that is inserted by the server for request tracking,
    // so we can be sure that it exists and is a valid UUID.
    let guid = Uuid::parse_str(headers["x-request-id"].to_str().unwrap()).unwrap();

    let mut conn = state.redis.get().await?;

    let (tx, rx) = tokio::sync::oneshot::channel();
    state.callback_map.lock().unwrap().insert(guid, tx);

    // Register as a new waiter for a random number
    conn.lpush::<_, _, ()>("pending_callbacks", guid).await?;
    conn.publish::<_, _, ()>(
        "state_updates",
        serde_json::to_string(&StateUpdate::Added(guid))?,
    )
    .await?;

    // Wait for the random number to be sent by a provider
    // TODO configurable timeout! And/or let user specify with request header
    let callback_result = timeout(Duration::from_secs(30), rx).await;

    conn.lrem::<_, _, ()>("pending_callbacks", 1, guid).await?;
    conn.publish::<_, _, ()>(
        "state_updates",
        serde_json::to_string(&StateUpdate::Removed(guid)).unwrap(),
    )
    .await?;

    if let Ok(random_number) = callback_result {
        tracing::debug!("Returning random number to client: {random_number:?}");
        let random_number = random_number.unwrap();
        return Ok((StatusCode::OK, format!("{random_number}\n",)).into_response());
    } else {
        // TODO return with Connection::close header in response
        tracing::debug!("Timed out waiting for random number");
        state.callback_map.lock().unwrap().remove(&guid);
        return Ok(StatusCode::REQUEST_TIMEOUT.into_response());
    }
}

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/get", get(get_random))
        .route("/submit", post(submit_random))
}
