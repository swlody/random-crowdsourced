use std::{
    future::Future,
    pin::Pin,
    task::{Context, Poll},
};

use axum::{
    body::Body,
    http::{header, Request, Response, StatusCode},
    Router,
};
use sentry::{
    integrations::tower::{NewSentryLayer, SentryHttpLayer},
    protocol::SpanStatus,
};
use tower::{Layer, Service, ServiceBuilder};
use tower_http::{
    request_id::{MakeRequestId, RequestId},
    trace::{DefaultMakeSpan, DefaultOnResponse, TraceLayer},
    ServiceBuilderExt as _,
};
use uuid::Uuid;

pub trait AddLayers<State> {
    fn with_tracing_layer(self) -> Router<State>;
    fn with_sentry_layer(self) -> Router<State>;
}

#[derive(Clone, Copy)]
pub struct MakeRequestUuidV7;
impl MakeRequestId for MakeRequestUuidV7 {
    fn make_request_id<B>(&mut self, _request: &Request<B>) -> Option<RequestId> {
        // Use UUIDv7 so that request ID can be sorted by time
        let request_id = Uuid::now_v7();
        Some(RequestId::new(request_id.to_string().parse().unwrap()))
    }
}

#[derive(Copy, Clone)]
pub struct SentryRequestIdLayer;

impl<S> Layer<S> for SentryRequestIdLayer {
    type Service = SentryRequestIdService<S>;

    fn layer(&self, service: S) -> Self::Service {
        SentryRequestIdService { service }
    }
}

#[derive(Copy, Clone)]
pub struct SentryRequestIdService<S> {
    service: S,
}

impl<S> Service<Request<Body>> for SentryRequestIdService<S>
where
    S: Service<Request<Body>>,
{
    type Response = S::Response;
    type Error = S::Error;
    type Future = S::Future;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.service.poll_ready(cx)
    }

    fn call(&mut self, request: Request<Body>) -> Self::Future {
        if let Some(request_id) = request
            .headers()
            .get("x-request-id")
            .and_then(|header| header.to_str().ok())
        {
            sentry::configure_scope(|scope| {
                scope.set_tag("request_id", request_id);
            });
        }
        self.service.call(request)
    }
}

#[derive(Copy, Clone)]
pub struct SentryReportRequestSizeLayer;

impl<S> Layer<S> for SentryReportRequestSizeLayer {
    type Service = SentryReportRequestSizeService<S>;

    fn layer(&self, service: S) -> Self::Service {
        SentryReportRequestSizeService { service }
    }
}

#[derive(Copy, Clone)]
pub struct SentryReportRequestSizeService<S> {
    service: S,
}

impl<S> Service<Request<Body>> for SentryReportRequestSizeService<S>
where
    S: Service<Request<Body>>,
{
    type Response = S::Response;
    type Error = S::Error;
    type Future = S::Future;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.service.poll_ready(cx)
    }

    fn call(&mut self, request: Request<Body>) -> Self::Future {
        if let Some(content_length) = request
            .headers()
            .get(header::CONTENT_LENGTH)
            .and_then(|header| header.to_str().ok())
        {
            sentry::configure_scope(|scope| {
                scope.set_tag("http.request.content_length", content_length);
            });
        }
        self.service.call(request)
    }
}

#[derive(Clone)]
pub struct SentryReportStatusCodeLayer;

impl<S> Layer<S> for SentryReportStatusCodeLayer {
    type Service = SentryReportStatusCodeService<S>;

    fn layer(&self, inner: S) -> Self::Service {
        SentryReportStatusCodeService { inner }
    }
}

#[derive(Clone)]
pub struct SentryReportStatusCodeService<S> {
    inner: S,
}

impl<S, ReqBody> Service<Request<ReqBody>> for SentryReportStatusCodeService<S>
where
    S: Service<Request<ReqBody>, Response = Response<Body>>,
    S::Future: Send + 'static,
    S::Error: Into<axum::BoxError>,
{
    type Response = Response<Body>;
    type Error = S::Error;
    type Future = Pin<Box<dyn Future<Output = Result<Self::Response, Self::Error>> + Send>>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, req: Request<ReqBody>) -> Self::Future {
        let future = self.inner.call(req);

        Box::pin(async move {
            let response = future.await?;

            // Extract HTTP status code
            let http_status = response.status();
            let sentry_status = match http_status {
                StatusCode::INTERNAL_SERVER_ERROR => Some(SpanStatus::InternalError),
                StatusCode::BAD_REQUEST => Some(SpanStatus::InvalidArgument),
                StatusCode::OK => Some(SpanStatus::Ok),
                _ => None,
            };

            // Report status code to Sentry
            sentry::configure_scope(|scope| {
                if let Some(sentry_status) = sentry_status {
                    scope.get_span().map(|span| span.set_status(sentry_status));
                }
                scope.set_tag(
                    "http.response.status_code",
                    http_status.as_u16().to_string(),
                );
            });

            Ok(response)
        })
    }
}

impl<T> AddLayers<T> for Router<T>
where
    T: Clone + Send + Sync + 'static,
{
    fn with_tracing_layer(self) -> Self {
        // Enables tracing for each request and adds a request ID header to resposne
        let tracing_service = ServiceBuilder::new()
            .set_x_request_id(MakeRequestUuidV7)
            .layer(
                TraceLayer::new_for_http()
                    .make_span_with(DefaultMakeSpan::new().include_headers(true))
                    .on_response(DefaultOnResponse::new().include_headers(true)),
            )
            .propagate_x_request_id();
        self.layer(tracing_service)
    }

    fn with_sentry_layer(self) -> Self {
        let sentry_service = ServiceBuilder::new()
            .layer(NewSentryLayer::new_from_top())
            .layer(SentryHttpLayer::with_transaction())
            .layer(SentryRequestIdLayer)
            .layer(SentryReportRequestSizeLayer)
            .layer(SentryReportStatusCodeLayer);
        self.layer(sentry_service)
    }
}
