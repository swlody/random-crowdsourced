use std::sync::atomic::Ordering;

use axum::{extract::State, http::StatusCode, response::IntoResponse, routing::get, Router};

use crate::state::AppState;

#[tracing::instrument]
async fn get_random(State(state): State<AppState>) -> impl IntoResponse {
    state.waiting_calls.fetch_add(1, Ordering::Relaxed);
    let random_number = state.rx.recv().await.unwrap();
    state.waiting_calls.fetch_sub(1, Ordering::Relaxed);
    (StatusCode::OK, random_number.to_string())
}

pub fn routes() -> Router<AppState> {
    Router::new().route("/get", get(get_random))
}
