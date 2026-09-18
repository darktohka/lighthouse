//! Registry refresh tokens ("offline tokens").
//!
//! `docker login` requests an offline token and stores the returned
//! `refresh_token` as `identitytoken`; the daemon then trades it for short-lived
//! bearer tokens instead of re-sending the account password. The Docker daemon
//! reuses that same secret for the life of the login rather than persisting a
//! rotated one, so the secret is stable: it is hashed at rest, expires, and is
//! revoked whenever the underlying credential changes.

use chrono::{DateTime, Duration, Utc};
use sqlx::FromRow;

use crate::auth::registry::{self, Scope};
use crate::auth::tokens::{self, RegistryAccess};
use crate::error::{ApiError, ApiResult};
use crate::state::{AppState, AuthContext, CredentialSource};

pub const SOURCE_PASSWORD: &str = "password";
pub const SOURCE_APP_PASSWORD: &str = "app_password";
pub const SOURCE_SERVICE_ACCOUNT: &str = "service_account";

/// `ApiError.code` carried by every rejected refresh redemption, so the token
/// endpoint can answer with an OAuth2 error body instead of a control-plane one.
pub const INVALID_GRANT: &str = "invalid_grant";

/// The identity a refresh token speaks for.
#[derive(Debug, Clone)]
pub struct Principal {
    /// Value copied into the bearer token's `sub` claim (`0` for a service
    /// account, matching the token endpoint).
    pub subject: i64,
    pub username: String,
    pub source: &'static str,
    pub user_id: Option<i64>,
    pub app_password_id: Option<i64>,
    pub service_account_id: Option<i64>,
}

impl Principal {
    /// Derives the refresh principal from an authenticated request context.
    /// Anonymous callers and registry bearer tokens have none.
    pub fn from_auth_context(ctx: &AuthContext) -> Option<Self> {
        match ctx.credential {
            CredentialSource::Password => Some(Self {
                subject: ctx.user_id?,
                username: ctx.username.clone()?,
                source: SOURCE_PASSWORD,
                user_id: ctx.user_id,
                app_password_id: None,
                service_account_id: None,
            }),
            CredentialSource::AppPassword => Some(Self {
                subject: ctx.user_id?,
                username: ctx.username.clone()?,
                source: SOURCE_APP_PASSWORD,
                user_id: ctx.user_id,
                app_password_id: ctx.app_password_id,
                service_account_id: None,
            }),
            CredentialSource::ServiceAccount => Some(Self {
                subject: 0,
                username: ctx.username.clone()?,
                source: SOURCE_SERVICE_ACCOUNT,
                user_id: None,
                app_password_id: None,
                service_account_id: Some(ctx.service_account_id?),
            }),
            _ => None,
        }
    }
}

/// A stored refresh token, without the secret.
#[derive(Debug, Clone, FromRow)]
pub struct RefreshRecord {
    pub id: i64,
    pub user_id: Option<i64>,
    pub app_password_id: Option<i64>,
    pub service_account_id: Option<i64>,
    pub subject: i64,
    pub username: String,
    pub scope: String,
    pub expires_at: DateTime<Utc>,
    pub revoked_at: Option<DateTime<Utc>>,
}

/// The result of redeeming a refresh token.
pub struct Redeemed {
    pub subject: i64,
    pub username: String,
    pub service_account_id: Option<i64>,
    pub access: Vec<RegistryAccess>,
}

fn invalid_grant() -> ApiError {
    ApiError::new(
        axum::http::StatusCode::BAD_REQUEST,
        INVALID_GRANT,
        "refresh token is invalid, expired or revoked",
    )
}

/// Issues a refresh token for an authenticated principal. The plaintext is
/// returned exactly once and never stored.
pub async fn issue(
    state: &AppState,
    principal: &Principal,
    scopes: &[String],
    client_id: Option<&str>,
) -> ApiResult<String> {
    let now = Utc::now();
    let token = tokens::generate_refresh_token();
    let expires_at = now + Duration::seconds(state.config.registry_refresh_token_ttl_secs);

    sqlx::query(
        "INSERT INTO registry_refresh_tokens \
         (token_hash, credential_source, user_id, app_password_id, service_account_id, \
          subject, username, scope, client_id, created_at, expires_at) \
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(tokens::hash_token(&token))
    .bind(principal.source)
    .bind(principal.user_id)
    .bind(principal.app_password_id)
    .bind(principal.service_account_id)
    .bind(principal.subject)
    .bind(&principal.username)
    .bind(scopes.join(" "))
    .bind(client_id)
    .bind(now)
    .bind(expires_at)
    .execute(&state.db)
    .await?;

    Ok(token)
}

/// Redeems a refresh token. The secret is not rotated — the client keeps using
/// the same value — but expiry and revocation are enforced on every use.
pub async fn redeem(
    state: &AppState,
    presented: &str,
    requested: &[String],
) -> ApiResult<Redeemed> {
    let now = Utc::now();
    let Some(record) = load(state, &tokens::hash_token(presented)).await? else {
        return Err(invalid_grant());
    };
    if record.revoked_at.is_some() || record.expires_at <= now {
        return Err(invalid_grant());
    }

    let access = resolve_access(state, &record, requested).await;

    let updated = sqlx::query(
        "UPDATE registry_refresh_tokens SET last_used_at = ? \
         WHERE id = ? AND revoked_at IS NULL AND expires_at > ?",
    )
    .bind(now)
    .bind(record.id)
    .bind(now)
    .execute(&state.db)
    .await?;
    if updated.rows_affected() == 0 {
        return Err(invalid_grant());
    }

    Ok(Redeemed {
        subject: record.subject,
        username: record.username,
        service_account_id: record.service_account_id,
        access,
    })
}

pub async fn revoke_for_user(state: &AppState, user_id: i64) -> ApiResult<u64> {
    let result = sqlx::query(
        "UPDATE registry_refresh_tokens SET revoked_at = ? \
         WHERE user_id = ? AND revoked_at IS NULL",
    )
    .bind(Utc::now())
    .bind(user_id)
    .execute(&state.db)
    .await?;
    Ok(result.rows_affected())
}

pub async fn revoke_for_app_password(state: &AppState, app_password_id: i64) -> ApiResult<u64> {
    let result = sqlx::query(
        "UPDATE registry_refresh_tokens SET revoked_at = ? \
         WHERE app_password_id = ? AND revoked_at IS NULL",
    )
    .bind(Utc::now())
    .bind(app_password_id)
    .execute(&state.db)
    .await?;
    Ok(result.rows_affected())
}

pub async fn revoke_for_service_account(
    state: &AppState,
    service_account_id: i64,
) -> ApiResult<u64> {
    let result = sqlx::query(
        "UPDATE registry_refresh_tokens SET revoked_at = ? \
         WHERE service_account_id = ? AND revoked_at IS NULL",
    )
    .bind(Utc::now())
    .bind(service_account_id)
    .execute(&state.db)
    .await?;
    Ok(result.rows_affected())
}

async fn load(state: &AppState, hash: &str) -> ApiResult<Option<RefreshRecord>> {
    Ok(sqlx::query_as::<_, RefreshRecord>(
        "SELECT id, user_id, app_password_id, service_account_id, subject, username, \
                scope, expires_at, revoked_at \
         FROM registry_refresh_tokens WHERE token_hash = ? LIMIT 1",
    )
    .bind(hash)
    .fetch_optional(&state.db)
    .await?)
}

/// Re-derives the granted access for the requested scopes, never widening past
/// the scopes the token was first issued for and always honouring the current
/// permission state (so a revoked grant stops working at the next refresh).
async fn resolve_access(
    state: &AppState,
    record: &RefreshRecord,
    requested: &[String],
) -> Vec<RegistryAccess> {
    let actor = AuthContext {
        user_id: record.user_id,
        username: Some(record.username.clone()),
        service_account_id: record.service_account_id,
        app_password_id: record.app_password_id,
        is_admin: false,
        credential: CredentialSource::RegistryToken,
        session_id: None,
    };
    let original: Vec<Scope> = registry::parse_scopes(std::slice::from_ref(&record.scope));
    let scopes: Vec<Scope> = if requested.is_empty() {
        original.clone()
    } else {
        registry::parse_scopes(requested)
    };

    let mut access = Vec::new();
    for scope in &scopes {
        let permitted = original.is_empty()
            || original
                .iter()
                .any(|entry| entry.kind == scope.kind && entry.name == scope.name);
        if !permitted {
            continue;
        }
        let actions = registry::granted_actions(state, &actor, scope).await;
        if !actions.is_empty() {
            access.push(RegistryAccess {
                kind: scope.kind.clone(),
                name: scope.name.clone(),
                actions,
            });
        }
    }
    access
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::test_support::{create_user, test_state};

    fn principal(user_id: i64, username: &str) -> Principal {
        Principal {
            subject: user_id,
            username: username.to_string(),
            source: SOURCE_PASSWORD,
            user_id: Some(user_id),
            app_password_id: None,
            service_account_id: None,
        }
    }

    async fn token_count(state: &AppState) -> i64 {
        sqlx::query_scalar("SELECT COUNT(*) FROM registry_refresh_tokens")
            .fetch_one(&state.db)
            .await
            .expect("count")
    }

    #[tokio::test]
    async fn issues_a_stable_token_that_can_be_reused() {
        let (_dir, state) = test_state().await;
        let user = create_user(&state, "alice").await;
        let scopes = vec!["repository:alice/app:pull".to_string()];

        let token = issue(
            &state,
            &principal(user.id, "alice"),
            &scopes,
            Some("docker"),
        )
        .await
        .expect("issue");

        let first = redeem(&state, &token, &[]).await.expect("first redeem");
        let second = redeem(&state, &token, &[]).await.expect("second redeem");
        assert_eq!(first.subject, second.subject);
        assert_eq!(token_count(&state).await, 1, "the secret never rotates");
    }

    #[tokio::test]
    async fn revoked_and_expired_tokens_are_rejected() {
        let (_dir, state) = test_state().await;
        let user = create_user(&state, "bob").await;
        let token = issue(
            &state,
            &principal(user.id, "bob"),
            &["repository:bob/app:pull".to_string()],
            None,
        )
        .await
        .expect("issue");

        assert_eq!(revoke_for_user(&state, user.id).await.expect("revoke"), 1);
        assert!(redeem(&state, &token, &[]).await.is_err());

        let (_dir, state) = test_state().await;
        let user = create_user(&state, "carol").await;
        let token = issue(
            &state,
            &principal(user.id, "carol"),
            &["repository:carol/app:pull".to_string()],
            None,
        )
        .await
        .expect("issue");
        sqlx::query("UPDATE registry_refresh_tokens SET expires_at = ?")
            .bind(Utc::now() - Duration::seconds(1))
            .execute(&state.db)
            .await
            .expect("expire");
        assert!(redeem(&state, &token, &[]).await.is_err());
    }

    #[tokio::test]
    async fn requested_scope_cannot_exceed_the_original_grant() {
        let (_dir, state) = test_state().await;
        let user = create_user(&state, "dave").await;
        let token = issue(
            &state,
            &principal(user.id, "dave"),
            &["repository:dave/app:pull".to_string()],
            None,
        )
        .await
        .expect("issue");

        let redeemed = redeem(
            &state,
            &token,
            &["repository:someone/else:pull".to_string()],
        )
        .await
        .expect("redeem");
        assert!(redeemed.access.is_empty());
    }
}
