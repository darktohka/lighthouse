//! App passwords: per-user registry credentials that bypass TOTP.
//!
//! The plaintext `lhp_<hex>` value is shown exactly once at creation. Only its
//! SHA-256 is persisted, indexed by `(user_id, token_hash)`, so verification is a
//! single indexed lookup plus a constant-time compare — the secret is 192 bits of
//! randomness, so the slow Argon2 path is unnecessary and would turn every Basic
//! request into a CPU-denial vector.

use axum::Router;
use chrono::Utc;

use crate::auth::tokens;
use crate::db::{self, Db};
use crate::error::{ApiError, ApiResult};
use crate::models::AppPassword;
use crate::state::AppState;

/// Lists a user's app passwords, oldest first.
pub async fn list(db: &Db, user_id: i64) -> ApiResult<Vec<AppPassword>> {
    let accounts = sqlx::query_as::<_, AppPassword>(
        "SELECT * FROM app_passwords WHERE user_id = ? ORDER BY created_at, id",
    )
    .bind(user_id)
    .fetch_all(db)
    .await?;
    Ok(accounts)
}

/// Loads an app password by id, scoped to its owner.
pub async fn find_for_user(db: &Db, user_id: i64, id: i64) -> ApiResult<Option<AppPassword>> {
    let account =
        sqlx::query_as::<_, AppPassword>("SELECT * FROM app_passwords WHERE id = ? AND user_id = ?")
            .bind(id)
            .bind(user_id)
            .fetch_optional(db)
            .await?;
    Ok(account)
}

/// Creates an app password and returns it with the plaintext, shown only here.
pub async fn create(state: &AppState, user_id: i64, name: &str) -> ApiResult<(AppPassword, String)> {
    let name = name.trim();
    if name.is_empty() {
        return Err(ApiError::bad_request("name is required"));
    }
    if name.chars().count() > 100 {
        return Err(ApiError::bad_request("name must be at most 100 characters"));
    }
    let name_taken: Option<i64> =
        sqlx::query_scalar("SELECT 1 FROM app_passwords WHERE user_id = ? AND name = ? LIMIT 1")
            .bind(user_id)
            .bind(name)
            .fetch_optional(&state.db)
            .await?;
    if name_taken.is_some() {
        return Err(ApiError::conflict("an app password with that name already exists"));
    }

    let (plaintext, prefix, suffix, hash) = tokens::generate_app_password_token();
    let now = Utc::now();
    sqlx::query(
        "INSERT INTO app_passwords \
         (user_id, name, token_prefix, token_suffix, token_hash, created_at, last_used_at) \
         VALUES (?, ?, ?, ?, ?, ?, NULL)",
    )
    .bind(user_id)
    .bind(name)
    .bind(&prefix)
    .bind(&suffix)
    .bind(&hash)
    .bind(now)
    .execute(&state.db)
    .await?;

    let created = sqlx::query_as::<_, AppPassword>(
        "SELECT * FROM app_passwords WHERE user_id = ? AND name = ? LIMIT 1",
    )
    .bind(user_id)
    .bind(name)
    .fetch_one(&state.db)
    .await?;

    Ok((created, plaintext))
}

/// Verifies a presented secret against a user's app passwords and records its
/// use. Returns `None` when no app password matches.
pub async fn authenticate(
    state: &AppState,
    user_id: i64,
    candidate: &str,
) -> ApiResult<Option<AppPassword>> {
    let hash = tokens::hash_token(candidate);
    let account = sqlx::query_as::<_, AppPassword>(
        "SELECT * FROM app_passwords WHERE user_id = ? AND token_hash = ? LIMIT 1",
    )
    .bind(user_id)
    .bind(&hash)
    .fetch_optional(&state.db)
    .await?;

    let Some(account) = account else {
        return Ok(None);
    };
    if !tokens::verify_token_hash(&account.token_hash, candidate) {
        return Ok(None);
    }

    let now = Utc::now();
    db::with_busy_retry(|| async {
        sqlx::query("UPDATE app_passwords SET last_used_at = ? WHERE id = ?")
            .bind(now)
            .bind(account.id)
            .execute(&state.db)
            .await?;
        Ok::<(), sqlx::Error>(())
    })
    .await
    .map_err(ApiError::from)?;

    let mut account = account;
    account.last_used_at = Some(now);
    Ok(Some(account))
}

/// Replaces an app password's secret, returning the new plaintext once. `None`
/// when the credential does not exist or is not owned by `user_id`.
pub async fn rotate(
    state: &AppState,
    user_id: i64,
    id: i64,
) -> ApiResult<Option<(AppPassword, String)>> {
    if find_for_user(&state.db, user_id, id).await?.is_none() {
        return Ok(None);
    }
    let (plaintext, prefix, suffix, hash) = tokens::generate_app_password_token();
    sqlx::query(
        "UPDATE app_passwords \
         SET token_prefix = ?, token_suffix = ?, token_hash = ?, last_used_at = NULL \
         WHERE id = ? AND user_id = ?",
    )
    .bind(&prefix)
    .bind(&suffix)
    .bind(&hash)
    .bind(id)
    .bind(user_id)
    .execute(&state.db)
    .await?;

    let account = find_for_user(&state.db, user_id, id)
        .await?
        .ok_or_else(|| ApiError::not_found("app password not found"))?;
    Ok(Some((account, plaintext)))
}

/// Deletes an app password. Returns `false` when it does not exist or is not
/// owned by `user_id`.
pub async fn delete(state: &AppState, user_id: i64, id: i64) -> ApiResult<bool> {
    let result = sqlx::query("DELETE FROM app_passwords WHERE id = ? AND user_id = ?")
        .bind(id)
        .bind(user_id)
        .execute(&state.db)
        .await?;
    Ok(result.rows_affected() > 0)
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
    async fn creates_and_authenticates_app_passwords() {
        let (_dir, state) = test_state().await;
        let user = create_user(&state, "alice").await;

        let (account, secret) = create(&state, user.id, "laptop").await.expect("create");
        assert!(secret.starts_with("lhp_"));
        assert_eq!(account.token_prefix, "lhp_");
        assert!(secret.ends_with(&account.token_suffix));
        assert_eq!(account.token_suffix.len(), 4);

        let matched = authenticate(&state, user.id, &secret)
            .await
            .expect("authenticate")
            .expect("found");
        assert_eq!(matched.id, account.id);
        assert!(matched.last_used_at.is_some());

        assert!(
            authenticate(&state, user.id, "lhp_wrong")
                .await
                .expect("authenticate")
                .is_none()
        );
        assert!(
            authenticate(&state, user.id + 1, &secret)
                .await
                .expect("authenticate")
                .is_none(),
            "app passwords are scoped to their owner"
        );
    }

    #[tokio::test]
    async fn duplicate_names_conflict_and_delete_is_scoped() {
        let (_dir, state) = test_state().await;
        let alice = create_user(&state, "alice").await;
        let bob = create_user(&state, "bob").await;

        let (account, _) = create(&state, alice.id, "ci").await.expect("create");
        assert!(create(&state, alice.id, "ci").await.is_err());

        assert!(!delete(&state, bob.id, account.id).await.expect("delete"));
        assert!(delete(&state, alice.id, account.id).await.expect("delete"));
        assert!(list(&state.db, alice.id).await.expect("list").is_empty());
    }
}
