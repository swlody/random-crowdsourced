mod api;
mod error;
mod layers;
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
use deadpool_redis::Runtime;
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

    if let Ok(dsn) = std::env::var("SENTRY_DSN") {
        let dsn = SecretString::from(dsn);
        let _guard = sentry::init((
            dsn.expose_secret(),
            sentry::ClientOptions {
                release: sentry::release_name!(),
                traces_sample_rate: 1.0,
                attach_stacktrace: true,
                ..Default::default()
            },
        ));
    }

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

    // TODO capacity? configurable?
    let (tx, rx) = tokio::sync::broadcast::channel(10);
    // Drop rx to avoid slow receiver problem: https://docs.rs/tokio/latest/tokio/sync/broadcast/index.html#lagging
    drop(rx);
    let state_updates = Arc::new(tx.clone());

    let redis_url = std::env::var("REDIS_URL")?;
    let config = deadpool_redis::Config::from_url(&redis_url);
    let redis = config.create_pool(Some(Runtime::Tokio1))?;

    let pubsub_task = {
        let (mut sink, mut stream) = redis::Client::open(redis_url)
            .unwrap()
            .get_async_pubsub()
            .await?
            .split();
        sink.subscribe("callbacks").await?;
        sink.subscribe("state_updates").await?;

        let callback_map = callback_map.clone();

        tokio::task::spawn(async move {
            while let Some(msg) = stream.next().await {
                let msg_str = std::str::from_utf8(msg.get_payload_bytes()).unwrap();

                match msg.get_channel_name() {
                    "callbacks" => {
                        let (callback_id, random_number) =
                            serde_json::from_str::<(Uuid, String)>(msg_str).unwrap();
                        let callback = callback_map.lock().unwrap().remove(&callback_id);
                        if let Some(callback) = callback {
                            let _ = callback.send(random_number);
                        }
                    }
                    "state_updates" => {
                        if tx.receiver_count() > 0 {
                            let state_update =
                                serde_json::from_str::<state::StateUpdate>(msg_str).unwrap();

                            tx.send(state_update).unwrap();
                        }
                    }

                    c => panic!("unknown channel: {c}"),
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
