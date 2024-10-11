use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
};

pub struct RrgError(pub anyhow::Error);

impl IntoResponse for RrgError {
    fn into_response(self) -> Response {
        tracing::error!("{}", self.0);
        StatusCode::INTERNAL_SERVER_ERROR.into_response()
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
