//! Service-account management API (control-plane surface).
//!
//! Credential minting reuses [`crate::auth::service_accounts`]; the plaintext
//! token is returned exactly once on create and rotate. Listings only ever
//! expose the 3-character prefix and suffix.

use axum::Json;
use axum::Router;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::auth::middleware::Authenticated;
use crate::auth::{ip_ranges, service_accounts as registry_accounts, tokens};
use crate::error::{ApiError, ApiResult};
use crate::models::{ServiceAccount, ServiceAccountCategory, ServiceAccountIpRange};
use crate::state::{AppState, AuthContext};

use super::{is_namespace_admin, load_namespace, namespace_by_id};

#[derive(Debug, Deserialize)]
struct CreateServiceAccount {
    name: String,
    #[serde(default)]
    username: Option<String>,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    category_id: Option<i64>,
    #[serde(default)]
    ip_ranges: Vec<String>,
    #[serde(default)]
    grants: Vec<CreateServiceAccountGrant>,
}

#[derive(Debug, Deserialize)]
struct UpdateServiceAccount {
    /// Mandatory key: `null` clears the category, a number assigns one. A
    /// request without the key is rejected rather than treated as "no change",
    /// hence the double option (missing vs. explicit `null`).
    #[serde(default, deserialize_with = "deserialize_present_option")]
    category_id: Option<Option<i64>>,
}

/// Distinguishes a missing key (`None`) from an explicit `null`
/// (`Some(None)`), which plain `Option` collapses.
fn deserialize_present_option<'de, D>(deserializer: D) -> Result<Option<Option<i64>>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    Option::<i64>::deserialize(deserializer).map(Some)
}

#[derive(Debug, Deserialize)]
struct CreateServiceAccountCategory {
    name: String,
}

#[derive(Debug, Deserialize)]
struct CreateServiceAccountIpRange {
    cidr: String,
}

#[derive(Debug, Deserialize)]
struct CreateServiceAccountGrant {
    #[serde(default)]
    namespace: Option<String>,
    #[serde(default)]
    repository: Option<String>,
    #[serde(default)]
    can_push: bool,
}

#[derive(Debug, Serialize)]
struct ServiceAccountGrantView {
    id: i64,
    namespace: Option<String>,
    repository: Option<String>,
    can_pull: bool,
    can_push: bool,
}

#[derive(Debug, Serialize)]
struct ServiceAccountIpRangeView {
    id: i64,
    cidr: String,
}

#[derive(Debug, Serialize)]
struct ServiceAccountView {
    id: i64,
    name: String,
    username: String,
    description: Option<String>,
    category_id: Option<i64>,
    token_prefix: String,
    token_suffix: String,
    created_at: DateTime<Utc>,
    last_used_at: Option<DateTime<Utc>>,
    grants: Vec<ServiceAccountGrantView>,
    ip_ranges: Vec<ServiceAccountIpRangeView>,
}

#[derive(Debug, Serialize)]
struct CreatedServiceAccount {
    account: ServiceAccountView,
    token: String,
}

#[derive(Debug, Serialize)]
struct ServiceAccountCategoryView {
    id: i64,
    name: String,
    account_count: i64,
}

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/service-accounts", get(list).post(create))
        .route(
            "/api/service-accounts/{id}",
            get(detail).patch(update).delete(remove),
        )
        .route("/api/service-accounts/{id}/token", post(rotate))
        .route("/api/service-accounts/{id}/grants", post(add_grant))
        .route(
            "/api/service-accounts/{id}/grants/{grant_id}",
            axum::routing::delete(remove_grant),
        )
        .route("/api/service-accounts/{id}/ip-ranges", post(add_ip_range))
        .route(
            "/api/service-accounts/{id}/ip-ranges/{range_id}",
            axum::routing::delete(remove_ip_range),
        )
        .route(
            "/api/service-account-categories",
            get(list_categories).post(create_category),
        )
        .route(
            "/api/service-account-categories/{id}",
            axum::routing::delete(remove_category),
        )
}

fn actor_user_id(ctx: &crate::state::AuthContext) -> ApiResult<i64> {
    ctx.user_id
        .ok_or_else(|| ApiError::unauthorized("user session required"))
}

async fn account_for_owner(state: &AppState, user_id: i64, id: i64) -> ApiResult<ServiceAccount> {
    let account =
        sqlx::query_as::<_, ServiceAccount>("SELECT * FROM service_accounts WHERE id = ? LIMIT 1")
            .bind(id)
            .fetch_optional(&state.db)
            .await?
            .ok_or_else(|| ApiError::not_found("service account not found"))?;
    if account.owner_user_id != user_id {
        return Err(ApiError::not_found("service account not found"));
    }
    Ok(account)
}

#[derive(sqlx::FromRow)]
struct GrantRow {
    id: i64,
    namespace_name: Option<String>,
    repository_name: Option<String>,
    can_pull: bool,
    can_push: bool,
}

async fn account_grants(
    state: &AppState,
    account_id: i64,
) -> ApiResult<Vec<ServiceAccountGrantView>> {
    let rows = sqlx::query_as::<_, GrantRow>(
        "SELECT g.id AS id, n.name AS namespace_name, r.name AS repository_name, \
                g.can_pull AS can_pull, g.can_push AS can_push \
         FROM service_account_grants g \
         LEFT JOIN namespaces n ON n.id = g.namespace_id \
         LEFT JOIN repositories r ON r.id = g.repository_id \
         WHERE g.service_account_id = ? ORDER BY g.id",
    )
    .bind(account_id)
    .fetch_all(&state.db)
    .await?;
    Ok(rows
        .into_iter()
        .map(|row| ServiceAccountGrantView {
            id: row.id,
            namespace: row.namespace_name,
            repository: row.repository_name,
            can_pull: row.can_pull,
            can_push: row.can_push,
        })
        .collect())
}

async fn account_ip_ranges(
    state: &AppState,
    account_id: i64,
) -> ApiResult<Vec<ServiceAccountIpRangeView>> {
    let rows = sqlx::query_as::<_, ServiceAccountIpRange>(
        "SELECT * FROM service_account_ip_ranges WHERE service_account_id = ? ORDER BY id",
    )
    .bind(account_id)
    .fetch_all(&state.db)
    .await?;
    Ok(rows
        .into_iter()
        .map(|row| ServiceAccountIpRangeView {
            id: row.id,
            cidr: row.cidr,
        })
        .collect())
}

async fn account_view(state: &AppState, account: &ServiceAccount) -> ApiResult<ServiceAccountView> {
    Ok(ServiceAccountView {
        id: account.id,
        name: account.name.clone(),
        username: account.username.clone(),
        description: account.description.clone(),
        category_id: account.category_id,
        token_prefix: account.token_prefix.clone(),
        token_suffix: account.token_suffix.clone(),
        created_at: account.created_at,
        last_used_at: account.last_used_at,
        grants: account_grants(state, account.id).await?,
        ip_ranges: account_ip_ranges(state, account.id).await?,
    })
}

const CATEGORY_NAME_MAX_LEN: usize = 40;

async fn category_for_owner(
    state: &AppState,
    user_id: i64,
    id: i64,
) -> ApiResult<ServiceAccountCategory> {
    let category = sqlx::query_as::<_, ServiceAccountCategory>(
        "SELECT * FROM service_account_categories WHERE id = ? LIMIT 1",
    )
    .bind(id)
    .fetch_optional(&state.db)
    .await?
    .ok_or_else(|| ApiError::not_found("category not found"))?;
    if category.owner_user_id != user_id {
        return Err(ApiError::not_found("category not found"));
    }
    Ok(category)
}

#[derive(sqlx::FromRow)]
struct CategoryRow {
    id: i64,
    name: String,
    account_count: i64,
}

async fn category_views(
    state: &AppState,
    user_id: i64,
) -> ApiResult<Vec<ServiceAccountCategoryView>> {
    let rows = sqlx::query_as::<_, CategoryRow>(
        "SELECT c.id AS id, c.name AS name, COUNT(a.id) AS account_count \
         FROM service_account_categories c \
         LEFT JOIN service_accounts a ON a.category_id = c.id AND a.owner_user_id = c.owner_user_id \
         WHERE c.owner_user_id = ? \
         GROUP BY c.id ORDER BY c.name COLLATE NOCASE",
    )
    .bind(user_id)
    .fetch_all(&state.db)
    .await?;
    Ok(rows
        .into_iter()
        .map(|row| ServiceAccountCategoryView {
            id: row.id,
            name: row.name,
            account_count: row.account_count,
        })
        .collect())
}

async fn list(
    State(state): State<AppState>,
    Authenticated(ctx): Authenticated,
) -> ApiResult<Response> {
    let user_id = actor_user_id(&ctx)?;
    let accounts = sqlx::query_as::<_, ServiceAccount>(
        "SELECT * FROM service_accounts WHERE owner_user_id = ? ORDER BY created_at",
    )
    .bind(user_id)
    .fetch_all(&state.db)
    .await?;
    let mut views = Vec::with_capacity(accounts.len());
    for account in &accounts {
        views.push(account_view(&state, account).await?);
    }
    Ok(Json(views).into_response())
}

fn slugify(value: &str) -> String {
    let mut slug = String::new();
    for character in value.chars() {
        if character.is_ascii_alphanumeric() {
            slug.push(character.to_ascii_lowercase());
        } else if matches!(character, '-' | '_' | '.') {
            slug.push(character);
        } else if character.is_whitespace() && !slug.ends_with('-') {
            slug.push('-');
        }
    }
    slug.trim_matches(['-', '.', '_']).to_string()
}

async fn create(
    State(state): State<AppState>,
    Authenticated(ctx): Authenticated,
    Json(request): Json<CreateServiceAccount>,
) -> ApiResult<Response> {
    let user_id = actor_user_id(&ctx)?;
    let normalized_ranges =
        ip_ranges::normalize_list(&request.ip_ranges).map_err(ApiError::bad_request)?;
    let name = request.name.trim().to_string();
    if name.is_empty() {
        return Err(ApiError::bad_request("name is required"));
    }

    let owner =
        sqlx::query_as::<_, crate::models::User>("SELECT * FROM users WHERE id = ? LIMIT 1")
            .bind(user_id)
            .fetch_one(&state.db)
            .await?;

    let username = match request.username {
        Some(raw) if !raw.trim().is_empty() => raw.trim().to_string(),
        _ => {
            let slug = slugify(&name);
            if slug.is_empty() {
                format!("{}-sa", owner.username.to_lowercase())
            } else {
                format!("{}-{}", owner.username.to_lowercase(), slug)
            }
        }
    };

    if registry_accounts::find_by_username(&state.db, &username)
        .await?
        .is_some()
    {
        return Err(ApiError::conflict("service account username is taken"));
    }
    let name_taken: Option<i64> = sqlx::query_scalar(
        "SELECT 1 FROM service_accounts WHERE owner_user_id = ? AND name = ? LIMIT 1",
    )
    .bind(user_id)
    .bind(&name)
    .fetch_optional(&state.db)
    .await?;
    if name_taken.is_some() {
        return Err(ApiError::conflict("service account name is taken"));
    }

    if let Some(category_id) = request.category_id {
        category_for_owner(&state, user_id, category_id).await?;
    }

    let (account, token) = registry_accounts::create(
        &state,
        registry_accounts::NewServiceAccount {
            owner_user_id: user_id,
            name: &name,
            username: &username,
            description: request.description.as_deref(),
        },
    )
    .await?;

    // Everything below must be atomic: a partially configured account would be
    // reachable with a token the caller already holds, so any failure deletes
    // the row (cascading grants and IP ranges) and reports the original error.
    if let Err(err) = sqlx::query("UPDATE service_accounts SET category_id = ? WHERE id = ?")
        .bind(request.category_id)
        .bind(account.id)
        .execute(&state.db)
        .await
    {
        rollback_account(&state, account.id).await;
        return Err(err.into());
    }

    if let Err(err) = ip_ranges::replace_all(&state.db, account.id, &normalized_ranges).await {
        rollback_account(&state, account.id).await;
        return Err(err);
    }

    for grant in &request.grants {
        if let Err(err) = insert_grant(
            &state,
            &ctx,
            &account,
            grant.namespace.as_deref(),
            grant.repository.as_deref(),
            grant.can_push,
        )
        .await
        {
            rollback_account(&state, account.id).await;
            return Err(err);
        }
    }

    super::record_activity(
        &state.db,
        Some(user_id),
        None,
        None,
        "service_account.created",
        &format!("created service account {name}"),
        Some(json!({ "service_account": name, "username": username })),
        false,
    )
    .await;

    let account =
        sqlx::query_as::<_, ServiceAccount>("SELECT * FROM service_accounts WHERE id = ? LIMIT 1")
            .bind(account.id)
            .fetch_one(&state.db)
            .await?;
    let view = account_view(&state, &account).await?;
    Ok((
        StatusCode::CREATED,
        Json(CreatedServiceAccount {
            account: view,
            token,
        }),
    )
        .into_response())
}

async fn detail(
    State(state): State<AppState>,
    Authenticated(ctx): Authenticated,
    Path(id): Path<i64>,
) -> ApiResult<Response> {
    let user_id = actor_user_id(&ctx)?;
    let account = account_for_owner(&state, user_id, id).await?;
    let view = account_view(&state, &account).await?;
    Ok(Json(view).into_response())
}

async fn update(
    State(state): State<AppState>,
    Authenticated(ctx): Authenticated,
    Path(id): Path<i64>,
    Json(request): Json<UpdateServiceAccount>,
) -> ApiResult<Response> {
    let user_id = actor_user_id(&ctx)?;
    let account = account_for_owner(&state, user_id, id).await?;
    let category_id = request
        .category_id
        .ok_or_else(|| ApiError::bad_request("category_id is required"))?;
    if let Some(category_id) = category_id {
        category_for_owner(&state, user_id, category_id).await?;
    }
    sqlx::query("UPDATE service_accounts SET category_id = ? WHERE id = ?")
        .bind(category_id)
        .bind(account.id)
        .execute(&state.db)
        .await?;
    let account =
        sqlx::query_as::<_, ServiceAccount>("SELECT * FROM service_accounts WHERE id = ? LIMIT 1")
            .bind(id)
            .fetch_one(&state.db)
            .await?;
    let view = account_view(&state, &account).await?;
    Ok(Json(view).into_response())
}

async fn list_categories(
    State(state): State<AppState>,
    Authenticated(ctx): Authenticated,
) -> ApiResult<Response> {
    let user_id = actor_user_id(&ctx)?;
    let views = category_views(&state, user_id).await?;
    Ok(Json(views).into_response())
}

async fn create_category(
    State(state): State<AppState>,
    Authenticated(ctx): Authenticated,
    Json(request): Json<CreateServiceAccountCategory>,
) -> ApiResult<Response> {
    let user_id = actor_user_id(&ctx)?;
    let name = request.name.trim().to_string();
    if name.is_empty() {
        return Err(ApiError::bad_request("name is required"));
    }
    if name.chars().count() > CATEGORY_NAME_MAX_LEN {
        return Err(ApiError::bad_request(format!(
            "name must be at most {CATEGORY_NAME_MAX_LEN} characters"
        )));
    }
    let existing: Option<i64> = sqlx::query_scalar(
        "SELECT 1 FROM service_account_categories \
         WHERE owner_user_id = ? AND name = ? COLLATE NOCASE LIMIT 1",
    )
    .bind(user_id)
    .bind(&name)
    .fetch_optional(&state.db)
    .await?;
    if existing.is_some() {
        return Err(ApiError::conflict("category name is taken"));
    }
    let id = sqlx::query(
        "INSERT INTO service_account_categories (owner_user_id, name, created_at) VALUES (?, ?, ?)",
    )
    .bind(user_id)
    .bind(&name)
    .bind(Utc::now())
    .execute(&state.db)
    .await?
    .last_insert_rowid();
    let view = ServiceAccountCategoryView {
        id,
        name,
        account_count: 0,
    };
    Ok((StatusCode::CREATED, Json(view)).into_response())
}

async fn remove_category(
    State(state): State<AppState>,
    Authenticated(ctx): Authenticated,
    Path(id): Path<i64>,
) -> ApiResult<Response> {
    let user_id = actor_user_id(&ctx)?;
    let category = category_for_owner(&state, user_id, id).await?;
    let in_use: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM service_accounts WHERE category_id = ?")
            .bind(category.id)
            .fetch_one(&state.db)
            .await?;
    if in_use > 0 {
        return Err(ApiError::conflict("category is not empty"));
    }
    sqlx::query("DELETE FROM service_account_categories WHERE id = ? AND owner_user_id = ?")
        .bind(category.id)
        .bind(user_id)
        .execute(&state.db)
        .await?;
    Ok(StatusCode::NO_CONTENT.into_response())
}

async fn remove(
    State(state): State<AppState>,
    Authenticated(ctx): Authenticated,
    Path(id): Path<i64>,
) -> ApiResult<Response> {
    let user_id = actor_user_id(&ctx)?;
    let _account = account_for_owner(&state, user_id, id).await?;
    sqlx::query("DELETE FROM service_accounts WHERE id = ?")
        .bind(id)
        .execute(&state.db)
        .await?;
    crate::auth::registry_refresh::revoke_for_service_account(&state, id).await?;
    Ok(StatusCode::NO_CONTENT.into_response())
}

async fn rotate(
    State(state): State<AppState>,
    Authenticated(ctx): Authenticated,
    Path(id): Path<i64>,
) -> ApiResult<Response> {
    let user_id = actor_user_id(&ctx)?;
    let account = account_for_owner(&state, user_id, id).await?;

    let (plaintext, prefix, suffix, hash) = tokens::generate_service_token();
    sqlx::query(
        "UPDATE service_accounts SET token_prefix = ?, token_suffix = ?, token_hash = ? WHERE id = ?",
    )
    .bind(&prefix)
    .bind(&suffix)
    .bind(&hash)
    .bind(account.id)
    .execute(&state.db)
    .await?;

    crate::auth::registry_refresh::revoke_for_service_account(&state, id).await?;

    let account =
        sqlx::query_as::<_, ServiceAccount>("SELECT * FROM service_accounts WHERE id = ? LIMIT 1")
            .bind(id)
            .fetch_one(&state.db)
            .await?;
    let view = account_view(&state, &account).await?;
    Ok(Json(CreatedServiceAccount {
        account: view,
        token: plaintext,
    })
    .into_response())
}

/// Deletes a freshly created account so a failed multi-step creation leaves
/// nothing behind; grants and IP ranges cascade.
async fn rollback_account(state: &AppState, account_id: i64) {
    let _ = sqlx::query("DELETE FROM service_accounts WHERE id = ?")
        .bind(account_id)
        .execute(&state.db)
        .await;
}

/// Validates and inserts one grant. Shared by `add_grant` and account creation
/// so the two paths can never diverge.
async fn insert_grant(
    state: &AppState,
    ctx: &AuthContext,
    account: &ServiceAccount,
    namespace: Option<&str>,
    repository: Option<&str>,
    can_push: bool,
) -> ApiResult<ServiceAccountGrantView> {
    let namespace_name = namespace.map(str::trim).filter(|value| !value.is_empty());
    let repository_name = repository.map(str::trim).filter(|value| !value.is_empty());

    let (namespace_id, repository_id) = match (namespace_name, repository_name) {
        (Some(_), Some(_)) => {
            return Err(ApiError::bad_request(
                "specify either a namespace or a repository, not both",
            ));
        }
        (Some(namespace_name), None) => {
            let namespace = load_namespace(&state.db, namespace_name)
                .await?
                .ok_or_else(|| ApiError::not_found("namespace not found"))?;
            if !is_namespace_admin(state, ctx, &namespace).await? {
                return Err(ApiError::forbidden("namespace owner required"));
            }
            (Some(namespace.id), None)
        }
        (None, Some(repository_name)) => {
            let repository = state
                .registry
                .find_repository(repository_name)
                .await
                .map_err(ApiError::from)?
                .ok_or_else(|| ApiError::not_found("repository not found"))?;
            let namespace = namespace_by_id(&state.db, repository.namespace_id)
                .await?
                .ok_or_else(|| ApiError::not_found("repository not found"))?;
            if !is_namespace_admin(state, ctx, &namespace).await? {
                return Err(ApiError::forbidden("namespace owner required"));
            }
            (None, Some(repository.id))
        }
        (None, None) => {
            return Err(ApiError::bad_request(
                "a namespace or repository is required",
            ));
        }
    };

    let can_pull = true;
    let duplicate: Option<i64> = sqlx::query_scalar(
        "SELECT 1 FROM service_account_grants \
         WHERE service_account_id = ? \
           AND COALESCE(namespace_id, 0) = COALESCE(?, 0) \
           AND COALESCE(repository_id, 0) = COALESCE(?, 0) LIMIT 1",
    )
    .bind(account.id)
    .bind(namespace_id)
    .bind(repository_id)
    .fetch_optional(&state.db)
    .await?;
    if duplicate.is_some() {
        return Err(ApiError::conflict("grant already exists"));
    }

    let id = sqlx::query(
        "INSERT INTO service_account_grants \
         (service_account_id, namespace_id, repository_id, can_pull, can_push) \
         VALUES (?, ?, ?, ?, ?)",
    )
    .bind(account.id)
    .bind(namespace_id)
    .bind(repository_id)
    .bind(can_pull)
    .bind(can_push)
    .execute(&state.db)
    .await?
    .last_insert_rowid();

    Ok(ServiceAccountGrantView {
        id,
        namespace: namespace_name.map(str::to_string),
        repository: repository_name.map(str::to_string),
        can_pull,
        can_push,
    })
}

async fn add_grant(
    State(state): State<AppState>,
    Authenticated(ctx): Authenticated,
    Path(id): Path<i64>,
    Json(request): Json<CreateServiceAccountGrant>,
) -> ApiResult<Response> {
    let user_id = actor_user_id(&ctx)?;
    let account = account_for_owner(&state, user_id, id).await?;
    let view = insert_grant(
        &state,
        &ctx,
        &account,
        request.namespace.as_deref(),
        request.repository.as_deref(),
        request.can_push,
    )
    .await?;
    Ok((StatusCode::CREATED, Json(view)).into_response())
}

async fn remove_grant(
    State(state): State<AppState>,
    Authenticated(ctx): Authenticated,
    Path((id, grant_id)): Path<(i64, i64)>,
) -> ApiResult<Response> {
    let user_id = actor_user_id(&ctx)?;
    let account = account_for_owner(&state, user_id, id).await?;
    let result =
        sqlx::query("DELETE FROM service_account_grants WHERE id = ? AND service_account_id = ?")
            .bind(grant_id)
            .bind(account.id)
            .execute(&state.db)
            .await?;
    if result.rows_affected() == 0 {
        return Err(ApiError::not_found("grant not found"));
    }
    Ok(StatusCode::NO_CONTENT.into_response())
}

async fn add_ip_range(
    State(state): State<AppState>,
    Authenticated(ctx): Authenticated,
    Path(id): Path<i64>,
    Json(request): Json<CreateServiceAccountIpRange>,
) -> ApiResult<Response> {
    let user_id = actor_user_id(&ctx)?;
    let account = account_for_owner(&state, user_id, id).await?;
    let range = ip_ranges::add(&state.db, account.id, &request.cidr).await?;
    let view = ServiceAccountIpRangeView {
        id: range.id,
        cidr: range.cidr,
    };
    Ok((StatusCode::CREATED, Json(view)).into_response())
}

async fn remove_ip_range(
    State(state): State<AppState>,
    Authenticated(ctx): Authenticated,
    Path((id, range_id)): Path<(i64, i64)>,
) -> ApiResult<Response> {
    let user_id = actor_user_id(&ctx)?;
    let account = account_for_owner(&state, user_id, id).await?;
    if !ip_ranges::remove(&state.db, account.id, range_id).await? {
        return Err(ApiError::not_found("IP range not found"));
    }
    Ok(StatusCode::NO_CONTENT.into_response())
}
