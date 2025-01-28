mod api;
mod error;
mod layers;
mod site;
mod state;
mod websocket;

use std::{
    collections::HashSet,
    net::SocketAddr,
    sync::{Arc, Mutex},
};

use anyhow::Result;
use askama::Template;
use aws_config::BehaviorVersion;
use axum::{response::IntoResponse, Router};
use futures_util::StreamExt as _;
use layers::AddLayers as _;
use secrecy::{ExposeSecret as _, SecretString};
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

    let dsn = SecretString::from(std::env::var("SENTRY_DSN")?);
    let _guard = sentry::init((
        dsn.expose_secret(),
        sentry::ClientOptions {
            debug: true,
            release: sentry::release_name!(),
            traces_sample_rate: 0.1,
            attach_stacktrace: true,
            ..Default::default()
        },
    ));

    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?
        .block_on(run())
}

async fn run() -> Result<()> {
    let callback_map = Arc::new(Mutex::new(state::CallbackMap::new()));

    // TODO capacity? configurable?
    let tx = tokio::sync::broadcast::Sender::new(10);
    let state_updates = Arc::new(tx.clone());

    let redis_url = std::env::var("REDIS_URL").expect("Missing environment variable REDIS_URL");
    let redis = redis::Client::open(redis_url.clone()).unwrap();

    let pubsub_task = {
        let (mut sink, mut stream) = redis::Client::open(redis_url)
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
                        if tx.receiver_count() > 0 {
                            tracing::debug!("Broadcasting state update: {state_update:?}");
                            tx.send(state_update).unwrap();
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

    // Initialize routes
    let app = Router::new()
        .nest("/", site::routes())
        .nest("/api", api::routes())
        .nest("/ws", websocket::routes())
        .nest_service("/static", ServeDir::new("assets/static"))
        .fallback(fallback_handler)
        .with_tracing_layer()
        .with_sentry_layer()
        .with_state(AppState {
            redis: redis.get_multiplexed_async_connection().await.unwrap(),
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

async fn fallback_handler() -> impl IntoResponse {
    #[derive(Template)]
    #[template(path = "404.html")]
    struct NotFoundTemplate;

    NotFoundTemplate
}
