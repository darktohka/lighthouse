// Shared contract surface consumed by the later waves (docs/ARCHITECTURE.md §3).
#![allow(dead_code)]

mod api;
mod auth;
mod captcha;
mod config;
mod db;
mod email;
mod error;
mod layer_cache;
mod logging;
mod models;
mod net;
mod oci;
mod permissions;
mod ratelimit;
mod routes;
mod state;
mod static_files;
mod storage;

use std::net::SocketAddr;
use std::time::Duration;

use anyhow::{Context, Result};
use tokio::net::TcpListener;

use crate::config::Config;
use crate::state::AppState;

/// How often the background task reclaims expired and generation-stale cache
/// entries.
const MAINTENANCE_INTERVAL: Duration = Duration::from_secs(60);

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
    let (maintenance_tx, maintenance_rx) = tokio::sync::watch::channel(false);
    let maintainer = tokio::spawn(maintain_caches(state.clone(), maintenance_rx));
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

    let _ = maintenance_tx.send(true);
    let _ = maintainer.await;

    Ok(())
}

/// Reclaims expired and stale cache entries on an interval until `shutdown`
/// flips. Each sweep runs on the blocking pool so a large cache cannot stall
/// the async runtime.
async fn maintain_caches(state: AppState, mut shutdown: tokio::sync::watch::Receiver<bool>) {
    let mut ticker = tokio::time::interval(MAINTENANCE_INTERVAL);
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    ticker.tick().await;
    loop {
        tokio::select! {
            _ = ticker.tick() => {
                let state = state.clone();
                if let Err(err) = tokio::task::spawn_blocking(move || state.maintain()).await {
                    tracing::warn!(error = %err, "cache maintenance panicked");
                }
                tracing::debug!("cache maintenance sweep complete");
            }
            _ = shutdown.changed() => break,
        }
    }
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
