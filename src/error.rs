use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
};
use sentry::protocol::SpanStatus;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum RrgError {
    #[error(transparent)]
    Uuid(#[from] uuid::Error),

    #[error(transparent)]
    Redis(#[from] redis::RedisError),

    #[error(transparent)]
    Json(#[from] serde_json::Error),

    #[error(transparent)]
    ToStr(#[from] axum::http::header::ToStrError),

    #[error("Bad request")]
    BadRequest,
}

impl IntoResponse for RrgError {
    fn into_response(self) -> Response {
        let (http_status, sentry_status) =
            if let Self::Uuid(_) | Self::ToStr(_) | Self::BadRequest = self {
                (StatusCode::BAD_REQUEST, SpanStatus::InvalidArgument)
            } else {
                tracing::error!("{:?}", self);
                (StatusCode::INTERNAL_SERVER_ERROR, SpanStatus::InternalError)
            };

        sentry::configure_scope(|scope| {
            scope.get_span().map(|span| span.set_status(sentry_status));
            scope.set_tag("http.status_code", http_status.as_u16());
        });

        http_status.into_response()
    }
}
