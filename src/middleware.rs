use std::{
    future::Future,
    pin::Pin,
    task::{Context, Poll},
};

use axum::http::{header, Request, Response, StatusCode};
use sentry::protocol::SpanStatus;
use tower::{Layer, Service};
use tower_http::request_id::{MakeRequestId, RequestId};
use uuid::Uuid;

#[derive(Clone, Copy)]
pub struct MakeRequestUuidV7;
impl MakeRequestId for MakeRequestUuidV7 {
    fn make_request_id<B>(&mut self, _request: &Request<B>) -> Option<RequestId> {
        // Use UUIDv7 so that request ID can be sorted by time
        let request_id = Uuid::now_v7();
        Some(RequestId::new(request_id.to_string().parse().unwrap()))
    }
}

#[derive(Clone)]
pub struct SentryReportRequestInfoLayer;

impl<S> Layer<S> for SentryReportRequestInfoLayer {
    type Service = SentryReportRequestInfoService<S>;

    fn layer(&self, inner: S) -> Self::Service {
        SentryReportRequestInfoService { inner }
    }
}

fn convert_http_status_to_sentry_status(http_status: StatusCode) -> SpanStatus {
    match http_status {
        StatusCode::OK => SpanStatus::Ok,
        StatusCode::UNAUTHORIZED => SpanStatus::Unauthenticated,
        StatusCode::FORBIDDEN => SpanStatus::PermissionDenied,
        StatusCode::NOT_FOUND => SpanStatus::NotFound,
        StatusCode::CONFLICT => SpanStatus::AlreadyExists,
        StatusCode::PRECONDITION_FAILED => SpanStatus::FailedPrecondition,
        StatusCode::TOO_MANY_REQUESTS => SpanStatus::ResourceExhausted,
        StatusCode::NOT_IMPLEMENTED => SpanStatus::Unimplemented,
        StatusCode::SERVICE_UNAVAILABLE => SpanStatus::Unavailable,

        // TODO remove if switching which status code is used for timeouts
        StatusCode::REQUEST_TIMEOUT => SpanStatus::DeadlineExceeded,

        other => {
            let code = other.as_u16();
            if (100..=399).contains(&code) {
                SpanStatus::Ok
            } else if (400..=499).contains(&code) {
                SpanStatus::InvalidArgument
            } else if (500..=599).contains(&code) {
                SpanStatus::InternalError
            } else {
                SpanStatus::UnknownError
            }
        }
    }
}

#[derive(Clone)]
pub struct SentryReportRequestInfoService<S> {
    inner: S,
}

impl<S, ReqBody, ResBody> Service<Request<ReqBody>> for SentryReportRequestInfoService<S>
where
    S: Service<Request<ReqBody>, Response = Response<ResBody>>,
    S::Future: Send + 'static,
    S::Error: Into<axum::BoxError>,
{
    type Response = S::Response;
    type Error = S::Error;
    type Future = Pin<Box<dyn Future<Output = Result<Self::Response, Self::Error>> + Send>>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, request: Request<ReqBody>) -> Self::Future {
        if let Some(content_length) = request
            .headers()
            .get(header::CONTENT_LENGTH)
            .and_then(|header| header.to_str().ok())
        {
            sentry::configure_scope(|scope| {
                scope.set_tag("http.request.content_length", content_length);
            });
        }

        if let Some(request_id) = request
            .headers()
            .get("x-request-id")
            .and_then(|header| header.to_str().ok())
        {
            sentry::configure_scope(|scope| {
                scope.set_tag("request_id", request_id);
            });
        }

        let future = self.inner.call(request);

        Box::pin(async move {
            let response = future.await?;

            // Extract HTTP status code
            let http_status = response.status();
            let sentry_status = convert_http_status_to_sentry_status(http_status);

            // Report status code to Sentry
            sentry::configure_scope(|scope| {
                if let Some(span) = scope.get_span() {
                    span.set_status(sentry_status);
                };
                scope.set_tag(
                    "http.response.status_code",
                    http_status.as_u16().to_string(),
                );
            });

            Ok(response)
        })
    }
}
