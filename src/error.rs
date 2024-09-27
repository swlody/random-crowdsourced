use axum::{
    body::Body,
    http::StatusCode,
    response::{IntoResponse, Response},
};

#[allow(clippy::module_name_repetitions)]
pub struct RrgError(pub anyhow::Error);

impl IntoResponse for RrgError {
    fn into_response(self) -> Response {
        tracing::error!("{}", self.0);
        (StatusCode::INTERNAL_SERVER_ERROR, Body::empty()).into_response()
    }
}

impl<E> From<E> for RrgError
where
    E: Into<anyhow::Error>,
{
    fn from(err: E) -> Self {
        Self(err.into())
    }
}
