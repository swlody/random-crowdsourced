mod api;
mod error;
mod layers;
mod site;
mod state;
mod websocket;

use core::panic;
use std::{
    collections::HashSet,
    net::SocketAddr,
    sync::{Arc, Mutex},
};

use anyhow::Result;
use aws_config::BehaviorVersion;
use axum::Router;
use error::RrgError;
use futures_util::StreamExt as _;
use layers::AddLayers as _;
use state::{AppState, BANNED_NUMBERS};
use tokio::net::TcpListener;
use tower_http::services::ServeDir;
use tracing::Level;
use tracing_subscriber::{layer::SubscriberExt as _, util::SubscriberInitExt as _};

fn main() -> Result<()> {
    rubenvy::rubenvy_auto()?;

    // Initialize tracing subscribe
    tracing_subscriber::fmt()
        .with_target(true)
        .with_max_level(Level::DEBUG)
        .pretty()
        .finish()
        .with(sentry::integrations::tracing::layer())
        .try_init()?;

    let guard = std::env::var("SENTRY_DSN").map_or_else(
        |_| None,
        |dsn| {
            let sample_rate = std::env::var("SENTRY_TRACES_SAMPLE_RATE")
                .map(|v| {
                    let rate = v.parse().unwrap_or_else(|_| {
                        panic!("Invalid value for SENTRY_TRACES_SAMPLE_RATE{v}")
                    });

                    assert!(
                        (0.0..=1.0).contains(&rate),
                        "Invalid value for SENTRY_TRACES_SAMPLE_RATE: {v}"
                    );
                    rate
                })
                .unwrap_or(0.1);
            Some(sentry::init((
                dsn,
                sentry::ClientOptions {
                    release: sentry::release_name!(),
                    traces_sample_rate: sample_rate,
                    attach_stacktrace: true,
                    ..Default::default()
                },
            )))
        },
    );

    if guard.is_some() {
        tracing::info!("Initialized sentry client");
    } else {
        tracing::warn!("Sentry DSN not found. Sentry is not initialized");
    }

    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?
        .block_on(run())
}

#[allow(clippy::too_many_lines)]
async fn run() -> Result<()> {
    let config = aws_config::load_defaults(BehaviorVersion::latest()).await;
    let s3 = aws_sdk_s3::Client::new(&config);
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

    let tx = tokio::sync::broadcast::Sender::new(
        std::env::var("BROADCAST_CAPACITY")
            .map(|s| s.parse().expect("Invalid broadcast capacity"))
            .unwrap_or(10),
    );
    let state_updates = Arc::new(tx.clone());

    let redis_url = std::env::var("REDIS_URL").expect("Missing environment variable REDIS_URL");
    let redis =
        redis::Client::open(redis_url.as_str()).expect("Unable to open initialize redis client");

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
                        let state_update = serde_json::from_slice(msg.get_payload_bytes()).unwrap();
                        if tx.len() >= 10 {
                            tracing::error!(
                                "Potentially dropping queued random numbers. Consider increasing \
                                 the capacity of the broadcast channel."
                            );
                        }
                        if tx.receiver_count() > 0 {
                            tracing::debug!("Broadcasting state update: {state_update:?}");
                            tx.send(state_update)
                                .expect("Receiver unexpectedly dropped");
                        } else {
                            tracing::debug!(
                                "Processing state update but no open subscribers: {state_update:?}"
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
        // sentry needs to go before tracing otherwise the request-id doesn't get passed to sentry
        .with_sentry_layer()
        .with_tracing_layer()
        // Very generous limit for submit requests
        .layer(tower_http::limit::RequestBodyLimitLayer::new(4096))
        .with_state(AppState {
            redis: redis
                .get_multiplexed_async_connection()
                .await
                .expect("Unable to open redis connection"),
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
