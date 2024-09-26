use askama::Template;
use axum::{
    extract::{Host, State},
    response::IntoResponse,
    routing::get,
    Router,
};
use redis::Commands;
use uuid::Uuid;

#[derive(Template)]
#[template(path = "index.html")]
struct IndexTemplate {
    pending_requests: Vec<Uuid>,
    host: String,
}

async fn index(Host(host): Host, State(mut redis): State<redis::Client>) -> impl IntoResponse {
    let pending_requests: Vec<Uuid> = redis.lrange("callbacks", 0, -1).unwrap();
    IndexTemplate {
        pending_requests,
        host,
    }
}

pub fn routes() -> Router<redis::Client> {
    Router::new().route("/", get(index))
}
