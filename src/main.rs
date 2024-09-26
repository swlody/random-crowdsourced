mod api;
mod layers;
mod message;
mod site;
mod websocket;

use std::net::SocketAddr;

use anyhow::Result;
use axum::{serve, Router};
use layers::AddLayers as _;
use secrecy::{ExposeSecret as _, SecretString};
use tokio::net::TcpListener;
use tower_http::services::{ServeDir, ServeFile};
use tracing::Level;
use tracing_subscriber::{layer::SubscriberExt as _, util::SubscriberInitExt as _};

async fn run() -> Result<()> {
    // Initialize tracing subscribe
    tracing_subscriber::fmt()
        .with_target(true)
        .with_max_level(Level::INFO)
        .finish()
        .with(sentry::integrations::tracing::layer())
        .try_init()?;

    // TODO connection pooling: https://docs.rs/deadpool-redis/latest/deadpool_redis/
    let client = redis::Client::open(format!("{}/?protocol=resp3", std::env::var("REDIS_URL")?))?;

    // Initialize routes
    let app = Router::new()
        .nest("/", site::routes())
        .nest("/api", api::routes())
        .nest("/ws", websocket::routes())
        .nest_service("/static", ServeDir::new("assets/static"))
        .fallback_service(ServeFile::new("/static/404.html"))
        .with_sentry_layer()
        .with_tracing_layer()
        .with_state(client);

    // Listen and serve
    let listener = TcpListener::bind("0.0.0.0:8080").await?;
    tracing::info!("Listening on {}", listener.local_addr()?);
    serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .await?;

    Ok(())
}

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
