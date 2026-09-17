//! TOTP two-factor authentication: enrolment, verification and recovery.
//!
//! The secret is written by [`begin_setup`] but stays inert (`totp_enabled = 0`)
//! until [`enable`] verifies a live code, so an abandoned enrolment can never
//! lock an account out. Backup codes are single-use and stored only as Argon2id
//! hashes; the plaintext exists solely in the setup response.

use chrono::{DateTime, Utc};
use serde::Serialize;

use crate::auth::password;
use crate::auth::sessions;
use crate::auth::totp;
use crate::db::Db;
use crate::error::{ApiError, ApiResult};
use crate::models::User;
use crate::state::AppState;

/// Accepted TOTP drift, in 30-second steps, in either direction.
const DRIFT_STEPS: i64 = 1;

#[derive(Debug, Serialize)]
pub struct TwoFactorStatus {
    pub enabled: bool,
    pub backup_codes_remaining: i64,
}

#[derive(Debug, Serialize)]
pub struct TwoFactorSetup {
    pub secret: String,
    pub otpauth_uri: String,
    pub backup_codes: Vec<String>,
}

/// Whether the account has TOTP enabled.
pub async fn is_enabled(db: &Db, user_id: i64) -> ApiResult<bool> {
    let enabled: Option<bool> = sqlx::query_scalar("SELECT totp_enabled FROM users WHERE id = ?")
        .bind(user_id)
        .fetch_optional(db)
        .await?;
    Ok(enabled.unwrap_or(false))
}

/// Enrolment state and remaining unused backup codes.
pub async fn status(db: &Db, user_id: i64) -> ApiResult<TwoFactorStatus> {
    let enabled = is_enabled(db, user_id).await?;
    let remaining: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM totp_backup_codes WHERE user_id = ? AND used_at IS NULL",
    )
    .bind(user_id)
    .fetch_one(db)
    .await?;
    Ok(TwoFactorStatus {
        enabled,
        backup_codes_remaining: remaining,
    })
}

async fn load_secret(db: &Db, user_id: i64) -> ApiResult<Option<String>> {
    let secret: Option<Option<String>> =
        sqlx::query_scalar("SELECT totp_secret FROM users WHERE id = ?")
            .bind(user_id)
            .fetch_optional(db)
            .await?;
    Ok(secret.flatten())
}

async fn load_username(db: &Db, user_id: i64) -> ApiResult<String> {
    let username: Option<String> = sqlx::query_scalar("SELECT username FROM users WHERE id = ?")
        .bind(user_id)
        .fetch_optional(db)
        .await?;
    username.ok_or_else(|| ApiError::not_found("user not found"))
}

/// Starts (or restarts) enrolment: a fresh secret plus a fresh set of backup
/// codes, replacing any previous unconfirmed state. TOTP stays disabled.
pub async fn begin_setup(
    state: &AppState,
    user_id: i64,
    issuer: &str,
) -> ApiResult<TwoFactorSetup> {
    let secret = totp::generate_secret();
    let codes = totp::generate_backup_codes();
    let username = load_username(&state.db, user_id).await?;
    let now = Utc::now();

    let mut tx = state.db.begin().await?;
    sqlx::query(
        "UPDATE users SET totp_secret = ?, totp_enabled = 0, totp_confirmed_at = NULL, \
         totp_last_used_step = NULL, updated_at = ? WHERE id = ?",
    )
    .bind(&secret)
    .bind(now)
    .bind(user_id)
    .execute(&mut *tx)
    .await?;
    sqlx::query("DELETE FROM totp_backup_codes WHERE user_id = ?")
        .bind(user_id)
        .execute(&mut *tx)
        .await?;
    for code in &codes {
        let hash = password::hash_password(code)?;
        sqlx::query(
            "INSERT INTO totp_backup_codes (user_id, code_hash, used_at, created_at) \
             VALUES (?, ?, NULL, ?)",
        )
        .bind(user_id)
        .bind(&hash)
        .bind(now)
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;

    Ok(TwoFactorSetup {
        otpauth_uri: totp::otpauth_uri(issuer, &username, &secret),
        secret,
        backup_codes: codes,
    })
}

/// Verifies a TOTP code and records the accepted step to reject replays. The
/// conditional update makes concurrent reuse of one code impossible.
async fn verify_totp_code(
    state: &AppState,
    user_id: i64,
    secret: &str,
    code: &str,
    now: DateTime<Utc>,
) -> ApiResult<bool> {
    let Some(secret_bytes) = totp::base32_decode(secret) else {
        return Ok(false);
    };
    let Some(step) = totp::verify_step(&secret_bytes, code, now.timestamp(), DRIFT_STEPS) else {
        return Ok(false);
    };
    let updated = sqlx::query(
        "UPDATE users SET totp_last_used_step = ? \
         WHERE id = ? AND (totp_last_used_step IS NULL OR totp_last_used_step < ?)",
    )
    .bind(step as i64)
    .bind(user_id)
    .bind(step as i64)
    .execute(&state.db)
    .await?;
    Ok(updated.rows_affected() == 1)
}

/// Consumes an unused backup code, claiming it atomically.
async fn consume_backup_code(state: &AppState, user_id: i64, code: &str) -> ApiResult<bool> {
    let Some(normalized) = totp::normalize_backup_code(code) else {
        return Ok(false);
    };
    let rows = sqlx::query_as::<_, (i64, String)>(
        "SELECT id, code_hash FROM totp_backup_codes WHERE user_id = ? AND used_at IS NULL",
    )
    .bind(user_id)
    .fetch_all(&state.db)
    .await?;

    for (id, hash) in rows {
        if !password::verify_password(&hash, &normalized) {
            continue;
        }
        let claimed = sqlx::query(
            "UPDATE totp_backup_codes SET used_at = ? WHERE id = ? AND used_at IS NULL",
        )
        .bind(Utc::now())
        .bind(id)
        .execute(&state.db)
        .await?;
        if claimed.rows_affected() == 1 {
            return Ok(true);
        }
    }
    Ok(false)
}

/// Accepts either a live TOTP code or an unused backup code.
pub async fn verify_login_code(
    state: &AppState,
    user_id: i64,
    code: &str,
    now: DateTime<Utc>,
) -> ApiResult<bool> {
    if !is_enabled(&state.db, user_id).await? {
        return Ok(false);
    }
    if let Some(secret) = load_secret(&state.db, user_id).await? {
        if verify_totp_code(state, user_id, &secret, code, now).await? {
            return Ok(true);
        }
    }
    consume_backup_code(state, user_id, code).await
}

/// Confirms enrolment with a live code and enables TOTP. Other sessions are
/// revoked so a previously stolen cookie cannot outlive the new factor.
pub async fn enable(
    state: &AppState,
    user_id: i64,
    code: &str,
    keep_session: Option<&str>,
    now: DateTime<Utc>,
) -> ApiResult<()> {
    let enabled = is_enabled(&state.db, user_id).await?;
    if enabled {
        return Err(ApiError::conflict("two-factor authentication is already enabled"));
    }
    let Some(secret) = load_secret(&state.db, user_id).await? else {
        return Err(ApiError::bad_request("start two-factor setup first"));
    };
    if !verify_totp_code(state, user_id, &secret, code, now).await? {
        return Err(ApiError::bad_request("invalid authentication code"));
    }

    let updated = sqlx::query(
        "UPDATE users SET totp_enabled = 1, totp_confirmed_at = ?, updated_at = ? \
         WHERE id = ? AND totp_enabled = 0",
    )
    .bind(now)
    .bind(now)
    .bind(user_id)
    .execute(&state.db)
    .await?;
    if updated.rows_affected() == 0 {
        return Err(ApiError::conflict("two-factor authentication is already enabled"));
    }

    sessions::revoke_all_except(state, user_id, keep_session).await?;
    Ok(())
}

/// Disables TOTP after confirming both the account password and a live TOTP or
/// backup code, then removes every trace of the factor.
pub async fn disable(
    state: &AppState,
    user: &User,
    password: &str,
    code: &str,
    keep_session: Option<&str>,
    now: DateTime<Utc>,
) -> ApiResult<()> {
    if !password::verify_password(&user.password_hash, password) {
        return Err(ApiError::new(
            axum::http::StatusCode::UNAUTHORIZED,
            "invalid_credentials",
            "current password is incorrect",
        ));
    }
    if !verify_login_code(state, user.id, code, now).await? {
        return Err(ApiError::bad_request("invalid authentication code"));
    }

    let mut tx = state.db.begin().await?;
    sqlx::query(
        "UPDATE users SET totp_enabled = 0, totp_secret = NULL, totp_confirmed_at = NULL, \
         totp_last_used_step = NULL, updated_at = ? WHERE id = ?",
    )
    .bind(now)
    .bind(user.id)
    .execute(&mut *tx)
    .await?;
    sqlx::query("DELETE FROM totp_backup_codes WHERE user_id = ?")
        .bind(user.id)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;

    sessions::revoke_all_except(state, user.id, keep_session).await?;
    Ok(())
}

/// Issues a fresh set of backup codes, invalidating the old ones. Requires a
/// live TOTP code (not a backup code) so a stolen backup code cannot rotate the
/// set.
pub async fn regenerate_backup_codes(
    state: &AppState,
    user_id: i64,
    code: &str,
    now: DateTime<Utc>,
) -> ApiResult<Vec<String>> {
    if !is_enabled(&state.db, user_id).await? {
        return Err(ApiError::bad_request("two-factor authentication is not enabled"));
    }
    let Some(secret) = load_secret(&state.db, user_id).await? else {
        return Err(ApiError::bad_request("two-factor authentication is not enabled"));
    };
    if !verify_totp_code(state, user_id, &secret, code, now).await? {
        return Err(ApiError::bad_request("invalid authentication code"));
    }

    let codes = totp::generate_backup_codes();
    let mut tx = state.db.begin().await?;
    sqlx::query("DELETE FROM totp_backup_codes WHERE user_id = ?")
        .bind(user_id)
        .execute(&mut *tx)
        .await?;
    for code in &codes {
        let hash = password::hash_password(code)?;
        sqlx::query(
            "INSERT INTO totp_backup_codes (user_id, code_hash, used_at, created_at) \
             VALUES (?, ?, NULL, ?)",
        )
        .bind(user_id)
        .bind(&hash)
        .bind(now)
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;
    Ok(codes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::test_support::{create_user, test_state};
    use chrono::Duration;

    async fn current_code(state: &AppState, user_id: i64) -> String {
        code_at(state, user_id, Utc::now()).await
    }

    async fn code_at(state: &AppState, user_id: i64, at: DateTime<Utc>) -> String {
        let secret = load_secret(&state.db, user_id).await.expect("secret").expect("set");
        let bytes = totp::base32_decode(&secret).expect("decode");
        totp::format_code(totp::totp(&bytes, at.timestamp()))
    }

    #[tokio::test]
    async fn setup_then_enable_flow() {
        let (_dir, state) = test_state().await;
        let user = create_user(&state, "alice").await;
        assert!(!is_enabled(&state.db, user.id).await.expect("enabled"));

        let setup = begin_setup(&state, user.id, "Lighthouse").await.expect("setup");
        assert_eq!(setup.backup_codes.len(), 8);
        assert!(setup.otpauth_uri.starts_with("otpauth://totp/Lighthouse:alice?"));
        assert!(!is_enabled(&state.db, user.id).await.expect("enabled"));

        let code = current_code(&state, user.id).await;
        enable(&state, user.id, &code, None, Utc::now()).await.expect("enable");
        assert!(is_enabled(&state.db, user.id).await.expect("enabled"));

        let status = status(&state.db, user.id).await.expect("status");
        assert!(status.enabled);
        assert_eq!(status.backup_codes_remaining, 8);
    }

    #[tokio::test]
    async fn enable_rejects_bad_code_and_double_enable() {
        let (_dir, state) = test_state().await;
        let user = create_user(&state, "alice").await;
        begin_setup(&state, user.id, "Lighthouse").await.expect("setup");

        assert!(enable(&state, user.id, "000000", None, Utc::now()).await.is_err());
        let code = current_code(&state, user.id).await;
        enable(&state, user.id, &code, None, Utc::now()).await.expect("enable");
        assert!(enable(&state, user.id, &code, None, Utc::now()).await.is_err());
    }

    #[tokio::test]
    async fn totp_code_cannot_be_replayed() {
        let (_dir, state) = test_state().await;
        let user = create_user(&state, "alice").await;
        let now = Utc::now();
        begin_setup(&state, user.id, "Lighthouse").await.expect("setup");
        let enable_code = code_at(&state, user.id, now).await;
        enable(&state, user.id, &enable_code, None, now).await.expect("enable");

        let later = now + Duration::seconds(totp::PERIOD as i64);
        let login_code = code_at(&state, user.id, later).await;
        assert!(
            verify_login_code(&state, user.id, &login_code, later)
                .await
                .expect("verify"),
            "first use of a fresh code succeeds"
        );
        assert!(
            !verify_login_code(&state, user.id, &login_code, later)
                .await
                .expect("verify"),
            "replay of the same code is rejected"
        );
    }

    #[tokio::test]
    async fn backup_codes_are_single_use() {
        let (_dir, state) = test_state().await;
        let user = create_user(&state, "alice").await;
        let setup = begin_setup(&state, user.id, "Lighthouse").await.expect("setup");
        let code = current_code(&state, user.id).await;
        enable(&state, user.id, &code, None, Utc::now()).await.expect("enable");

        let backup = &setup.backup_codes[0];
        assert!(verify_login_code(&state, user.id, backup, Utc::now()).await.expect("verify"));
        assert!(!verify_login_code(&state, user.id, backup, Utc::now()).await.expect("verify"));
        let status = status(&state.db, user.id).await.expect("status");
        assert_eq!(status.backup_codes_remaining, 7);
    }

    #[tokio::test]
    async fn disable_requires_password_and_clears_state() {
        let (_dir, state) = test_state().await;
        let user = create_user(&state, "alice").await;
        let now = Utc::now();
        begin_setup(&state, user.id, "Lighthouse").await.expect("setup");
        let enable_code = code_at(&state, user.id, now).await;
        enable(&state, user.id, &enable_code, None, now).await.expect("enable");

        let fresh = sqlx::query_as::<_, User>("SELECT * FROM users WHERE id = ?")
            .bind(user.id)
            .fetch_one(&state.db)
            .await
            .expect("user");

        assert!(
            disable(&state, &fresh, "wrong-password", &enable_code, None, now)
                .await
                .is_err()
        );

        let later = now + Duration::seconds(totp::PERIOD as i64);
        let disable_code = code_at(&state, user.id, later).await;
        disable(
            &state,
            &fresh,
            "correct-horse-battery",
            &disable_code,
            None,
            later,
        )
        .await
        .expect("disable");

        assert!(!is_enabled(&state.db, user.id).await.expect("enabled"));
        assert!(load_secret(&state.db, user.id).await.expect("secret").is_none());
        assert_eq!(status(&state.db, user.id).await.expect("status").backup_codes_remaining, 0);
    }
}
