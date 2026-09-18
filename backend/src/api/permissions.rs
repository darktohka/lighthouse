//! Permission delegation CRUD for namespaces and repositories.
//!
//! User autocomplete lives at `GET /api/users/search` and is owned by
//! [`super::users`]; the grants themselves are restricted to the namespace
//! owner or a namespace `admin` member.

use axum::Json;
use axum::Router;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use chrono::Utc;
use serde::Deserialize;
use serde_json::json;

use crate::auth::middleware::Authenticated;
use crate::error::{ApiError, ApiResult};
use crate::models::{NamespacePermission, RepositoryPermission, User};
use crate::state::{AppState, AuthContext};

use super::{
    PublicPermission, UserSummary, is_namespace_admin, load_namespace, namespace_by_id,
    user_summaries, visible_repository,
};

/// Body shared by namespace- and repository-scoped grant creation.
#[derive(Debug, Deserialize)]
pub struct CreateGrant {
    pub subject_type: String,
    #[serde(default)]
    pub subject: Option<String>,
    #[serde(default)]
    pub can_push: bool,
}

pub fn router() -> Router<AppState> {
    Router::new()
        .route(
            "/api/namespaces/{name}/permissions",
            get(list_namespace_permissions).post(add_namespace_permission),
        )
        .route(
            "/api/namespaces/{name}/permissions/{id}",
            axum::routing::delete(revoke_namespace_permission),
        )
}

/// Validates the grant subject and resolves it to a user id.
async fn resolve_subject(
    state: &AppState,
    grant: &CreateGrant,
) -> ApiResult<(String, Option<i64>)> {
    match grant.subject_type.as_str() {
        "anonymous" => Ok(("anonymous".to_string(), None)),
        "user" => {
            let username = grant
                .subject
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .ok_or_else(|| ApiError::bad_request("subject is required for a user grant"))?;
            let user = sqlx::query_as::<_, User>(
                "SELECT * FROM users WHERE username = ? COLLATE NOCASE LIMIT 1",
            )
            .bind(username)
            .fetch_optional(&state.db)
            .await?
            .ok_or_else(|| ApiError::not_found("user not found"))?;
            Ok(("user".to_string(), Some(user.id)))
        }
        _ => Err(ApiError::bad_request(
            "subject_type must be `user` or `anonymous`",
        )),
    }
}

/// A permission row flattened into the tuple [`permission_views`] consumes:
/// `(id, subject_type, subject_user_id, can_pull, can_push, created_at)`.
type PermissionRow = (i64, String, Option<i64>, bool, bool, chrono::DateTime<Utc>);

async fn permission_views(
    state: &AppState,
    rows: Vec<PermissionRow>,
) -> ApiResult<Vec<PublicPermission>> {
    let ids: Vec<i64> = rows.iter().filter_map(|row| row.2).collect();
    let summaries = user_summaries(&state.db, &ids).await?;
    Ok(rows
        .into_iter()
        .map(
            |(id, subject_type, subject_user_id, can_pull, can_push, created_at)| {
                let subject = subject_user_id.and_then(|user_id| summaries.get(&user_id).cloned());
                PublicPermission {
                    id,
                    subject_type,
                    subject,
                    can_pull,
                    can_push,
                    created_at,
                }
            },
        )
        .collect())
}

async fn namespace_for_admin(
    state: &AppState,
    actor: &AuthContext,
    name: &str,
) -> ApiResult<crate::models::Namespace> {
    let Some(namespace) = load_namespace(&state.db, name).await? else {
        return Err(ApiError::not_found("namespace not found"));
    };
    if !super::namespace_visible(state, actor, &namespace).await? {
        return Err(ApiError::not_found("namespace not found"));
    }
    if !is_namespace_admin(state, actor, &namespace).await? {
        return Err(ApiError::forbidden("namespace owner required"));
    }
    Ok(namespace)
}

async fn list_namespace_permissions(
    State(state): State<AppState>,
    Authenticated(ctx): Authenticated,
    Path(name): Path<String>,
) -> ApiResult<Response> {
    let namespace = namespace_for_admin(&state, &ctx, &name).await?;
    let rows = sqlx::query_as::<_, NamespacePermission>(
        "SELECT * FROM namespace_permissions WHERE namespace_id = ? ORDER BY created_at",
    )
    .bind(namespace.id)
    .fetch_all(&state.db)
    .await?;
    let tuples = rows
        .into_iter()
        .map(|row| {
            (
                row.id,
                row.subject_type,
                row.subject_user_id,
                row.can_pull,
                row.can_push,
                row.created_at,
            )
        })
        .collect();
    let views = permission_views(&state, tuples).await?;
    Ok(Json(views).into_response())
}

async fn add_namespace_permission(
    State(state): State<AppState>,
    Authenticated(ctx): Authenticated,
    Path(name): Path<String>,
    Json(grant): Json<CreateGrant>,
) -> ApiResult<Response> {
    let namespace = namespace_for_admin(&state, &ctx, &name).await?;
    let (subject_type, subject_user_id) = resolve_subject(&state, &grant).await?;

    let exists: Option<i64> = sqlx::query_scalar(
        "SELECT 1 FROM namespace_permissions \
         WHERE namespace_id = ? AND subject_type = ? \
           AND COALESCE(subject_user_id, 0) = COALESCE(?, 0) LIMIT 1",
    )
    .bind(namespace.id)
    .bind(&subject_type)
    .bind(subject_user_id)
    .fetch_optional(&state.db)
    .await?;
    if exists.is_some() {
        return Err(ApiError::conflict("grant already exists"));
    }

    let can_push = grant.can_push;
    let can_pull = true;
    let id = sqlx::query(
        "INSERT INTO namespace_permissions \
         (namespace_id, subject_type, subject_user_id, can_pull, can_push, created_by, created_at) \
         VALUES (?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(namespace.id)
    .bind(&subject_type)
    .bind(subject_user_id)
    .bind(can_pull)
    .bind(can_push)
    .bind(ctx.user_id)
    .bind(Utc::now())
    .execute(&state.db)
    .await?
    .last_insert_rowid();

    super::record_activity(
        &state.db,
        ctx.user_id,
        Some(namespace.id),
        None,
        "permission.granted",
        &format!("granted access on namespace {}", namespace.name),
        Some(json!({ "namespace": namespace.name, "subject_type": subject_type, "can_push": can_push })),
        namespace.is_public,
    )
    .await;

    let subject = match subject_user_id {
        Some(user_id) => super::user_summary(&state.db, user_id).await?,
        None => None,
    };
    let view = PublicPermission {
        id,
        subject_type,
        subject,
        can_pull,
        can_push,
        created_at: Utc::now(),
    };
    Ok((StatusCode::CREATED, Json(view)).into_response())
}

async fn revoke_namespace_permission(
    State(state): State<AppState>,
    Authenticated(ctx): Authenticated,
    Path((name, id)): Path<(String, i64)>,
) -> ApiResult<Response> {
    let namespace = namespace_for_admin(&state, &ctx, &name).await?;
    let result = sqlx::query("DELETE FROM namespace_permissions WHERE id = ? AND namespace_id = ?")
        .bind(id)
        .bind(namespace.id)
        .execute(&state.db)
        .await?;
    if result.rows_affected() == 0 {
        return Err(ApiError::not_found("grant not found"));
    }

    super::record_activity(
        &state.db,
        ctx.user_id,
        Some(namespace.id),
        None,
        "permission.revoked",
        &format!("revoked access on namespace {}", namespace.name),
        Some(json!({ "namespace": namespace.name, "grant_id": id })),
        namespace.is_public,
    )
    .await;
    Ok(StatusCode::NO_CONTENT.into_response())
}

// ---------------------------------------------------------------------------
// Repository-scoped grants (invoked from the repository dispatcher)
// ---------------------------------------------------------------------------

/// Lists a repository's grants. Callable by an owner/admin only.
pub async fn list_repository_permissions(
    state: AppState,
    actor: AuthContext,
    name: String,
) -> ApiResult<Response> {
    let repository = repository_for_admin(&state, &actor, &name).await?;
    let views = repository_grants(&state, repository.id).await?;
    Ok(Json(views).into_response())
}

/// Adds a repository-scoped grant.
pub async fn add_repository_permission(
    state: AppState,
    actor: AuthContext,
    name: String,
    grant: CreateGrant,
) -> ApiResult<Response> {
    let repository = repository_for_admin(&state, &actor, &name).await?;
    let (subject_type, subject_user_id) = resolve_subject(&state, &grant).await?;

    let exists: Option<i64> = sqlx::query_scalar(
        "SELECT 1 FROM repository_permissions \
         WHERE repository_id = ? AND subject_type = ? \
           AND COALESCE(subject_user_id, 0) = COALESCE(?, 0) LIMIT 1",
    )
    .bind(repository.id)
    .bind(&subject_type)
    .bind(subject_user_id)
    .fetch_optional(&state.db)
    .await?;
    if exists.is_some() {
        return Err(ApiError::conflict("grant already exists"));
    }

    let can_push = grant.can_push;
    let can_pull = true;
    let id = sqlx::query(
        "INSERT INTO repository_permissions \
         (repository_id, subject_type, subject_user_id, can_pull, can_push, created_by, created_at) \
         VALUES (?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(repository.id)
    .bind(&subject_type)
    .bind(subject_user_id)
    .bind(can_pull)
    .bind(can_push)
    .bind(actor.user_id)
    .bind(Utc::now())
    .execute(&state.db)
    .await?
    .last_insert_rowid();

    super::record_activity(
        &state.db,
        actor.user_id,
        Some(repository.namespace_id),
        Some(repository.id),
        "permission.granted",
        &format!("granted access on {}", repository.name),
        Some(json!({ "repository": repository.name, "subject_type": subject_type, "can_push": can_push })),
        repository.is_public,
    )
    .await;

    let subject = match subject_user_id {
        Some(user_id) => super::user_summary(&state.db, user_id).await?,
        None => None,
    };
    let view = PublicPermission {
        id,
        subject_type,
        subject,
        can_pull,
        can_push,
        created_at: Utc::now(),
    };
    Ok((StatusCode::CREATED, Json(view)).into_response())
}

/// Revokes a repository-scoped grant.
pub async fn revoke_repository_permission(
    state: AppState,
    actor: AuthContext,
    name: String,
    id: i64,
) -> ApiResult<Response> {
    let repository = repository_for_admin(&state, &actor, &name).await?;
    let result =
        sqlx::query("DELETE FROM repository_permissions WHERE id = ? AND repository_id = ?")
            .bind(id)
            .bind(repository.id)
            .execute(&state.db)
            .await?;
    if result.rows_affected() == 0 {
        return Err(ApiError::not_found("grant not found"));
    }

    super::record_activity(
        &state.db,
        actor.user_id,
        Some(repository.namespace_id),
        Some(repository.id),
        "permission.revoked",
        &format!("revoked access on {}", repository.name),
        Some(json!({ "repository": repository.name, "grant_id": id })),
        repository.is_public,
    )
    .await;
    Ok(StatusCode::NO_CONTENT.into_response())
}

async fn repository_for_admin(
    state: &AppState,
    actor: &AuthContext,
    name: &str,
) -> ApiResult<crate::models::Repository> {
    let repository = visible_repository(state, actor, name).await?;
    let namespace = namespace_by_id(&state.db, repository.namespace_id)
        .await?
        .ok_or_else(|| ApiError::not_found("repository not found"))?;
    if !is_namespace_admin(state, actor, &namespace).await? {
        return Err(ApiError::forbidden("namespace owner required"));
    }
    Ok(repository)
}

/// The public view of a repository's grants.
pub async fn repository_grants(
    state: &AppState,
    repository_id: i64,
) -> ApiResult<Vec<PublicPermission>> {
    let rows = sqlx::query_as::<_, RepositoryPermission>(
        "SELECT * FROM repository_permissions WHERE repository_id = ? ORDER BY created_at",
    )
    .bind(repository_id)
    .fetch_all(&state.db)
    .await?;
    let tuples = rows
        .into_iter()
        .map(|row| {
            (
                row.id,
                row.subject_type,
                row.subject_user_id,
                row.can_pull,
                row.can_push,
                row.created_at,
            )
        })
        .collect();
    permission_views(state, tuples).await
}

/// Used by other modules to describe a grant subject.
pub(crate) fn subject_summary(subject: &User) -> UserSummary {
    UserSummary::from(subject)
}
