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

#[tracing::instrument]
async fn get_pending(mut conn: redis::aio::MultiplexedConnection) -> Result<Vec<Uuid>, RrgError> {
    Ok(conn.lrange("callbacks", 0, -1).await?)
}

#[tracing::instrument]
async fn index(Host(host): Host, State(redis): State<redis::Client>) -> Result<Response, RrgError> {
    let conn = redis.get_multiplexed_async_connection().await?;
    let pending_requests = get_pending(conn).await?;
    Ok(IndexTemplate {
        pending_requests,
        host,
    }
    .into_response())
}

#[derive(Template)]
#[template(path = "stats.html")]
struct StatsTemplate {
    top_n: Vec<(String, f64)>,
}

#[tracing::instrument]
async fn get_top_n(
    mut conn: redis::aio::MultiplexedConnection,
    n: isize,
) -> Result<Vec<(String, f64)>, RrgError> {
    Ok(conn.zrevrange_withscores("counts", 0, n).await?)
}

#[tracing::instrument]
async fn stats(State(redis): State<redis::Client>) -> Result<Response, RrgError> {
    let conn = redis.get_multiplexed_async_connection().await?;
    let top_10: Vec<(String, f64)> = get_top_n(conn, 9).await?;

    Ok(StatsTemplate { top_n: top_10 }.into_response())
}

// TODO it's a bit weird to create a new struct for every HTML template file
// and also to have to add a new route for each HTML template file?
#[derive(Template)]
#[template(path = "about.html")]
struct AboutTemplate;

#[tracing::instrument]
async fn about() -> impl IntoResponse {
    AboutTemplate
}

pub fn routes() -> Router<redis::Client> {
    Router::new()
        .route("/", get(index))
        .route("/stats", get(stats))
        .route("/about", get(about))
}
