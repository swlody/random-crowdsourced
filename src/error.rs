use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
};
use thiserror::Error;

#[derive(Error, Debug)]
pub enum RrgError {
    #[error(transparent)]
    Uuid(#[from] uuid::Error),

    #[error(transparent)]
    Redis(#[from] redis::RedisError),

    #[error(transparent)]
    Json(#[from] serde_json::Error),
}

impl IntoResponse for RrgError {
    fn into_response(self) -> Response {
        if let Self::Uuid(_) = self {
            StatusCode::BAD_REQUEST.into_response()
        } else {
            tracing::error!("{:?}", self);
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}
