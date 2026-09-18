//! Proof-of-work captcha challenge issuance and verification.
//!
//! This is a thin wrapper over `pow-captcha-axum`: challenges are persisted in
//! the shared SQLite pool and redeemed captcha tokens are consumed through the
//! crate's own `CaptchaState`. When `CAPTCHA_ENABLED` is false every operation
//! is a pass-through.

#![allow(dead_code)]

use std::sync::OnceLock;
use std::time::Duration;

use axum::Router;
use axum::http::HeaderMap;
use pow_captcha_axum::{
    CaptchaConfig, CaptchaState, ChallengeParams, ScopeConfig, ScopeMode, ScopeSource,
};
use serde::Serialize;
use tokio::sync::OnceCell;

use crate::config::Config;
use crate::db::Db;
use crate::error::{ApiError, ApiResult};
use crate::state::AppState;

const CHALLENGE_TTL: Duration = Duration::from_secs(600);
const TOKEN_TTL: Duration = Duration::from_secs(1200);
const CHALLENGE_COUNT: u32 = 50;
const CHALLENGE_SIZE: u32 = 32;

static STATE: OnceLock<CaptchaState> = OnceLock::new();
static SCHEMA: OnceCell<()> = OnceCell::const_new();

/// Parameters a client must solve.
#[derive(Debug, Clone, Serialize)]
pub struct ChallengeDescriptor {
    pub c: u32,
    pub s: u32,
    pub d: u32,
}

/// A proof-of-work challenge issued to a client.
#[derive(Debug, Clone, Serialize)]
pub struct Challenge {
    pub challenge: ChallengeDescriptor,
    pub token: String,
    pub expires: i64,
}

/// Builds the captcha configuration from the registry's settings.
pub fn captcha_config(config: &Config) -> CaptchaConfig {
    CaptchaConfig {
        challenge: ChallengeParams {
            count: CHALLENGE_COUNT,
            size: CHALLENGE_SIZE,
            difficulty: config.captcha_difficulty,
            ttl: CHALLENGE_TTL,
        },
        token_ttl: TOKEN_TTL,
        challenge_scope: ScopeConfig {
            primary: ScopeSource::Referer,
            fallbacks: vec![ScopeSource::Origin, ScopeSource::Host],
            mode: ScopeMode::Origin,
            allow_missing: true,
        },
        token_scope: ScopeConfig {
            primary: ScopeSource::Origin,
            fallbacks: vec![ScopeSource::Referer, ScopeSource::Host],
            mode: ScopeMode::Origin,
            allow_missing: true,
        },
        allow_scope_from_body: true,
    }
}

/// Installs the captcha state during application boot.
pub fn install(db: &Db, config: &Config) {
    if !config.captcha_enabled {
        return;
    }
    let _ = STATE.set(CaptchaState::new(db.clone(), captcha_config(config)));
}

/// Returns the crate's captcha routes when captcha is enabled.
pub fn service() -> Option<Router> {
    STATE
        .get()
        .map(|state| pow_captcha_axum::captcha_router_with_state(state.clone()))
}

fn captcha_state(app: &AppState) -> Option<CaptchaState> {
    if !app.config.captcha_enabled {
        return None;
    }
    Some(
        STATE
            .get()
            .cloned()
            .unwrap_or_else(|| CaptchaState::new(app.db.clone(), captcha_config(&app.config))),
    )
}

async fn ensure_schema(db: &Db) -> ApiResult<()> {
    SCHEMA
        .get_or_try_init(|| async {
            let migrations = pow_captcha_axum::MIGRATOR.iter().cloned().collect();
            let mut migrator = sqlx::migrate::Migrator::with_migrations(migrations);
            // The crate shares sqlx's `_sqlx_migrations` table by default, which
            // collides with this binary's own migrator (`VersionMissing`). Track
            // the captcha migrations separately; the table is created on demand.
            migrator.dangerous_set_table_name("_pow_captcha_migrations");
            migrator.run(db).await.map_err(|err| {
                tracing::error!(error = %err, "failed to migrate captcha tables");
                ApiError::internal("captcha unavailable")
            })
        })
        .await
        .map(|_| ())
}

/// Issues a challenge, or `None` when captcha is disabled.
pub async fn issue(app: &AppState, headers: &HeaderMap) -> ApiResult<Option<Challenge>> {
    let Some(state) = captcha_state(app) else {
        return Ok(None);
    };
    ensure_schema(&app.db).await?;

    let now = chrono::Utc::now().timestamp_millis();
    let challenge_scope = state
        .config
        .challenge_scope
        .extract(headers)
        .unwrap_or_default();
    let token_scope = state
        .config
        .token_scope
        .extract(headers)
        .unwrap_or_default();
    let token = random_hex(25);
    let expires = now + duration_ms(state.config.challenge.ttl);

    sqlx::query(
        "INSERT INTO challenges \
         (token, challenge_count, challenge_size, challenge_difficulty, challenge_scope, token_scope, expires_at, created_at) \
         VALUES (?, ?, ?, ?, ?, ?, ?, ?) \
         ON CONFLICT(token) DO UPDATE SET \
             challenge_count = excluded.challenge_count, \
             challenge_size = excluded.challenge_size, \
             challenge_difficulty = excluded.challenge_difficulty, \
             challenge_scope = excluded.challenge_scope, \
             token_scope = excluded.token_scope, \
             expires_at = excluded.expires_at, \
             created_at = excluded.created_at",
    )
    .bind(&token)
    .bind(i64::from(state.config.challenge.count))
    .bind(i64::from(state.config.challenge.size))
    .bind(i64::from(state.config.challenge.difficulty))
    .bind(&challenge_scope)
    .bind(&token_scope)
    .bind(expires)
    .bind(now)
    .execute(&app.db)
    .await?;

    Ok(Some(Challenge {
        challenge: ChallengeDescriptor {
            c: state.config.challenge.count,
            s: state.config.challenge.size,
            d: state.config.challenge.difficulty,
        },
        token,
        expires,
    }))
}

/// Consumes a captcha token; a no-op when captcha is disabled.
pub async fn verify(app: &AppState, token: &str, headers: &HeaderMap) -> ApiResult<()> {
    let Some(state) = captcha_state(app) else {
        return Ok(());
    };
    ensure_schema(&app.db).await?;

    if token.trim().is_empty() {
        return Err(ApiError::bad_request("captcha is required"));
    }

    match state.consume_token(token, headers).await {
        Ok(true) => Ok(()),
        Ok(false) => Err(ApiError::bad_request("captcha is invalid")),
        Err(err) => {
            tracing::warn!(error = %err, "captcha verification failed");
            Err(ApiError::bad_request("captcha is invalid"))
        }
    }
}

fn duration_ms(duration: Duration) -> i64 {
    duration.as_millis().min(i64::MAX as u128) as i64
}

fn random_hex(bytes: usize) -> String {
    let mut buffer = vec![0u8; bytes];
    rand::fill(buffer.as_mut_slice());
    hex::encode(buffer)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::test_support::{test_config, test_state};

    #[tokio::test]
    async fn disabled_captcha_passes_through() {
        let (_dir, state) = test_state().await;
        let headers = HeaderMap::new();
        assert!(issue(&state, &headers).await.expect("issue").is_none());
        assert!(verify(&state, "", &headers).await.is_ok());
    }

    #[test]
    fn captcha_config_uses_configured_difficulty() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut config = test_config(dir.path());
        config.captcha_difficulty = 7;
        let captcha = captcha_config(&config);
        assert_eq!(captcha.challenge.difficulty, 7);
        assert!(captcha.challenge_scope.allow_missing);
    }
}
