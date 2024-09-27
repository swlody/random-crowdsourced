use askama::Template;
use axum::{
    extract::{Host, State},
    response::{IntoResponse, Response},
    routing::get,
    Router,
};
use redis::Commands;
use uuid::Uuid;

use crate::error::RrgError;

#[derive(Template)]
#[template(path = "index.html")]
struct IndexTemplate {
    pending_requests: Vec<Uuid>,
    host: String,
}

async fn index(
    Host(host): Host,
    State(mut redis): State<redis::Client>,
) -> Result<Response, RrgError> {
    let pending_requests: Vec<Uuid> = redis.lrange("callbacks", 0, -1)?;
    Ok(IndexTemplate {
        pending_requests,
        host,
    }
    .into_response())
}

pub fn routes() -> Router<redis::Client> {
    Router::new().route("/", get(index))
}
