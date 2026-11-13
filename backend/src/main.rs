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
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use tokio::net::TcpListener;

use crate::config::Config;
use crate::state::AppState;
use crate::storage::upload::Uploads;

/// Fallback delay when no cached entry has a TTL deadline. While idle the task
/// normally wakes on a deadline or a generation advance; this only bounds how
/// long it can sleep with nothing to expire.
const MAINTENANCE_IDLE: Duration = Duration::from_secs(60);

/// Delay from `now` until the next midnight in UTC, when the janitor runs.
///
/// Registry timestamps are UTC, so the schedule is too: the sweep fires once a
/// day at 00:00 UTC rather than on a rolling interval.
fn until_next_midnight(now: chrono::DateTime<chrono::Utc>) -> Duration {
    let tomorrow = now.date_naive() + chrono::Days::new(1);
    let midnight = tomorrow
        .and_hms_opt(0, 0, 0)
        .expect("midnight is a valid time")
        .and_utc();
    (midnight - now).to_std().unwrap_or_default()
}

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
    let maintainer = tokio::spawn(maintain_caches(state.clone(), maintenance_rx.clone()));
    let janitor = tokio::spawn(run_janitor(state.clone(), maintenance_rx));
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
    let _ = janitor.await;

    Ok(())
}

/// Reclaims expired and stale cache entries until `shutdown` flips.
///
/// Sleeps until the earliest entry TTL, or until the layer-index generation
/// advances, and only then sweeps; with nothing cached it falls back to
/// [`MAINTENANCE_IDLE`]. Each sweep runs on the blocking pool so a large cache
/// cannot stall the async runtime.
async fn maintain_caches(state: AppState, mut shutdown: tokio::sync::watch::Receiver<bool>) {
    let changed = state.caches_changed();
    loop {
        let wait = state
            .next_cache_deadline()
            .map(|deadline| deadline.saturating_duration_since(std::time::Instant::now()))
            .unwrap_or(MAINTENANCE_IDLE);
        tokio::select! {
            _ = tokio::time::sleep(wait) => {}
            _ = changed.notified() => {}
            _ = shutdown.changed() => break,
        }
        let state = state.clone();
        if let Err(err) = tokio::task::spawn_blocking(move || state.maintain()).await {
            tracing::warn!(error = %err, "cache maintenance panicked");
        }
        tracing::debug!("cache maintenance sweep complete");
    }
}

/// Background upkeep for on-disk state: expires abandoned upload sessions and
/// sweeps blob files whose `blobs` row disappeared but whose unlink failed. It
/// sleeps until the next UTC midnight, runs one sweep, and repeats. Failures are
/// logged as warnings; the task never exits early.
async fn run_janitor(state: AppState, mut shutdown: tokio::sync::watch::Receiver<bool>) {
    loop {
        let wait = until_next_midnight(chrono::Utc::now());
        tracing::info!(
            wait_secs = wait.as_secs(),
            "janitor scheduled for next UTC midnight"
        );
        tokio::select! {
            _ = tokio::time::sleep(wait) => {}
            _ = shutdown.changed() => break,
        }
        janitor_sweep(&state).await;
    }
}

/// One janitor pass: stale upload sessions (plus orphan upload directories) and
/// orphan blob files. Each half is independent; one failing does not skip the
/// other.
async fn janitor_sweep(state: &AppState) {
    let uploads = Uploads::new(
        state.db.clone(),
        Arc::clone(&state.storage),
        Arc::clone(&state.registry),
    );
    match uploads
        .cleanup_stale(state.config.upload_session_ttl_secs)
        .await
    {
        Ok(removed) => tracing::info!(removed, "expired stale uploads"),
        Err(err) => tracing::warn!(error = %err, "stale upload cleanup failed"),
    }

    match crate::storage::gc::sweep_orphan_files(
        &state.registry,
        &state.storage,
        state.config.gc_grace_secs,
    )
    .await
    {
        Ok(removed) => tracing::info!(removed, "swept orphan blob files"),
        Err(err) => tracing::warn!(error = %err, "orphan blob file sweep failed"),
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

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    #[test]
    fn janitor_waits_until_next_utc_midnight() {
        // One minute before midnight: exactly one minute to wait.
        let just_before = chrono::Utc
            .with_ymd_and_hms(2026, 11, 13, 23, 59, 0)
            .single()
            .expect("valid instant");
        assert_eq!(until_next_midnight(just_before), Duration::from_secs(60));

        // Exactly midnight: the next run is 24 hours away, not immediate.
        let midnight = chrono::Utc
            .with_ymd_and_hms(2026, 11, 13, 0, 0, 0)
            .single()
            .expect("valid instant");
        assert_eq!(
            until_next_midnight(midnight),
            Duration::from_secs(24 * 60 * 60)
        );

        // Any time of day maps to the same next 00:00:00.
        let afternoon = chrono::Utc
            .with_ymd_and_hms(2026, 11, 13, 15, 30, 0)
            .single()
            .expect("valid instant");
        assert_eq!(
            until_next_midnight(afternoon),
            Duration::from_secs(8 * 60 * 60 + 30 * 60)
        );
    }
}
