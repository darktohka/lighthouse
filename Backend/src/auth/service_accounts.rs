//! Service-account credentials for registry clients.
//!
//! A service account authenticates with HTTP Basic auth: the username selects
//! the row and the password is the plaintext token. Only an Argon2id hash is
//! stored, so verification is deliberately the slow path.

use axum::Router;
use chrono::Utc;

use crate::auth::tokens;
use crate::db::{self, Db};
use crate::error::{ApiError, ApiResult};
use crate::models::ServiceAccount;
use crate::state::AppState;

/// Looks up a service account by its login username.
pub async fn find_by_username(db: &Db, username: &str) -> ApiResult<Option<ServiceAccount>> {
    let account = sqlx::query_as::<_, ServiceAccount>(
        "SELECT * FROM service_accounts WHERE username = ? COLLATE NOCASE LIMIT 1",
    )
    .bind(username)
    .fetch_optional(db)
    .await?;
    Ok(account)
}

/// Verifies a plaintext token against a service account and records its use.
pub async fn authenticate(
    state: &AppState,
    username: &str,
    candidate: &str,
) -> ApiResult<Option<ServiceAccount>> {
    let Some(account) = find_by_username(&state.db, username).await? else {
        return Ok(None);
    };
    if !tokens::verify_service_token(&account.token_hash, candidate) {
        return Ok(None);
    }

    let now = Utc::now();
    db::with_busy_retry(|| async {
        sqlx::query("UPDATE service_accounts SET last_used_at = ? WHERE id = ?")
            .bind(now)
            .bind(account.id)
            .execute(&state.db)
            .await?;
        Ok::<(), sqlx::Error>(())
    })
    .await
    .map_err(ApiError::from)?;

    Ok(Some(account))
}

/// Fields required to mint a service account.
#[derive(Debug, Clone, Copy)]
pub struct NewServiceAccount<'a> {
    pub owner_user_id: i64,
    pub name: &'a str,
    pub username: &'a str,
    pub description: Option<&'a str>,
}

/// Creates a service account and returns it with the plaintext token, which is
/// only ever available at creation time.
pub async fn create(
    state: &AppState,
    account: NewServiceAccount<'_>,
) -> ApiResult<(ServiceAccount, String)> {
    if account.name.trim().is_empty() || account.username.trim().is_empty() {
        return Err(ApiError::bad_request("name and username are required"));
    }
    let (plaintext, prefix, suffix, hash) = tokens::generate_service_token();
    let now = Utc::now();

    sqlx::query(
        "INSERT INTO service_accounts \
         (owner_user_id, name, username, description, token_prefix, token_suffix, token_hash, created_at, last_used_at) \
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, NULL)",
    )
    .bind(account.owner_user_id)
    .bind(account.name)
    .bind(account.username)
    .bind(account.description)
    .bind(&prefix)
    .bind(&suffix)
    .bind(&hash)
    .bind(now)
    .execute(&state.db)
    .await?;

    let created = sqlx::query_as::<_, ServiceAccount>(
        "SELECT * FROM service_accounts WHERE owner_user_id = ? AND name = ? LIMIT 1",
    )
    .bind(account.owner_user_id)
    .bind(account.name)
    .fetch_one(&state.db)
    .await?;

    Ok((created, plaintext))
}

/// Empty router following the submodule convention.
pub fn router() -> Router<AppState> {
    Router::new()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::test_support::{create_user, test_state};

    #[tokio::test]
    async fn verifies_a_service_token() {
        let (_dir, state) = test_state().await;
        let user = create_user(&state, "owner").await;

        let (account, token) = create(
            &state,
            NewServiceAccount {
                owner_user_id: user.id,
                name: "ci",
                username: "owner-ci",
                description: Some("CI"),
            },
        )
        .await
        .expect("create");
        assert_eq!(account.token_prefix.len(), 3);
        assert_eq!(account.token_suffix.len(), 3);

        let authenticated = authenticate(&state, "owner-ci", &token)
            .await
            .expect("authenticate")
            .expect("found");
        assert_eq!(authenticated.id, account.id);

        assert!(
            authenticate(&state, "owner-ci", "lhr_wrong")
                .await
                .expect("authenticate")
                .is_none()
        );
        assert!(
            authenticate(&state, "missing", &token)
                .await
                .expect("authenticate")
                .is_none()
        );
    }
}
