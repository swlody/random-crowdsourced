use std::sync::Arc;

use axum::{
    extract::State,
    response::{Html, IntoResponse},
    routing::get,
    Router,
};
use axum_extra::extract::Host;
use redis::{cmd, AsyncCommands as _};
use rinja::Template;
use uuid::Uuid;

use crate::{error::RrgError, state::AppState};

#[tracing::instrument]
async fn get_pending(redis: Arc<deadpool_redis::Pool>) -> Result<Vec<Uuid>, RrgError> {
    // get all members pending_callbacks list
    let mut conn = redis.get().await?;
    // let pending_requests = conn
    //     .lrange("pending_callbacks", 0, -1)
    //     .await
    //     .map_err(|e| RrgError::RenderingInternalError(anyhow::Error::new(e)))?;

    let pending_requests = cmd("LRANGE")
        .arg("pending_callbacks")
        .arg(0)
        .arg(-1)
        .query_async(&mut conn)
        .await
        .unwrap();
    Ok(pending_requests)
}

#[tracing::instrument]
async fn index(
    Host(host): Host,
    State(state): State<AppState>,
) -> Result<impl IntoResponse, RrgError> {
    #[derive(Template)]
    #[template(path = "index.html")]
    struct IndexTemplate {
        pending_requests: Vec<Uuid>,
        host: String,
    }

    let pending_requests = get_pending(state.redis).await?;

    Ok(Html(
        IndexTemplate {
            pending_requests,
            host,
        }
        .render()
        .map_err(|e| RrgError::RenderingInternalError(anyhow::Error::new(e)))?,
    ))
}

#[tracing::instrument]
async fn get_top_n(
    redis: Arc<deadpool_redis::Pool>,
    n: isize,
) -> Result<Vec<(String, f64)>, RrgError> {
    let mut conn = redis.get().await?;
    // Get the top n values with the highest scores along with their scores
    let top_n = conn
        .zrevrange_withscores("counts", 0, n)
        .await
        .map_err(|e| RrgError::RenderingInternalError(anyhow::Error::new(e)))?;
    Ok(top_n)
}

#[tracing::instrument]
async fn stats(State(state): State<AppState>) -> Result<impl IntoResponse, RrgError> {
    #[derive(Template)]
    #[template(path = "stats.html")]
    struct StatsTemplate {
        top_n: Vec<(String, f64)>,
    }

    let top_10 = get_top_n(state.redis, 9).await?;

    Ok(Html(StatsTemplate { top_n: top_10 }.render().map_err(
        |e| RrgError::RenderingInternalError(anyhow::Error::new(e)),
    )?))
}

#[tracing::instrument]
async fn about() -> Result<impl IntoResponse, RrgError> {
    #[derive(Template)]
    #[template(path = "about.html")]
    struct AboutTemplate;

    Ok(Html(AboutTemplate.render().map_err(|e| {
        RrgError::RenderingInternalError(anyhow::Error::new(e))
    })?))
}

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/", get(index))
        .route("/stats", get(stats))
        .route("/about", get(about))
}
