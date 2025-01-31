mod api;
mod error;
mod middleware;
mod site;
mod state;
mod websocket;

use core::panic;
use std::{
    collections::HashSet,
    net::SocketAddr,
    str::FromStr,
    sync::{Arc, Mutex},
    time::Duration,
};

use anyhow::Result;
use aws_config::BehaviorVersion;
use axum::Router;
use deadpool_redis::Runtime;
use error::RrgError;
use futures_util::StreamExt as _;
use middleware::{MakeRequestUuidV7, SentryReportRequestInfoLayer};
use rinja::Template;
use secrecy::{ExposeSecret as _, SecretString};
use sentry::integrations::tower::{NewSentryLayer, SentryHttpLayer};
use state::{AppState, StateUpdate, BANNED_NUMBERS};
use tokio::net::TcpListener;
use tower::ServiceBuilder;
use tower_http::{
    limit::RequestBodyLimitLayer,
    services::ServeDir,
    timeout::TimeoutLayer,
    trace::{DefaultMakeSpan, DefaultOnResponse, TraceLayer},
    ServiceBuilderExt as _,
};
use tracing::Level;
use tracing_subscriber::{layer::SubscriberExt as _, util::SubscriberInitExt as _};
use uuid::Uuid;

struct Config {
    log_level: Option<tracing::metadata::Level>,
    trace_sample_rate: Option<f32>,
    error_sample_rate: Option<f32>,
    broadcast_capacity: Option<usize>,

    sentry_dsn: SecretString,
    redis_url: String,
}

impl Config {
    fn from_environment() -> Self {
        Self {
            broadcast_capacity: std::env::var("RRG_BROADCAST_CAPACITY")
                .ok()
                .map(|capacity| {
                    capacity
                        .parse()
                        .unwrap_or_else(|_| panic!("Invalid broadcast capacity: {capacity}"))
                }),

            log_level: std::env::var("RRG_LOG_LEVEL").ok().map(|level| {
                Level::from_str(level.as_str())
                    .unwrap_or_else(|_| panic!("Invalid value for RRG_LOG_LEVEL: {level}"))
            }),

            trace_sample_rate: std::env::var("RRG_SENTRY_TRACING_SAMPLE_RATE")
                .ok()
                .map(|rate| {
                    let rate = rate.parse().unwrap_or_else(|_| {
                        panic!("Invalid value for RRG_SENTRY_TRACING_SAMPLE_RATE: {rate}")
                    });
                    assert!((0.0..=1.0).contains(&rate));
                    rate
                }),

            error_sample_rate: std::env::var("RRG_SENTRY_ERROR_SAMPLE_RATE")
                .ok()
                .map(|rate| {
                    let rate = rate.parse().unwrap_or_else(|_| {
                        panic!("Invalid value for RRG_SENTRY_TRACING_SAMPLE_RATE: {rate}")
                    });
                    assert!((0.0..=1.0).contains(&rate));
                    rate
                }),

            sentry_dsn: SecretString::from(
                std::env::var("SENTRY_DSN").expect("Missing SENTRY_DSN"),
            ),

            redis_url: std::env::var("REDIS_URL").expect("Missing environment variable REDIS_URL"),
        }
    }
}

fn main() -> Result<()> {
    rubenvy::rubenvy_auto()?;

    let config = Config::from_environment();

    // Initialize tracing subscribe
    tracing_subscriber::fmt()
        .with_target(true)
        .with_max_level(config.log_level.unwrap_or(Level::DEBUG))
        .pretty()
        .finish()
        .with(sentry::integrations::tracing::layer())
        .try_init()?;

    let _guard = sentry::init((
        config.sentry_dsn.expose_secret(),
        sentry::ClientOptions {
            release: sentry::release_name!(),
            sample_rate: config.error_sample_rate.unwrap_or(1.0),
            traces_sample_rate: config.trace_sample_rate.unwrap_or(0.1),
            attach_stacktrace: true,
            ..Default::default()
        },
    ));

    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?
        .block_on(run(config))
}

#[allow(clippy::too_many_lines)]
async fn run(config: Config) -> Result<()> {
    let aws_config = aws_config::load_defaults(BehaviorVersion::latest()).await;
    let s3 = aws_sdk_s3::Client::new(&aws_config);
    let banned_numbers = s3
        .get_object()
        .bucket("random-crowdsourced")
        .key("banned_numbers.txt")
        .send()
        .await?
        .body
        .collect()
        .await?
        .to_vec();
    BANNED_NUMBERS
        .set(
            String::from_utf8(banned_numbers)?
                .lines()
                .map(str::to_owned)
                .collect::<HashSet<_>>(),
        )
        .unwrap();

    let callback_map = Arc::new(Mutex::new(state::CallbackMap::new()));

    let tx = tokio::sync::broadcast::Sender::new(config.broadcast_capacity.unwrap_or(10));
    let state_updates = Arc::new(tx.clone());

    let redis_url = config.redis_url;
    let deadpool_config = deadpool_redis::Config::from_url(redis_url.as_str());
    let redis = Arc::new(deadpool_config.create_pool(Some(Runtime::Tokio1))?);

    let pubsub_task = {
        let (mut sink, mut stream) = redis::Client::open(redis_url.as_str())
            .unwrap()
            .get_async_pubsub()
            .await
            .expect("Unable to connect to Redis instance")
            .split();
        sink.subscribe("callbacks").await?;
        sink.subscribe("state_updates").await?;

        let callback_map = callback_map.clone();

        tokio::task::spawn(async move {
            while let Some(msg) = stream.next().await {
                match msg.get_channel_name() {
                    "callbacks" => {
                        let (callback_id, random_number) =
                            serde_json::from_slice(msg.get_payload_bytes()).unwrap();
                        let callback = callback_map.lock().unwrap().remove(&callback_id);
                        if let Some(callback) = callback {
                            if callback.send(random_number).is_err() {
                                tracing::debug!(
                                    "{callback_id} dropped before receiving random number"
                                );
                            }
                        }
                    }
                    "state_updates" => {
                        let state_update: StateUpdate =
                            serde_json::from_slice(msg.get_payload_bytes()).unwrap();
                        if tx.len() >= 10 {
                            tracing::error!(
                                "Potentially dropping queued random numbers. Consider increasing \
                                 the capacity of the broadcast channel."
                            );
                        }
                        if tx.receiver_count() > 0 {
                            tracing::debug!("Broadcasting state update: {state_update:?}");

                            let list_item = match state_update {
                                StateUpdate::Added(guid) => ListItemFragment {
                                    client: guid,
                                    delete: false,
                                }
                                .render()
                                .unwrap(),
                                StateUpdate::Removed(guid) => ListItemFragment {
                                    client: guid,
                                    delete: true,
                                }
                                .render()
                                .unwrap(),
                            };

                            tx.send(list_item).expect("Receiver unexpectedly dropped");
                        } else {
                            tracing::debug!(
                                "Processed state update but no open subscribers: {state_update:?}"
                            );
                        }
                    }

                    c => panic!("unknown channel: {c}"),
                }
            }
        })
    };

    // Initialize routes
    let app = Router::new()
        .merge(site::routes())
        .nest("/api", api::routes())
        .nest("/ws", websocket::routes())
        .nest_service("/static", ServeDir::new("assets/static"))
        .fallback(|| async { RrgError::NotFound })
        .layer(
            ServiceBuilder::new()
                .set_x_request_id(MakeRequestUuidV7)
                .layer(NewSentryLayer::new_from_top())
                .layer(SentryHttpLayer::with_transaction())
                .layer(SentryReportRequestInfoLayer)
                .layer(
                    TraceLayer::new_for_http()
                        .make_span_with(DefaultMakeSpan::new().include_headers(true))
                        .on_response(DefaultOnResponse::new().include_headers(true)),
                )
                .propagate_x_request_id()
                // Very generous limit for submit requests
                .layer(RequestBodyLimitLayer::new(4096))
                .layer(TimeoutLayer::new(Duration::from_secs(30))),
        )
        .with_state(AppState {
            redis,
            callback_map,
            state_updates,
        });

    // Listen and serve
    let listener = TcpListener::bind("0.0.0.0:8080").await?;
    tracing::info!("Listening on {}", listener.local_addr()?);
    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .await?;

    // TODO clean shutdown
    pubsub_task.abort();

    Ok(())
}

#[derive(Template)]
#[template(path = "list_item.html")]
struct ListItemFragment {
    client: Uuid,
    delete: bool,
}
