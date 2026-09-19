//! Control-plane JSON API (`/api/*`).
//!
//! The module owns users/social, namespaces, repositories, permissions,
//! service accounts, analytics, activity and the manifest/layer browser. Every
//! response is JSON and errors use the `ApiError` envelope.
//!
//! Repository paths are variable length, so `/api/repositories/{namespace}/
//! {*rest}` is registered as a single catch-all and disambiguated by
//! [`repositories::dispatch`]; the sub-resources (tags, manifests, layers,
//! permissions, pulls) are relative suffixes parsed from the capture. This is
//! the same technique the OCI router uses, and it is the only way to mix a
//! capture-all with sibling paths in Axum 0.8 (nesting rejects wildcards).
//!
//! Visibility is enforced with [`authz::repository_access`] and
//! [`authz::namespace_access`]. A caller without pull access always gets
//! `404 not_found`, never `403`, so private resources stay unenumerable.

pub mod activity;
pub mod analytics;
pub mod app_passwords;
pub mod layers;
pub mod namespaces;
pub mod permissions;
pub mod repositories;
pub mod service_accounts;
pub mod users;

use std::collections::{HashMap, HashSet};

use axum::Router;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::db::Db;
use crate::error::{ApiError, ApiResult};
use crate::models::{Namespace, Repository, User};
use crate::permissions as authz;
use crate::state::{AppState, AuthContext};

/// Default page size for control-plane list endpoints.
pub const PAGE_DEFAULT: i64 = 25;
/// Largest accepted page size.
pub const PAGE_MAX: i64 = 100;

// ---------------------------------------------------------------------------
// Pagination
// ---------------------------------------------------------------------------

/// `?page=` / `?per_page=` query parameters.
#[derive(Debug, Default, Deserialize)]
pub struct PageQuery {
    #[serde(default)]
    pub page: Option<i64>,
    #[serde(default)]
    pub per_page: Option<i64>,
}

/// A normalized 1-based page request.
#[derive(Debug, Clone, Copy)]
pub struct Pagination {
    pub page: i64,
    pub per_page: i64,
}

impl Pagination {
    pub fn from_query(query: &PageQuery) -> Self {
        let page = query.page.unwrap_or(1).max(1);
        let per_page = query.per_page.unwrap_or(PAGE_DEFAULT).clamp(1, PAGE_MAX);
        Self { page, per_page }
    }

    pub fn offset(&self) -> usize {
        ((self.page - 1).max(0) * self.per_page) as usize
    }

    /// Renders the standard list envelope.
    pub fn envelope<T: Serialize>(&self, items: Vec<T>, total: i64) -> Value {
        serde_json::json!({
            "items": items,
            "total": total,
            "page": self.page,
            "per_page": self.per_page,
        })
    }

    /// Applies the page window to an already-filtered collection.
    pub fn window<T>(&self, items: Vec<T>) -> Vec<T> {
        items
            .into_iter()
            .skip(self.offset())
            .take(self.per_page as usize)
            .collect()
    }
}

// ---------------------------------------------------------------------------
// Shared DTOs (the contract in docs/API.md)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
pub struct UserSummary {
    pub id: i64,
    pub username: String,
    pub first_name: Option<String>,
    pub last_name: Option<String>,
    pub avatar_url: Option<String>,
    /// Lowercase hex SHA-256 of the normalized e-mail (Libravatar key).
    pub avatar_hash: String,
}

/// Lowercase hex SHA-256 of the normalized e-mail, the public Libravatar
/// identifier. The e-mail itself is never exposed.
pub(crate) fn avatar_hash(email: &str) -> String {
    hex::encode(Sha256::digest(email.trim().to_lowercase().as_bytes()))
}

impl From<&User> for UserSummary {
    fn from(user: &User) -> Self {
        Self {
            id: user.id,
            username: user.username.clone(),
            first_name: user.first_name.clone(),
            last_name: user.last_name.clone(),
            avatar_url: user.avatar_url.clone(),
            avatar_hash: avatar_hash(&user.email),
        }
    }
}

impl From<User> for UserSummary {
    fn from(user: User) -> Self {
        Self::from(&user)
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct Platform {
    pub os: String,
    pub architecture: String,
    pub variant: Option<String>,
    pub digest: String,
    pub size: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct LayerInfo {
    pub digest: String,
    pub media_type: Option<String>,
    pub size: i64,
    pub role: String,
    pub created: Option<String>,
    pub created_by: Option<String>,
    pub comment: Option<String>,
}

/// One non-empty entry of an image config's `history` array.
#[derive(Debug, Clone)]
pub(crate) struct LayerHistory {
    pub created: Option<String>,
    pub created_by: Option<String>,
    pub comment: Option<String>,
}

/// Extracts the image config's non-empty `history` entries, in order.
///
/// `history` entries with `empty_layer: true` do not correspond to a layer and
/// are skipped, so the remaining entries map positionally onto the manifest's
/// layer descriptors. Returns an empty vec when `config` is `None` or carries no
/// usable `history` array.
pub(crate) fn layer_history(config: Option<&Value>) -> Vec<LayerHistory> {
    let Some(entries) = config
        .and_then(|config| config.get("history"))
        .and_then(Value::as_array)
    else {
        return Vec::new();
    };
    entries
        .iter()
        .filter(|entry| entry.get("empty_layer").and_then(Value::as_bool) != Some(true))
        .map(|entry| LayerHistory {
            created: string_field(entry, "created"),
            created_by: string_field(entry, "created_by"),
            comment: string_field(entry, "comment"),
        })
        .collect()
}

fn string_field(value: &Value, key: &str) -> Option<String> {
    value.get(key).and_then(Value::as_str).map(str::to_string)
}

/// Attaches the config's `history` to the `layer`-role entries of `layers`, in
/// order. Config rows keep all three fields `None`; a history shorter than the
/// layer list leaves the remaining layers `None`.
pub(crate) fn apply_layer_history(layers: &mut [LayerInfo], config: Option<&Value>) {
    let history = layer_history(config);
    for (layer, entry) in layers
        .iter_mut()
        .filter(|layer| layer.role == "layer")
        .zip(history)
    {
        layer.created = entry.created;
        layer.created_by = entry.created_by;
        layer.comment = entry.comment;
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct PlatformDetail {
    pub os: String,
    pub architecture: String,
    pub variant: Option<String>,
    pub digest: String,
    pub media_type: String,
    pub size: i64,
    pub manifest: Value,
    pub config: Option<Value>,
    pub layers: Vec<LayerInfo>,
}

#[derive(Debug, Clone, Serialize)]
pub struct PublicPermission {
    pub id: i64,
    pub subject_type: String,
    pub subject: Option<UserSummary>,
    pub can_pull: bool,
    pub can_push: bool,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize)]
pub struct TagSummary {
    pub name: String,
    pub digest: String,
    pub media_type: String,
    pub size: i64,
    pub compressed_size: i64,
    pub platforms: Vec<Platform>,
    pub pull_count: i64,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize)]
pub struct TagDetail {
    #[serde(flatten)]
    pub summary: TagSummary,
    pub manifest: Value,
    pub config: Option<Value>,
    pub layers: Vec<LayerInfo>,
    pub platform_details: Vec<PlatformDetail>,
    pub can_pull: bool,
    pub can_push: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct TagSizeEntry {
    pub repository: String,
    pub namespace: String,
    pub tag: String,
    pub total_size: i64,
    pub unique_size: i64,
    pub shared_size: i64,
    pub platforms: Vec<Platform>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize)]
pub struct RepositorySummary {
    pub id: i64,
    pub namespace: String,
    pub path: String,
    pub name: String,
    pub description: Option<String>,
    pub is_public: bool,
    pub is_hidden: bool,
    pub tag_count: i64,
    pub size: i64,
    pub pull_count: i64,
    pub updated_at: DateTime<Utc>,
}

impl RepositorySummary {
    pub fn new(
        repository: &Repository,
        namespace: String,
        tag_count: i64,
        size: i64,
        pull_count: i64,
    ) -> Self {
        Self {
            id: repository.id,
            namespace,
            path: repository.path.clone(),
            name: repository.name.clone(),
            description: repository.description.clone(),
            is_public: repository.is_public,
            is_hidden: repository.is_hidden,
            tag_count,
            size,
            pull_count,
            updated_at: repository.updated_at,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct RepositoryDetail {
    #[serde(flatten)]
    pub summary: RepositorySummary,
    pub manifest_count: i64,
    pub platform_count: i64,
    pub total_size: i64,
    pub unique_size: i64,
    pub shared_size: i64,
    pub created_at: DateTime<Utc>,
    pub created_by: Option<UserSummary>,
    pub permissions: Vec<PublicPermission>,
    pub can_pull: bool,
    pub can_push: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct NamespaceView {
    pub id: i64,
    pub name: String,
    pub kind: String,
    pub owner: Option<UserSummary>,
    pub description: Option<String>,
    pub is_public: bool,
    pub repository_count: i64,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize)]
pub struct NamespaceMemberView {
    pub user: UserSummary,
    pub role: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ActivityEntry {
    pub id: i64,
    pub kind: String,
    pub summary: String,
    pub actor: Option<UserSummary>,
    pub namespace: Option<String>,
    pub repository: Option<String>,
    pub metadata: Value,
    pub created_at: DateTime<Utc>,
}

// ---------------------------------------------------------------------------
// User helpers
// ---------------------------------------------------------------------------

pub async fn load_user(db: &Db, user_id: i64) -> ApiResult<User> {
    sqlx::query_as::<_, User>("SELECT * FROM users WHERE id = ? LIMIT 1")
        .bind(user_id)
        .fetch_optional(db)
        .await?
        .ok_or_else(|| ApiError::not_found("user not found"))
}

pub async fn user_summary(db: &Db, user_id: i64) -> ApiResult<Option<UserSummary>> {
    let user = sqlx::query_as::<_, User>("SELECT * FROM users WHERE id = ? LIMIT 1")
        .bind(user_id)
        .fetch_optional(db)
        .await?;
    Ok(user.as_ref().map(UserSummary::from))
}

/// Batches user lookups into one `IN` query (the SQL text is generated from a
/// placeholder count only, never user input).
pub async fn user_summaries(db: &Db, ids: &[i64]) -> ApiResult<HashMap<i64, UserSummary>> {
    let mut map = HashMap::new();
    if ids.is_empty() {
        return Ok(map);
    }
    let mut unique: Vec<i64> = ids.to_vec();
    unique.sort_unstable();
    unique.dedup();

    let placeholders = vec!["?"; unique.len()].join(",");
    let sql = format!("SELECT * FROM users WHERE id IN ({placeholders})");
    let mut query = sqlx::query_as::<_, User>(sqlx::AssertSqlSafe(sql));
    for id in &unique {
        query = query.bind(*id);
    }
    for user in query.fetch_all(db).await? {
        map.insert(user.id, UserSummary::from(&user));
    }
    Ok(map)
}

// ---------------------------------------------------------------------------
// Namespace / visibility helpers
// ---------------------------------------------------------------------------

pub async fn load_namespace(db: &Db, name: &str) -> ApiResult<Option<Namespace>> {
    let namespace = sqlx::query_as::<_, Namespace>(
        "SELECT * FROM namespaces WHERE name = ? COLLATE NOCASE LIMIT 1",
    )
    .bind(name)
    .fetch_optional(db)
    .await?;
    Ok(namespace)
}

pub async fn namespace_by_id(db: &Db, id: i64) -> ApiResult<Option<Namespace>> {
    let namespace = sqlx::query_as::<_, Namespace>("SELECT * FROM namespaces WHERE id = ? LIMIT 1")
        .bind(id)
        .fetch_optional(db)
        .await?;
    Ok(namespace)
}

/// A namespace is visible when the caller holds explicit access (ownership,
/// membership or a delegation) or can see at least one repository inside it, so
/// a public namespace whose entire visible set is hidden is not enumerated.
pub async fn namespace_visible(
    state: &AppState,
    actor: &AuthContext,
    namespace: &Namespace,
) -> ApiResult<bool> {
    if authz::resolve_namespace_access(state, actor, &namespace.name)
        .await?
        .explicit
    {
        return Ok(true);
    }
    let repositories = sqlx::query_as::<_, Repository>(
        "SELECT * FROM repositories WHERE namespace_id = ? ORDER BY id",
    )
    .bind(namespace.id)
    .fetch_all(&state.db)
    .await?;
    for repository in &repositories {
        if authz::repository_visible(state, actor, repository).await? {
            return Ok(true);
        }
    }
    Ok(false)
}

/// True when the caller owns the namespace (or is a global administrator).
pub fn is_namespace_owner(actor: &AuthContext, namespace: &Namespace) -> bool {
    if actor.is_admin {
        return true;
    }
    matches!(
        (actor.user_id, namespace.owner_user_id),
        (Some(actor), Some(owner)) if actor == owner
    )
}

/// True when the caller may administer the namespace: the owner, a member with
/// the `admin` role, or a global administrator.
pub async fn is_namespace_admin(
    state: &AppState,
    actor: &AuthContext,
    namespace: &Namespace,
) -> ApiResult<bool> {
    if is_namespace_owner(actor, namespace) {
        return Ok(true);
    }
    let Some(user_id) = actor.user_id else {
        return Ok(false);
    };
    let role: Option<String> = sqlx::query_scalar(
        "SELECT role FROM namespace_members WHERE namespace_id = ? AND user_id = ? LIMIT 1",
    )
    .bind(namespace.id)
    .bind(user_id)
    .fetch_optional(&state.db)
    .await?;
    Ok(role.as_deref() == Some("admin"))
}

/// Resolves + authorizes a repository for reading. Returns `404` when the
/// repository is missing or the caller may not pull it.
pub async fn visible_repository(
    state: &AppState,
    actor: &AuthContext,
    name: &str,
) -> ApiResult<Repository> {
    let Some(repository) = state
        .registry
        .find_repository(name)
        .await
        .map_err(ApiError::from)?
    else {
        return Err(ApiError::not_found("repository not found"));
    };
    if !authz::repository_visible(state, actor, &repository).await? {
        return Err(ApiError::not_found("repository not found"));
    }
    Ok(repository)
}

/// Resolves + authorizes a repository for mutation. A caller that cannot see
/// the repository gets `404`; one that can see but not push gets `403`.
pub async fn mutable_repository(
    state: &AppState,
    actor: &AuthContext,
    name: &str,
) -> ApiResult<Repository> {
    let Some(repository) = state
        .registry
        .find_repository(name)
        .await
        .map_err(ApiError::from)?
    else {
        return Err(ApiError::not_found("repository not found"));
    };
    let access = authz::repository_access(state, actor, name).await?;
    if !access.can_pull && !access.can_push {
        return Err(ApiError::not_found("repository not found"));
    }
    if !access.can_push {
        return Err(ApiError::forbidden("push access required"));
    }
    Ok(repository)
}

/// Every repository id the caller may pull.
pub async fn visible_repository_ids(
    state: &AppState,
    actor: &AuthContext,
) -> ApiResult<HashSet<i64>> {
    let repositories = sqlx::query_as::<_, Repository>("SELECT * FROM repositories ORDER BY id")
        .fetch_all(&state.db)
        .await?;
    let mut visible = HashSet::new();
    for repository in repositories {
        if authz::repository_visible(state, actor, &repository).await? {
            visible.insert(repository.id);
        }
    }
    Ok(visible)
}

/// Every repository id the caller may push to. Anonymous callers never hold
/// push access, so this is empty for them.
pub async fn pushable_repository_ids(
    state: &AppState,
    actor: &AuthContext,
) -> ApiResult<HashSet<i64>> {
    let repositories = sqlx::query_as::<_, Repository>("SELECT * FROM repositories ORDER BY id")
        .fetch_all(&state.db)
        .await?;
    let mut pushable = HashSet::new();
    for repository in repositories {
        if authz::repository_access(state, actor, &repository.name)
            .await?
            .can_push
        {
            pushable.insert(repository.id);
        }
    }
    Ok(pushable)
}

// ---------------------------------------------------------------------------
// Size accounting
// ---------------------------------------------------------------------------

/// One reachable `(tag, blob)` edge: the flattened reference graph.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct TagBlobRow {
    pub tag_id: i64,
    pub repository_id: i64,
    pub blob_id: i64,
    pub size: i64,
    pub tag_created_at: DateTime<Utc>,
}

/// Loads the flattened reachability graph for every tag in one query. A tag's
/// reachable manifests are its own manifest plus, transitively, index children;
/// the blobs are the config/layer edges of those manifests.
pub async fn load_tag_blob_rows(state: &AppState) -> ApiResult<Vec<TagBlobRow>> {
    let rows = sqlx::query_as::<_, TagBlobRow>(
        "WITH RECURSIVE reach(tag_id, manifest_id) AS ( \
             SELECT id, manifest_id FROM tags \
             UNION \
             SELECT r.tag_id, mc.child_manifest_id \
             FROM reach r JOIN manifest_children mc ON mc.parent_manifest_id = r.manifest_id \
         ), \
         tag_blob(tag_id, blob_id) AS ( \
             SELECT DISTINCT r.tag_id, mb.blob_id \
             FROM reach r JOIN manifest_blobs mb ON mb.manifest_id = r.manifest_id \
         ) \
         SELECT tb.tag_id AS tag_id, t.repository_id AS repository_id, \
                tb.blob_id AS blob_id, b.size AS size, t.created_at AS tag_created_at \
         FROM tag_blob tb \
         JOIN tags t ON t.id = tb.tag_id \
         JOIN blobs b ON b.id = tb.blob_id",
    )
    .fetch_all(&state.db)
    .await?;
    Ok(rows)
}

/// For each blob, the tag that first referenced it anywhere in the registry.
/// The earliest tag by `created_at` owns the blob; ties break on the lower tag
/// id so ownership is deterministic. Every reachable blob has exactly one owner.
pub fn blob_owners(rows: &[TagBlobRow]) -> HashMap<i64, i64> {
    let mut owners: HashMap<i64, (DateTime<Utc>, i64)> = HashMap::new();
    for row in rows {
        let candidate = (row.tag_created_at, row.tag_id);
        let replace = match owners.get(&row.blob_id) {
            Some(current) => candidate < *current,
            None => true,
        };
        if replace {
            owners.insert(row.blob_id, candidate);
        }
    }
    owners
        .into_iter()
        .map(|(blob_id, (_, tag_id))| (blob_id, tag_id))
        .collect()
}

/// Maps each tag to the repository it belongs to, derived from the graph rows.
fn tag_repositories(rows: &[TagBlobRow]) -> HashMap<i64, i64> {
    let mut repositories: HashMap<i64, i64> = HashMap::new();
    for row in rows {
        repositories.entry(row.tag_id).or_insert(row.repository_id);
    }
    repositories
}

/// The repository that owns `blob_id`, resolved through its earliest tag.
fn owner_repository(
    owners: &HashMap<i64, i64>,
    repositories: &HashMap<i64, i64>,
    blob_id: i64,
) -> Option<i64> {
    owners
        .get(&blob_id)
        .and_then(|tag_id| repositories.get(tag_id))
        .copied()
}

/// Per-tag `(total_size, unique_size)`. `total_size` is the distinct blob bytes
/// the tag reaches; `unique_size` is the storage the tag owns — every blob for
/// which it is the earliest referencing tag. `shared_size` is the remainder,
/// i.e. bytes an earlier tag already owned.
pub fn tag_size_map(rows: &[TagBlobRow]) -> HashMap<i64, (i64, i64)> {
    let owners = blob_owners(rows);
    let mut totals: HashMap<i64, (i64, i64)> = HashMap::new();
    let mut seen: HashSet<(i64, i64)> = HashSet::new();
    for row in rows {
        if !seen.insert((row.tag_id, row.blob_id)) {
            continue;
        }
        let entry = totals.entry(row.tag_id).or_insert((0, 0));
        entry.0 += row.size;
        if owners.get(&row.blob_id).copied() == Some(row.tag_id) {
            entry.1 += row.size;
        }
    }
    totals
}

/// Per-repository `(total_size, unique_size)` over distinct reachable blobs.
/// `unique_size` is the storage the repository owns: blobs whose earliest
/// referencing tag lives in this repository. A blob owned by another repository
/// counts as shared here even when this repository also reaches it.
pub fn repository_size_map(rows: &[TagBlobRow]) -> HashMap<i64, (i64, i64)> {
    let owners = blob_owners(rows);
    let repositories = tag_repositories(rows);
    let mut totals: HashMap<i64, (i64, i64)> = HashMap::new();
    let mut seen: HashSet<(i64, i64)> = HashSet::new();
    for row in rows {
        if !seen.insert((row.repository_id, row.blob_id)) {
            continue;
        }
        let entry = totals.entry(row.repository_id).or_insert((0, 0));
        entry.0 += row.size;
        if owner_repository(&owners, &repositories, row.blob_id) == Some(row.repository_id) {
            entry.1 += row.size;
        }
    }
    totals
}

/// Distinct reachable blobs (with sizes) for one repository.
pub fn repository_blob_totals(rows: &[TagBlobRow], repository_id: i64) -> (i64, i64) {
    let owners = blob_owners(rows);
    let repositories = tag_repositories(rows);
    let mut seen: HashSet<i64> = HashSet::new();
    let mut total = 0i64;
    let mut unique = 0i64;
    for row in rows.iter().filter(|row| row.repository_id == repository_id) {
        if !seen.insert(row.blob_id) {
            continue;
        }
        total += row.size;
        if owner_repository(&owners, &repositories, row.blob_id) == Some(repository_id) {
            unique += row.size;
        }
    }
    (total, unique)
}

// ---------------------------------------------------------------------------
// Activity
// ---------------------------------------------------------------------------

/// Inserts an `activity` row. Best-effort: failures are logged only.
#[allow(clippy::too_many_arguments)]
pub async fn record_activity(
    db: &Db,
    actor_user_id: Option<i64>,
    namespace_id: Option<i64>,
    repository_id: Option<i64>,
    kind: &str,
    summary: &str,
    metadata: Option<Value>,
    is_public: bool,
) {
    let metadata = metadata.and_then(|value| serde_json::to_string(&value).ok());
    let result = sqlx::query(
        "INSERT INTO activity \
         (actor_user_id, namespace_id, repository_id, kind, summary, metadata, is_public, created_at) \
         VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(actor_user_id)
    .bind(namespace_id)
    .bind(repository_id)
    .bind(kind)
    .bind(summary)
    .bind(metadata)
    .bind(is_public)
    .bind(Utc::now())
    .execute(db)
    .await;

    if let Err(err) = result {
        tracing::warn!(error = %err, "failed to record activity");
    }
}

/// Parses an activity row's stored metadata into JSON.
pub fn parse_metadata(raw: Option<&str>) -> Value {
    raw.and_then(|value| serde_json::from_str(value).ok())
        .unwrap_or(Value::Null)
}

// ---------------------------------------------------------------------------
// Router
// ---------------------------------------------------------------------------

/// Merges every control-plane submodule router.
pub fn router() -> Router<AppState> {
    Router::new()
        .merge(users::router())
        .merge(namespaces::router())
        .merge(repositories::router())
        .merge(permissions::router())
        .merge(service_accounts::router())
        .merge(app_passwords::router())
        .merge(analytics::router())
        .merge(activity::router())
        .merge(layers::router())
}

#[cfg(test)]
mod tests;
