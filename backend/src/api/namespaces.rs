//! Workspaces, membership and visibility.

use axum::Json;
use axum::Router;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use chrono::Utc;
use serde::Deserialize;
use serde_json::json;

use crate::auth::middleware::{Auth, Authenticated};
use crate::error::{ApiError, ApiResult};
use crate::models::{Namespace, Repository, User};
use crate::oci::reference;
use crate::permissions;
use crate::state::{AppState, AuthContext};

use super::{
    NamespaceMemberView, NamespaceView, PageQuery, Pagination, is_namespace_owner, load_namespace,
    namespace_visible, user_summary,
};

#[derive(Debug, Deserialize)]
struct CreateNamespace {
    name: String,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    is_public: Option<bool>,
}

#[derive(Debug, Deserialize)]
struct UpdateNamespace {
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    is_public: Option<bool>,
}

#[derive(Debug, Deserialize)]
struct CreateRepository {
    name: String,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    is_public: Option<bool>,
}

#[derive(Debug, Deserialize)]
struct AddMember {
    username: String,
    #[serde(default)]
    role: Option<String>,
}

#[derive(Debug, sqlx::FromRow)]
struct MemberRow {
    user_id: i64,
    role: String,
    created_at: chrono::DateTime<Utc>,
}

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/namespaces", get(list).post(create))
        .route(
            "/api/namespaces/{name}",
            get(detail).patch(update).delete(remove),
        )
        .route(
            "/api/namespaces/{name}/repositories",
            get(repositories_in_namespace).post(create_repository),
        )
        .route(
            "/api/namespaces/{name}/members",
            get(list_members).post(add_member),
        )
        .route(
            "/api/namespaces/{name}/members/{username}",
            axum::routing::delete(remove_member),
        )
}

fn normalize(value: Option<String>) -> Option<String> {
    value.and_then(|value| {
        let trimmed = value.trim().to_string();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed)
        }
    })
}

pub(crate) async fn namespace_view(
    state: &AppState,
    actor: &AuthContext,
    namespace: &Namespace,
) -> ApiResult<NamespaceView> {
    let access = permissions::namespace_access(state, actor, &namespace.name).await?;
    let repository_count: i64 = if access.can_push {
        sqlx::query_scalar("SELECT COUNT(*) FROM repositories WHERE namespace_id = ?")
            .bind(namespace.id)
            .fetch_one(&state.db)
            .await?
    } else if access.can_pull {
        let rows: Vec<(String, bool)> =
            sqlx::query_as("SELECT name, is_hidden FROM repositories WHERE namespace_id = ?")
                .bind(namespace.id)
                .fetch_all(&state.db)
                .await?;
        let mut count = 0i64;
        for (name, is_hidden) in rows {
            if is_hidden && !actor.is_authenticated() {
                continue;
            }
            if permissions::repository_access(state, actor, &name)
                .await?
                .can_pull
            {
                count += 1;
            }
        }
        count
    } else {
        0
    };

    let owner = match namespace.owner_user_id {
        Some(owner_id) => user_summary(&state.db, owner_id).await?,
        None => None,
    };

    Ok(NamespaceView {
        id: namespace.id,
        name: namespace.name.clone(),
        kind: namespace.kind.clone(),
        owner,
        description: namespace.description.clone(),
        is_public: namespace.is_public,
        repository_count,
        created_at: namespace.created_at,
    })
}

async fn list(
    State(state): State<AppState>,
    auth: Auth,
    Query(query): Query<PageQuery>,
) -> ApiResult<Response> {
    let actor = auth.0;
    let anonymous = !actor.is_authenticated();
    let user_id = actor.user_id;
    let service_account_id = actor.service_account_id;

    let namespaces = sqlx::query_as::<_, Namespace>(
        "SELECT n.* FROM namespaces n \
         WHERE n.owner_user_id = ? \
            OR EXISTS (SELECT 1 FROM namespace_members m \
                       WHERE m.namespace_id = n.id AND m.user_id = ?) \
            OR EXISTS (SELECT 1 FROM namespace_permissions p \
                       WHERE p.namespace_id = n.id \
                         AND ((p.subject_type = 'user' AND p.subject_user_id = ?) \
                              OR (p.subject_type = 'anonymous' AND ?))) \
            OR EXISTS (SELECT 1 FROM service_account_grants g \
                       WHERE g.namespace_id = n.id AND g.service_account_id = ?) \
            OR (n.is_public = 1 AND EXISTS ( \
                    SELECT 1 FROM repositories r \
                    WHERE r.namespace_id = n.id AND r.is_public = 1 \
                      AND (r.is_hidden = 0 OR ?))) \
         ORDER BY n.name COLLATE NOCASE",
    )
    .bind(user_id)
    .bind(user_id)
    .bind(user_id)
    .bind(anonymous)
    .bind(service_account_id)
    .bind(actor.is_authenticated())
    .fetch_all(&state.db)
    .await?;

    let pagination = Pagination::from_query(&query);
    let mut views = Vec::with_capacity(namespaces.len());
    for namespace in &namespaces {
        views.push(namespace_view(&state, &actor, namespace).await?);
    }
    let total = views.len() as i64;
    let items = pagination.window(views);
    Ok(Json(pagination.envelope(items, total)).into_response())
}

async fn create(
    State(state): State<AppState>,
    Authenticated(ctx): Authenticated,
    Json(request): Json<CreateNamespace>,
) -> ApiResult<Response> {
    let user_id = ctx
        .user_id
        .ok_or_else(|| ApiError::unauthorized("user session required"))?;
    let name = request.name.trim().to_lowercase();
    if name.is_empty()
        || name.contains('/')
        || !reference::validate_repository_name(&name)
        || reference::is_reserved_namespace(&name)
    {
        return Err(ApiError::bad_request("invalid namespace name"));
    }

    let namespace_taken: Option<i64> =
        sqlx::query_scalar("SELECT 1 FROM namespaces WHERE name = ? COLLATE NOCASE LIMIT 1")
            .bind(&name)
            .fetch_optional(&state.db)
            .await?;
    let username_taken: Option<i64> =
        sqlx::query_scalar("SELECT 1 FROM users WHERE username = ? COLLATE NOCASE LIMIT 1")
            .bind(&name)
            .fetch_optional(&state.db)
            .await?;
    if namespace_taken.is_some() || username_taken.is_some() {
        return Err(ApiError::conflict("namespace_taken"));
    }

    let now = Utc::now();
    let namespace_id = sqlx::query(
        "INSERT INTO namespaces (name, kind, owner_user_id, description, is_public, created_at, updated_at) \
         VALUES (?, 'workspace', ?, ?, ?, ?, ?)",
    )
    .bind(&name)
    .bind(user_id)
    .bind(normalize(request.description))
    .bind(request.is_public.unwrap_or(true))
    .bind(now)
    .bind(now)
    .execute(&state.db)
    .await?
    .last_insert_rowid();

    super::record_activity(
        &state.db,
        Some(user_id),
        Some(namespace_id),
        None,
        "namespace.created",
        &format!("created namespace {name}"),
        Some(json!({ "namespace": name })),
        true,
    )
    .await;

    let namespace = sqlx::query_as::<_, Namespace>("SELECT * FROM namespaces WHERE id = ?")
        .bind(namespace_id)
        .fetch_one(&state.db)
        .await?;
    let view = namespace_view(&state, &ctx, &namespace).await?;
    Ok((StatusCode::CREATED, Json(view)).into_response())
}

async fn detail(
    State(state): State<AppState>,
    auth: Auth,
    Path(name): Path<String>,
) -> ApiResult<Response> {
    let Some(namespace) = load_namespace(&state.db, &name).await? else {
        return Err(ApiError::not_found("namespace not found"));
    };
    if !namespace_visible(&state, &auth.0, &namespace).await? {
        return Err(ApiError::not_found("namespace not found"));
    }
    let view = namespace_view(&state, &auth.0, &namespace).await?;
    Ok(Json(view).into_response())
}

async fn update(
    State(state): State<AppState>,
    Authenticated(ctx): Authenticated,
    Path(name): Path<String>,
    Json(request): Json<UpdateNamespace>,
) -> ApiResult<Response> {
    let Some(namespace) = load_namespace(&state.db, &name).await? else {
        return Err(ApiError::not_found("namespace not found"));
    };
    if !namespace_visible(&state, &ctx, &namespace).await? {
        return Err(ApiError::not_found("namespace not found"));
    }
    if !is_namespace_owner(&ctx, &namespace) {
        return Err(ApiError::forbidden("namespace owner required"));
    }

    sqlx::query(
        "UPDATE namespaces SET \
             description = COALESCE(?, description), \
             is_public = COALESCE(?, is_public), \
             updated_at = ? \
         WHERE id = ?",
    )
    .bind(normalize(request.description))
    .bind(request.is_public)
    .bind(Utc::now())
    .bind(namespace.id)
    .execute(&state.db)
    .await?;

    let namespace = sqlx::query_as::<_, Namespace>("SELECT * FROM namespaces WHERE id = ?")
        .bind(namespace.id)
        .fetch_one(&state.db)
        .await?;
    let view = namespace_view(&state, &ctx, &namespace).await?;
    Ok(Json(view).into_response())
}

async fn remove(
    State(state): State<AppState>,
    Authenticated(ctx): Authenticated,
    Path(name): Path<String>,
) -> ApiResult<Response> {
    let Some(namespace) = load_namespace(&state.db, &name).await? else {
        return Err(ApiError::not_found("namespace not found"));
    };
    if !namespace_visible(&state, &ctx, &namespace).await? {
        return Err(ApiError::not_found("namespace not found"));
    }
    if !is_namespace_owner(&ctx, &namespace) {
        return Err(ApiError::forbidden("namespace owner required"));
    }
    if namespace.kind == "user" {
        return Err(ApiError::bad_request(
            "personal namespaces cannot be deleted",
        ));
    }

    sqlx::query("DELETE FROM namespaces WHERE id = ?")
        .bind(namespace.id)
        .execute(&state.db)
        .await?;

    if let Ok(report) = crate::storage::gc::collect(&state.registry, &state.storage, false).await {
        for digest in &report.deleted_digests {
            state.layer_cache.invalidate(digest);
        }
        tracing::info!(blobs = report.blobs_deleted, "namespace delete gc");
    }

    Ok(StatusCode::NO_CONTENT.into_response())
}

async fn list_members(
    State(state): State<AppState>,
    auth: Auth,
    Path(name): Path<String>,
) -> ApiResult<Response> {
    let Some(namespace) = load_namespace(&state.db, &name).await? else {
        return Err(ApiError::not_found("namespace not found"));
    };
    if !namespace_visible(&state, &auth.0, &namespace).await? {
        return Err(ApiError::not_found("namespace not found"));
    }

    let members = sqlx::query_as::<_, MemberRow>(
        "SELECT user_id, role, created_at FROM namespace_members \
         WHERE namespace_id = ? ORDER BY created_at",
    )
    .bind(namespace.id)
    .fetch_all(&state.db)
    .await?;

    let ids: Vec<i64> = members.iter().map(|member| member.user_id).collect();
    let summaries = super::user_summaries(&state.db, &ids).await?;
    let mut items = Vec::with_capacity(members.len());
    for member in members {
        if let Some(user) = summaries.get(&member.user_id) {
            items.push(NamespaceMemberView {
                user: user.clone(),
                role: member.role,
                created_at: member.created_at,
            });
        }
    }
    Ok(Json(items).into_response())
}

async fn add_member(
    State(state): State<AppState>,
    Authenticated(ctx): Authenticated,
    Path(name): Path<String>,
    Json(request): Json<AddMember>,
) -> ApiResult<Response> {
    let Some(namespace) = load_namespace(&state.db, &name).await? else {
        return Err(ApiError::not_found("namespace not found"));
    };
    if !namespace_visible(&state, &ctx, &namespace).await? {
        return Err(ApiError::not_found("namespace not found"));
    }
    if !is_namespace_owner(&ctx, &namespace) {
        return Err(ApiError::forbidden("namespace owner required"));
    }

    let role = request.role.unwrap_or_else(|| "member".to_string());
    if role != "admin" && role != "member" {
        return Err(ApiError::bad_request("role must be `admin` or `member`"));
    }

    let user =
        sqlx::query_as::<_, User>("SELECT * FROM users WHERE username = ? COLLATE NOCASE LIMIT 1")
            .bind(request.username.trim())
            .fetch_optional(&state.db)
            .await?
            .ok_or_else(|| ApiError::not_found("user not found"))?;

    let now = Utc::now();
    sqlx::query(
        "INSERT INTO namespace_members (namespace_id, user_id, role, created_at) \
         VALUES (?, ?, ?, ?) \
         ON CONFLICT(namespace_id, user_id) DO UPDATE SET role = excluded.role",
    )
    .bind(namespace.id)
    .bind(user.id)
    .bind(&role)
    .bind(now)
    .execute(&state.db)
    .await?;

    let view = NamespaceMemberView {
        user: super::UserSummary::from(&user),
        role,
        created_at: now,
    };
    Ok((StatusCode::CREATED, Json(view)).into_response())
}

async fn remove_member(
    State(state): State<AppState>,
    Authenticated(ctx): Authenticated,
    Path((name, username)): Path<(String, String)>,
) -> ApiResult<Response> {
    let Some(namespace) = load_namespace(&state.db, &name).await? else {
        return Err(ApiError::not_found("namespace not found"));
    };
    if !namespace_visible(&state, &ctx, &namespace).await? {
        return Err(ApiError::not_found("namespace not found"));
    }
    if !is_namespace_owner(&ctx, &namespace) {
        return Err(ApiError::forbidden("namespace owner required"));
    }

    let user_id: Option<i64> =
        sqlx::query_scalar("SELECT id FROM users WHERE username = ? COLLATE NOCASE LIMIT 1")
            .bind(username)
            .fetch_optional(&state.db)
            .await?;
    if let Some(user_id) = user_id {
        sqlx::query("DELETE FROM namespace_members WHERE namespace_id = ? AND user_id = ?")
            .bind(namespace.id)
            .bind(user_id)
            .execute(&state.db)
            .await?;
    }
    Ok(StatusCode::NO_CONTENT.into_response())
}

async fn create_repository(
    State(state): State<AppState>,
    Authenticated(ctx): Authenticated,
    Path(name): Path<String>,
    Json(request): Json<CreateRepository>,
) -> ApiResult<Response> {
    let repo_path = request.name.trim().trim_matches('/').to_string();
    if repo_path.is_empty() {
        return Err(ApiError::bad_request("repository name required"));
    }
    let full = format!("{name}/{repo_path}");
    if !reference::validate_repository_name(&full) {
        return Err(ApiError::bad_request("invalid repository name"));
    }

    let access = permissions::repository_access(&state, &ctx, &full).await?;
    if !access.can_pull && !access.can_push {
        return Err(ApiError::not_found("namespace not found"));
    }
    if !access.can_push {
        return Err(ApiError::forbidden("push access required"));
    }

    let namespace = load_namespace(&state.db, &name)
        .await?
        .ok_or_else(|| ApiError::not_found("namespace not found"))?;
    let is_public = request.is_public.unwrap_or(namespace.is_public);

    let now = Utc::now();
    let path = full.split_once('/').map(|(_, path)| path).unwrap_or(&full);
    let inserted = match sqlx::query(
        "INSERT INTO repositories \
         (namespace_id, name, path, description, is_public, created_by, created_at, updated_at) \
         VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(namespace.id)
    .bind(&full)
    .bind(path)
    .bind(normalize(request.description))
    .bind(is_public)
    .bind(ctx.user_id)
    .bind(now)
    .bind(now)
    .execute(&state.db)
    .await
    {
        Ok(inserted) => inserted,
        Err(err) => {
            if matches!(&err, sqlx::Error::Database(db) if db.is_unique_violation()) {
                return Err(ApiError::conflict("repository already exists"));
            }
            return Err(err.into());
        }
    };

    let repository = sqlx::query_as::<_, Repository>("SELECT * FROM repositories WHERE id = ?")
        .bind(inserted.last_insert_rowid())
        .fetch_one(&state.db)
        .await?;

    super::record_activity(
        &state.db,
        ctx.user_id,
        Some(namespace.id),
        Some(repository.id),
        "repository.created",
        &format!("created repository {}", repository.name),
        Some(json!({ "repository": repository.name.clone() })),
        repository.is_public,
    )
    .await;

    let detail = super::repositories::build_detail(&state, &ctx, &repository).await?;
    Ok((StatusCode::CREATED, Json(detail)).into_response())
}

/// `?page=` / `?per_page=` plus the repository sort controls. axum permits only
/// one `Query` extractor per handler, so pagination and ordering share a struct.
#[derive(Debug, Deserialize)]
struct RepositoryListQuery {
    #[serde(default)]
    page: Option<i64>,
    #[serde(default)]
    per_page: Option<i64>,
    #[serde(default)]
    sort: Option<String>,
    #[serde(default)]
    order: Option<String>,
}

async fn repositories_in_namespace(
    State(state): State<AppState>,
    auth: Auth,
    Path(name): Path<String>,
    Query(query): Query<RepositoryListQuery>,
) -> ApiResult<Response> {
    let Some(namespace) = load_namespace(&state.db, &name).await? else {
        return Err(ApiError::not_found("namespace not found"));
    };
    if !namespace_visible(&state, &auth.0, &namespace).await? {
        return Err(ApiError::not_found("namespace not found"));
    }

    let sort = query.sort.as_deref().unwrap_or("updated");
    if sort != "name" && sort != "size" && sort != "updated" {
        return Err(ApiError::bad_request(
            "sort must be `name`, `size` or `updated`",
        ));
    }
    let order = query.order.as_deref().unwrap_or("desc");
    if order != "asc" && order != "desc" {
        return Err(ApiError::bad_request("order must be `asc` or `desc`"));
    }
    let ascending = order == "asc";

    let mut summaries =
        super::repositories::namespace_repository_summaries(&state, &auth.0, namespace.id).await?;
    summaries.sort_by(|a, b| {
        let primary = match sort {
            "name" => a.path.to_lowercase().cmp(&b.path.to_lowercase()),
            "size" => a.size.cmp(&b.size),
            _ => a.updated_at.cmp(&b.updated_at),
        };
        let primary = if ascending {
            primary
        } else {
            primary.reverse()
        };
        primary.then_with(|| a.path.cmp(&b.path))
    });

    let page = PageQuery {
        page: query.page,
        per_page: query.per_page,
    };
    let pagination = Pagination::from_query(&page);
    let total = summaries.len() as i64;
    let items = pagination.window(summaries);
    Ok(Json(pagination.envelope(items, total)).into_response())
}
