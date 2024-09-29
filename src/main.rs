mod api;
mod error;
mod layers;
mod message;
mod site;
mod state;
mod websocket;

use std::{
    net::SocketAddr,
    sync::{Arc, Mutex},
};

use anyhow::Result;
use askama::Template;
use axum::{response::IntoResponse, Router};
use futures_util::StreamExt as _;
use layers::AddLayers as _;
use secrecy::{ExposeSecret as _, SecretString};
use state::AppState;
use tokio::net::TcpListener;
use tower_http::services::ServeDir;
use tracing::Level;
use tracing_subscriber::{layer::SubscriberExt as _, util::SubscriberInitExt as _};
use uuid::Uuid;

fn main() -> Result<()> {
    rubenvy::rubenvy_auto()?;

    let dsn = SecretString::from(std::env::var("SENTRY_DSN")?);
    let _guard = sentry::init((
        dsn.expose_secret(),
        sentry::ClientOptions {
            release: sentry::release_name!(),
            traces_sample_rate: 1.0,
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
    // Initialize tracing subscribe
    tracing_subscriber::fmt()
        .with_target(true)
        .with_max_level(Level::INFO)
        .finish()
        .with(sentry::integrations::tracing::layer())
        .try_init()?;

    let callback_map = Arc::new(Mutex::new(state::CallbackMap::new()));

    // TODO connection pooling: https://docs.rs/deadpool-redis/latest/deadpool_redi
    let redis_url = format!("{}/?protocol=resp3", std::env::var("REDIS_URL")?);
    let redis = Arc::new(redis::Client::open(redis_url)?);

    // TODO additional single subscriber for all state updates - broadcast to
    // websockets via broadcast channel
    let (mut sink, mut stream) = redis.get_async_pubsub().await?.split();
    sink.subscribe("callbacks").await?;
    let pubsub_task = {
        let callback_map = callback_map.clone();
        tokio::task::spawn(async move {
            while let Some(msg) = stream.next().await {
                let (callback_id, random_number) = serde_json::from_str::<(Uuid, String)>(
                    std::str::from_utf8(msg.get_payload_bytes()).unwrap(),
                )
                .unwrap();
                let callback = callback_map.lock().unwrap().remove(&callback_id);
                if let Some(callback) = callback {
                    callback.send(random_number).unwrap();
                }
            }
        })
    };

    // Initialize routes
    let app = Router::new()
        .nest("/", site::routes())
        .nest("/api", api::routes())
        .nest("/ws", websocket::routes())
        .nest_service("/static", ServeDir::new("assets/static"))
        .fallback(fallback_handler)
        .with_sentry_layer()
        .with_tracing_layer()
        .with_state(AppState {
            redis,
            callback_map,
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
