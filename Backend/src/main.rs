// Shared contract surface consumed by the later waves (docs/ARCHITECTURE.md §3).
#![allow(dead_code)]

mod api;
mod auth;
mod captcha;
mod config;
mod db;
mod email;
mod error;
mod logging;
mod models;
mod oci;
mod permissions;
mod ratelimit;
mod routes;
mod static_files;
mod state;
mod storage;

use std::net::SocketAddr;

use anyhow::{Context, Result};
use tokio::net::TcpListener;

use crate::config::Config;
use crate::state::AppState;

#[tokio::main]
async fn main() -> Result<()> {
    dotenvy::dotenv().ok();

    let config = Config::from_env()?;
    logging::init(&config)?;

    std::fs::create_dir_all(&config.database_dir).context("creating database directory")?;

    let db = db::connect(&config.database_url).await?;
    db::migrate(&db).await?;

    let addr = config.bind_addr;
    let state = AppState::new(config, db).await?;
    let app = routes::build(state);

    let listener = TcpListener::bind(addr)
        .await
        .with_context(|| format!("binding {addr}"))?;
    tracing::info!(%addr, "lighthouse registry listening");

    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .with_graceful_shutdown(shutdown_signal())
    .await
    .context("server error")?;

    Ok(())
}

async fn shutdown_signal() {
    let ctrl_c = async {
        tokio::signal::ctrl_c().await.ok();
    };

    #[cfg(unix)]
    let terminate = async {
        match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            Ok(mut signal) => {
                signal.recv().await;
            }
            Err(err) => tracing::warn!(error = %err, "cannot install SIGTERM handler"),
        }
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {}
        _ = terminate => {}
    }

    tracing::info!("shutdown signal received");
}
