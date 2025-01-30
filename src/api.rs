use std::time::Duration;

use axum::{
    extract::State,
    http::{header, HeaderMap, StatusCode},
    response::{Html, IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use redis::AsyncCommands as _;
use rinja::Template;
use serde::Deserialize;
use tokio::time::timeout;
use uuid::Uuid;

use crate::{
    error::RrgError,
    state::{AppState, StateUpdate, BANNED_NUMBERS},
};

#[derive(Deserialize, Debug)]
struct SubmitParams {
    random_number: String,
}

#[tracing::instrument]
async fn submit_random(
    State(state): State<AppState>,
    Json(SubmitParams { random_number }): Json<SubmitParams>,
) -> Result<impl IntoResponse, RrgError> {
    #[derive(Template)]
    #[template(path = "index.html", block = "input_field")]
    struct InputFieldTemplate<'a> {
        classes: &'a str,
        context: &'a str,
    }

    if random_number.len() > 50 || BANNED_NUMBERS.get().unwrap().contains(&random_number) {
        tracing::warn!("Ignoring banned number");
        // Keep track of users who are being mean!
        sentry::configure_scope(|scope| scope.set_tag("naughty_user", "true"));

        return Ok((
            StatusCode::BAD_REQUEST,
            Html(
                InputFieldTemplate {
                    classes: r#"class="error" classes="remove error""#,
                    context: "Bad!",
                }
                .render()
                .map_err(anyhow::Error::from)?,
            ),
        ));
    }

    let mut conn = state.redis.get().await.map_err(anyhow::Error::from)?;

    // If there is someone waiting for a random number...
    if let Some(guid) = conn
        .rpop("pending_callbacks", None)
        .await
        .map_err(anyhow::Error::from)?
    {
        tracing::debug!("Random number submitted: {random_number}, returning to client: {guid}");
        sentry::configure_scope(|scope| {
            scope.set_tag("random_number", &random_number);
            scope.set_tag("associated_guid", guid);
        });

        // Send the random number over the waiter's response channel
        conn.publish::<_, _, ()>(
            "callbacks",
            serde_json::to_string(&(guid, &random_number)).unwrap(),
        )
        .await
        .map_err(anyhow::Error::from)?;

        // Indicate to any open provider portals that the user no longer needs a number
        conn.publish::<_, _, ()>(
            "state_updates",
            serde_json::to_string(&StateUpdate::Removed(guid)).unwrap(),
        )
        .await
        .map_err(anyhow::Error::from)?;

        conn.zincr::<_, _, _, ()>("counts", &random_number, 1)
            .await
            .map_err(anyhow::Error::from)?;
    } else {
        tracing::debug!("Random number submitted for no active waiters: {random_number}");

        return Ok((
            StatusCode::OK,
            Html(
                InputFieldTemplate {
                    classes: r#"class="warning" classes="remove warning""#,
                    context: "Nobody got your number!",
                }
                .render()
                .map_err(anyhow::Error::from)?,
            ),
        ));
    }

    return Ok((
        StatusCode::OK,
        Html(
            InputFieldTemplate {
                classes: r#"class="success" classes="remove success""#,
                context: "Thanks!",
            }
            .render()
            .map_err(anyhow::Error::from)?,
        ),
    ));
}

#[tracing::instrument]
async fn get_random(
    headers: HeaderMap,
    State(state): State<AppState>,
) -> Result<Response, RrgError> {
    // Grab the request-id from request headers.
    // This is a header that is inserted by the server for request tracking,
    // so we can be sure that it exists and is a valid UUID.
    let request_id_header = headers["x-request-id"].to_str()?;
    let guid = Uuid::parse_str(request_id_header)
        .inspect_err(|_| tracing::warn!("Invalid x-request-id '{request_id_header}'"))?;

    let mut conn = state.redis.get().await.map_err(anyhow::Error::from)?;

    let (tx, rx) = tokio::sync::oneshot::channel();
    state.callback_map.lock().unwrap().insert(guid, tx);

    // Register as a new waiter for a random number
    conn.lpush::<_, _, ()>("pending_callbacks", guid)
        .await
        .map_err(anyhow::Error::from)?;
    conn.publish::<_, _, ()>(
        "state_updates",
        serde_json::to_string(&StateUpdate::Added(guid)).map_err(anyhow::Error::from)?,
    )
    .await
    .map_err(anyhow::Error::from)?;

    // Wait for the random number to be sent by a provider
    // TODO configurable timeout!
    let callback_result = timeout(Duration::from_secs(30), rx).await;

    conn.lrem::<_, _, ()>("pending_callbacks", 1, guid)
        .await
        .map_err(anyhow::Error::from)?;
    conn.publish::<_, _, ()>(
        "state_updates",
        serde_json::to_string(&StateUpdate::Removed(guid)).unwrap(),
    )
    .await
    .map_err(anyhow::Error::from)?;

    if let Ok(random_number) = callback_result {
        tracing::debug!("Returning random number to client: {random_number:?}");
        let random_number = random_number.expect("Sender unexpectedly dropped");
        return Ok((StatusCode::OK, format!("{random_number}\n",)).into_response());
    } else {
        tracing::debug!("Timed out waiting for random number");
        state.callback_map.lock().unwrap().remove(&guid);
        // TODO is REQUEST_TIMEOUT the right status code? should maybe be 503
        return Ok(([(header::CONNECTION, "close")], StatusCode::REQUEST_TIMEOUT).into_response());
    }
}

async fn health_check(State(state): State<AppState>) -> impl IntoResponse {
    let mut conn = state.redis.get().await.unwrap();
    if conn.ping::<()>().await.is_err() {
        StatusCode::INTERNAL_SERVER_ERROR
    } else {
        StatusCode::OK
    }
}

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/get", get(get_random))
        .route("/submit", post(submit_random))
        .route("/health", get(health_check))
}
