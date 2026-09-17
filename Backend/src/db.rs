use std::str::FromStr;
use std::time::Duration;

use anyhow::{Context, Result};
use sqlx::SqlitePool;
use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteSynchronous};

pub type Db = SqlitePool;

/// Opens the SQLite pool with WAL journaling and a 5s busy timeout so that
/// concurrent readers never block writers and writers wait instead of failing.
pub async fn connect(url: &str) -> Result<Db> {
    let options = SqliteConnectOptions::from_str(url)
        .with_context(|| format!("invalid DATABASE_URL: {url}"))?
        .create_if_missing(true)
        .journal_mode(SqliteJournalMode::Wal)
        .synchronous(SqliteSynchronous::Normal)
        .busy_timeout(Duration::from_millis(5000))
        .foreign_keys(true)
        .pragma("temp_store", "memory")
        .pragma("cache_size", "-8000");

    let pool = SqlitePoolOptions::new()
        .max_connections(8)
        .min_connections(1)
        .acquire_timeout(Duration::from_secs(15))
        .idle_timeout(Duration::from_secs(600))
        .connect_with(options)
        .await?;

    Ok(pool)
}

pub async fn migrate(pool: &Db) -> Result<()> {
    sqlx::migrate!("./migrations")
        .run(pool)
        .await
        .context("failed to run database migrations")?;
    Ok(())
}

/// True when the error is a transient SQLite lock/busy condition.
pub fn is_busy(err: &sqlx::Error) -> bool {
    match err {
        sqlx::Error::Database(db) => {
            let code = db.code().map(|c| c.to_string());
            if matches!(code.as_deref(), Some("5") | Some("6") | Some("261")) {
                return true;
            }
            let message = db.message();
            message.contains("database is locked")
                || message.contains("database table is locked")
                || message.contains("database schema is locked")
        }
        _ => false,
    }
}

/// Retries an operation with exponential backoff when SQLite reports busy.
pub async fn with_busy_retry<T, F, Fut>(mut op: F) -> Result<T, sqlx::Error>
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = Result<T, sqlx::Error>>,
{
    let mut attempt = 0u32;
    loop {
        match op().await {
            Ok(value) => return Ok(value),
            Err(err) if is_busy(&err) && attempt < 8 => {
                attempt += 1;
                let backoff = Duration::from_millis(20 * u64::from(attempt) * u64::from(attempt));
                tokio::time::sleep(backoff).await;
            }
            Err(err) => return Err(err),
        }
    }
}

/// Current UTC timestamp serialized in the format stored throughout the schema.
pub fn now() -> chrono::DateTime<chrono::Utc> {
    chrono::Utc::now()
}
