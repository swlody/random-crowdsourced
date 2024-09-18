mod api;
mod layers;
mod state;
mod websocket;

use std::{
    net::SocketAddr,
    sync::{atomic::AtomicUsize, Arc},
};

use anyhow::Result;
use axum::{serve, Router};
use layers::AddLayers as _;
use secrecy::{ExposeSecret, Secret};
use tokio::net::TcpListener;
use tower_http::services::{ServeDir, ServeFile};
use tracing::Level;
use tracing_subscriber::{layer::SubscriberExt as _, util::SubscriberInitExt as _};

use crate::state::AppState;

async fn run() -> Result<()> {
    // Initialize tracing subscribe
    tracing_subscriber::fmt()
        .with_target(true)
        .with_max_level(Level::DEBUG)
        .finish()
        .with(sentry::integrations::tracing::layer())
        .try_init()?;

    let (tx, rx) = async_channel::unbounded();

    // Initialize routes
    let app = Router::new()
        .route_service("/", ServeFile::new("static/index.html"))
        .nest("/api", api::routes())
        .nest("/ws", websocket::routes())
        .nest_service("/static", ServeDir::new("static"))
        .fallback_service(ServeFile::new("/static/404.html"))
        .with_sentry_layer()
        .with_tracing_layer()
        .with_state(AppState {
            tx,
            rx,
            waiting_calls: Arc::new(AtomicUsize::new(0)),
        });

    // Listen and serve
    let listener = TcpListener::bind("0.0.0.0:8080").await?;
    serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .await?;

    Ok(())
}

fn main() -> Result<()> {
    rubenvy::rubenvy_auto()?;

    let dsn = Secret::new(std::env::var("SENTRY_DSN")?);
    let _guard = sentry::init((
        dsn.expose_secret().as_str(),
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
