use askama::Template;
use axum::{
    extract::{Host, State},
    response::{IntoResponse, Response},
    routing::get,
    Router,
};
use redis::AsyncCommands as _;
use uuid::Uuid;

use crate::{error::RrgError, state::AppState};

#[derive(Template)]
#[template(path = "index.html")]
struct IndexTemplate {
    pending_requests: Vec<Uuid>,
    host: String,
}

#[tracing::instrument]
async fn get_pending(redis: deadpool_redis::Pool) -> Result<Vec<Uuid>, RrgError> {
    let mut conn = redis.get().await?;
    Ok(conn.lrange("pending_callbacks", 0, -1).await?)
}

#[tracing::instrument]
async fn index(Host(host): Host, State(state): State<AppState>) -> Result<Response, RrgError> {
    let pending_requests = get_pending(state.redis).await?;
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
async fn get_top_n(redis: deadpool_redis::Pool, n: isize) -> Result<Vec<(String, f64)>, RrgError> {
    let mut conn = redis.get().await?;
    Ok(conn.zrevrange_withscores("counts", 0, n).await?)
}

#[tracing::instrument]
async fn stats(State(state): State<AppState>) -> Result<Response, RrgError> {
    let top_10 = get_top_n(state.redis, 9).await?;

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

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/", get(index))
        .route("/stats", get(stats))
        .route("/about", get(about))
}
