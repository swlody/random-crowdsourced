use askama::Template;
use axum::{
    extract::{Host, State},
    response::{IntoResponse, Response},
    routing::get,
    Router,
};
use redis::AsyncCommands as _;
use uuid::Uuid;

use crate::error::RrgError;

#[derive(Template)]
#[template(path = "index.html")]
struct IndexTemplate {
    pending_requests: Vec<Uuid>,
    host: String,
}

async fn index(Host(host): Host, State(redis): State<redis::Client>) -> Result<Response, RrgError> {
    let mut conn = redis.get_multiplexed_async_connection().await?;
    let pending_requests: Vec<Uuid> = conn.lrange("callbacks", 0, -1).await?;
    Ok(IndexTemplate {
        pending_requests,
        host,
    }
    .into_response())
}

pub fn routes() -> Router<redis::Client> {
    Router::new().route("/", get(index))
}
