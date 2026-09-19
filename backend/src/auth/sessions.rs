//! Cookie and refresh-token session persistence.
//!
//! A session row is the refresh-token family: its `id` is also the `jti` of the
//! access tokens minted for it, so `logout` can revoke it from the cookie alone.
//! Only the SHA-256 hash of the refresh token is stored.

use axum::Router;
use axum::http::{HeaderMap, HeaderValue, header};
use chrono::{Duration, Utc};

use crate::auth::tokens;
use crate::config::Config;
use crate::db::{self, Db};
use crate::error::{ApiError, ApiResult};
use crate::models::{Session, User};
use crate::state::AppState;

/// Number of random bytes in a session identifier (rendered as 64 hex chars).
const SESSION_ID_BYTES: usize = 32;

/// A rotated session's freshly minted credentials.
#[derive(Debug, Clone)]
pub struct Rotated {
    pub access_token: String,
    pub refresh_token: String,
}

/// Audit fields captured when a session is created.
#[derive(Debug, Clone, Copy, Default)]
pub struct SessionAudit<'a> {
    pub user_agent: Option<&'a str>,
    pub ip: Option<&'a str>,
}

/// Creates a session for `user` and returns it together with the plaintext
/// refresh token.
pub async fn create(
    state: &AppState,
    user: &User,
    audit: SessionAudit<'_>,
) -> ApiResult<(Session, String)> {
    let mut raw_id = [0u8; SESSION_ID_BYTES];
    rand::fill(&mut raw_id);
    let id = hex::encode(raw_id);
    let refresh_token = tokens::generate_refresh_token();
    let refresh_hash = tokens::hash_token(&refresh_token);
    let now = Utc::now();
    let expires_at = now + Duration::seconds(state.config.refresh_token_ttl_secs);

    delete_expired(&state.db).await;

    sqlx::query(
        "INSERT INTO sessions \
         (id, user_id, refresh_token_hash, user_agent, ip, created_at, last_seen_at, expires_at, revoked_at) \
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, NULL)",
    )
    .bind(&id)
    .bind(user.id)
    .bind(&refresh_hash)
    .bind(audit.user_agent)
    .bind(audit.ip)
    .bind(now)
    .bind(now)
    .bind(expires_at)
    .execute(&state.db)
    .await?;

    let session = Session {
        id,
        user_id: user.id,
        refresh_token_hash: Some(refresh_hash),
        user_agent: audit.user_agent.map(str::to_string),
        ip: audit.ip.map(str::to_string),
        created_at: now,
        last_seen_at: now,
        expires_at,
        revoked_at: None,
    };
    Ok((session, refresh_token))
}

/// Builds the `Set-Cookie` header that installs the access token.
pub fn set_access_cookie(config: &Config, token: &str) -> ApiResult<HeaderValue> {
    let cookie = render_cookie(
        config,
        token,
        config.access_token_ttl_secs,
        !token.is_empty(),
    );
    HeaderValue::from_str(&cookie).map_err(|_| ApiError::internal("invalid cookie value"))
}

/// Builds the `Set-Cookie` header that clears the access token.
pub fn clear_access_cookie(config: &Config) -> ApiResult<HeaderValue> {
    let cookie = render_cookie(config, "", 0, false);
    HeaderValue::from_str(&cookie).map_err(|_| ApiError::internal("invalid cookie value"))
}

fn render_cookie(config: &Config, value: &str, max_age: i64, keep: bool) -> String {
    let mut cookie = format!(
        "{}={}; Path=/; HttpOnly; SameSite=Lax; Max-Age={}",
        config.cookie_name,
        value,
        if keep { max_age } else { 0 }
    );
    if config.cookie_secure {
        cookie.push_str("; Secure");
    }
    if let Some(domain) = &config.cookie_domain {
        cookie.push_str("; Domain=");
        cookie.push_str(domain);
    }
    cookie
}

/// Reads the access token from the request's `Cookie` header.
pub fn access_token_from_headers(headers: &HeaderMap, config: &Config) -> Option<String> {
    let raw = headers.get(header::COOKIE)?.to_str().ok()?;
    for pair in raw.split(';') {
        let pair = pair.trim();
        let Some((name, value)) = pair.split_once('=') else {
            continue;
        };
        if name == config.cookie_name {
            return Some(value.to_string());
        }
    }
    None
}

/// Rotates the refresh token of `session_id`, returning a new access/refresh
/// pair on success.
pub async fn rotate_refresh(
    state: &AppState,
    session_id: &str,
    presented_token: &str,
) -> ApiResult<Rotated> {
    let session = find(&state.db, session_id)
        .await?
        .filter(is_active)
        .ok_or_else(|| ApiError::unauthorized("session is not valid"))?;

    let stored = session
        .refresh_token_hash
        .as_deref()
        .ok_or_else(|| ApiError::unauthorized("session has no refresh token"))?;
    if !tokens::verify_token_hash(stored, presented_token) {
        return Err(ApiError::unauthorized("refresh token is not valid"));
    }

    let user = sqlx::query_as::<_, User>("SELECT * FROM users WHERE id = ? LIMIT 1")
        .bind(session.user_id)
        .fetch_optional(&state.db)
        .await?
        .ok_or_else(|| ApiError::unauthorized("account no longer exists"))?;

    let refresh_token = tokens::generate_refresh_token();
    let refresh_hash = tokens::hash_token(&refresh_token);
    let access_token =
        tokens::issue_access_token(&state.config, user.id, &user.username, &session.id)?;
    let now = Utc::now();

    let rotated = db::with_busy_retry(|| async {
        sqlx::query(
            "UPDATE sessions SET refresh_token_hash = ?, last_seen_at = ? \
             WHERE id = ? AND refresh_token_hash = ? AND revoked_at IS NULL",
        )
        .bind(&refresh_hash)
        .bind(now)
        .bind(&session.id)
        .bind(stored)
        .execute(&state.db)
        .await
    })
    .await
    .map_err(ApiError::from)?;

    if rotated.rows_affected() != 1 {
        return Err(ApiError::unauthorized("refresh token is not valid"));
    }

    Ok(Rotated {
        access_token,
        refresh_token,
    })
}

/// Revokes a single session. Returns `false` when it was already revoked or
/// missing.
pub async fn revoke(state: &AppState, session_id: &str) -> ApiResult<bool> {
    let result =
        sqlx::query("UPDATE sessions SET revoked_at = ? WHERE id = ? AND revoked_at IS NULL")
            .bind(Utc::now())
            .bind(session_id)
            .execute(&state.db)
            .await?;
    Ok(result.rows_affected() > 0)
}

/// Revokes every active session belonging to `user_id`.
pub async fn revoke_all_for_user(state: &AppState, user_id: i64) -> ApiResult<u64> {
    let result =
        sqlx::query("UPDATE sessions SET revoked_at = ? WHERE user_id = ? AND revoked_at IS NULL")
            .bind(Utc::now())
            .bind(user_id)
            .execute(&state.db)
            .await?;
    Ok(result.rows_affected())
}

/// Revokes every active session except `keep`, so a factor change does not log
/// the caller out of the session they are using.
pub async fn revoke_all_except(
    state: &AppState,
    user_id: i64,
    keep: Option<&str>,
) -> ApiResult<u64> {
    let Some(keep) = keep else {
        return revoke_all_for_user(state, user_id).await;
    };
    let result = sqlx::query(
        "UPDATE sessions SET revoked_at = ? \
         WHERE user_id = ? AND revoked_at IS NULL AND id != ?",
    )
    .bind(Utc::now())
    .bind(user_id)
    .bind(keep)
    .execute(&state.db)
    .await?;
    Ok(result.rows_affected())
}

/// Lists a user's sessions, most recently seen first.
pub async fn list_for_user(db: &Db, user_id: i64) -> ApiResult<Vec<Session>> {
    let sessions = sqlx::query_as::<_, Session>(
        "SELECT * FROM sessions WHERE user_id = ? ORDER BY last_seen_at DESC",
    )
    .bind(user_id)
    .fetch_all(db)
    .await?;
    Ok(sessions)
}

/// Loads a session by id.
pub async fn find(db: &Db, session_id: &str) -> ApiResult<Option<Session>> {
    let session = sqlx::query_as::<_, Session>("SELECT * FROM sessions WHERE id = ? LIMIT 1")
        .bind(session_id)
        .fetch_optional(db)
        .await?;
    Ok(session)
}

/// Loads the active session holding `refresh_token`.
pub async fn find_by_refresh_token(db: &Db, refresh_token: &str) -> ApiResult<Option<Session>> {
    let hash = tokens::hash_token(refresh_token);
    let session =
        sqlx::query_as::<_, Session>("SELECT * FROM sessions WHERE refresh_token_hash = ? LIMIT 1")
            .bind(&hash)
            .fetch_optional(db)
            .await?;
    Ok(session.filter(is_active))
}

/// A session is active while it is neither revoked nor expired.
pub fn is_active(session: &Session) -> bool {
    session.revoked_at.is_none() && session.expires_at > Utc::now()
}

/// Removes expired session rows.
pub async fn delete_expired(db: &Db) {
    if let Err(err) = sqlx::query("DELETE FROM sessions WHERE expires_at <= ?")
        .bind(Utc::now())
        .execute(db)
        .await
    {
        tracing::warn!(error = %err, "failed to prune expired sessions");
    }
}

/// Empty router following the submodule convention.
pub fn router() -> Router<AppState> {
    Router::new()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::test_support::{test_config, test_state};

    fn config() -> Config {
        let dir = tempfile::tempdir().expect("tempdir");
        test_config(dir.path())
    }

    #[test]
    fn access_cookie_has_required_attributes() {
        let mut config = config();
        config.cookie_secure = true;
        config.cookie_domain = Some("example.com".to_string());
        let value = set_access_cookie(&config, "jwt.value.here").expect("cookie");
        let rendered = value.to_str().expect("utf8");
        assert!(rendered.contains("HttpOnly"));
        assert!(rendered.contains("SameSite=Lax"));
        assert!(rendered.contains("Path=/"));
        assert!(rendered.contains("Max-Age=3600"));
        assert!(rendered.contains("Secure"));
        assert!(rendered.contains("Domain=example.com"));
    }

    #[test]
    fn clear_cookie_expires_immediately() {
        let config = config();
        let value = clear_access_cookie(&config).expect("cookie");
        let rendered = value.to_str().expect("utf8");
        assert!(rendered.contains("Max-Age=0"));
        assert!(rendered.contains(&config.cookie_name));
    }

    #[test]
    fn reads_access_token_from_headers() {
        let config = config();
        let mut headers = HeaderMap::new();
        headers.insert(
            header::COOKIE,
            HeaderValue::from_static("other=1; lighthouse_token=abc.def.ghi; more=2"),
        );
        assert_eq!(
            access_token_from_headers(&headers, &config).as_deref(),
            Some("abc.def.ghi")
        );
    }

    #[tokio::test]
    async fn creates_and_rotates_a_session() {
        let (_dir, state) = test_state().await;
        let user = crate::auth::test_support::create_user(&state, "alice").await;

        let (session, refresh) = create(
            &state,
            &user,
            SessionAudit {
                user_agent: Some("test-agent"),
                ip: Some("127.0.0.1"),
            },
        )
        .await
        .expect("create session");
        assert_eq!(session.id.len(), 64);
        assert!(tokens::verify_token_hash(
            session.refresh_token_hash.as_deref().expect("hash"),
            &refresh
        ));

        let rotated = rotate_refresh(&state, &session.id, &refresh)
            .await
            .expect("rotate");
        assert!(!rotated.access_token.is_empty());
        assert_ne!(rotated.refresh_token, refresh);

        assert!(rotate_refresh(&state, &session.id, &refresh).await.is_err());

        assert!(revoke(&state, &session.id).await.expect("revoke"));
        assert!(!revoke(&state, &session.id).await.expect("re-revoke"));
    }

    #[tokio::test]
    async fn concurrent_refresh_rotation_has_a_single_winner() {
        let (_dir, state) = test_state().await;
        let user = crate::auth::test_support::create_user(&state, "alice").await;
        let (session, refresh) = create(&state, &user, SessionAudit::default())
            .await
            .expect("create session");

        let (first, second) = tokio::join!(
            rotate_refresh(&state, &session.id, &refresh),
            rotate_refresh(&state, &session.id, &refresh),
        );
        assert!(
            first.is_ok() ^ second.is_ok(),
            "exactly one concurrent rotation must win"
        );
    }
}
