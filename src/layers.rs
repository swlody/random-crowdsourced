use std::task::{Context, Poll};

use axum::{body::Body, http::Request, Router};
use sentry::integrations::tower::{NewSentryLayer, SentryHttpLayer};
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
            .layer(SentryRequestIdLayer);
        self.layer(sentry_service)
    }
}
