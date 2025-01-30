use axum::{
    http::StatusCode,
    response::{Html, IntoResponse, Response},
};
use rinja::Template;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum RrgError {
    #[error(transparent)]
    Uuid(#[from] uuid::Error),

    #[error(transparent)]
    Deadpool(#[from] deadpool_redis::PoolError),

    #[error(transparent)]
    Redis(#[from] redis::RedisError),

    #[error(transparent)]
    Json(#[from] serde_json::Error),

    #[error(transparent)]
    ToStr(#[from] axum::http::header::ToStrError),

    #[error(transparent)]
    Render(#[from] rinja::Error),

    #[error("Not found")]
    NotFound,

    #[error(transparent)]
    RenderingInternalError(#[from] anyhow::Error),
}

#[derive(Template)]
#[template(path = "404.html")]
pub struct NotFoundTemplate;

#[derive(Template)]
#[template(path = "something_went_wrong.html")]
pub struct SomethingWentWrongTemplate;

impl IntoResponse for RrgError {
    fn into_response(self) -> Response {
        match self {
            Self::Uuid(_) | Self::ToStr(_) => StatusCode::BAD_REQUEST.into_response(),
            Self::NotFound => NotFoundTemplate.render().map_or_else(
                |_| StatusCode::INTERNAL_SERVER_ERROR.into_response(),
                |body| (StatusCode::NOT_FOUND, Html(body)).into_response(),
            ),
            Self::RenderingInternalError(e) => {
                tracing::error!("Error occurred while rendering page: {e:?}");
                SomethingWentWrongTemplate.render().map_or_else(
                    |_| StatusCode::INTERNAL_SERVER_ERROR.into_response(),
                    |body| (StatusCode::INTERNAL_SERVER_ERROR, Html(body)).into_response(),
                )
            }
            _ => {
                tracing::error!("{:?}", self);
                StatusCode::INTERNAL_SERVER_ERROR.into_response()
            }
        }
    }
}
