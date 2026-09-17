//! Image and tag listing, repository detail, deletion and size accounting.

use std::collections::{HashMap, HashSet};

use axum::Json;
use axum::Router;
use axum::body::Bytes;
use axum::extract::{Path, Query, Request, State};
use axum::http::{Method, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{any, get, post};
use chrono::{DateTime, Duration, Utc};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::auth::middleware::{Auth, Authenticated};
use crate::error::{ApiError, ApiResult};
use crate::models::{Manifest, Repository, Tag};
use crate::oci::digest::Digest;
use crate::oci::media_types;
use crate::permissions as authz;
use crate::state::{AppState, AuthContext};

use super::layers;
use super::permissions as delegations;
use super::{
    LayerInfo, PageQuery, Pagination, Platform, PlatformDetail, RepositoryDetail,
    RepositorySummary, TagDetail, TagSizeEntry, TagSummary, is_namespace_owner, load_namespace,
    namespace_by_id, repository_blob_totals, tag_size_map, user_summary, visible_repository,
    visible_repository_ids,
};

// ---------------------------------------------------------------------------
// Router + dispatch
// ---------------------------------------------------------------------------

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/repositories/{namespace}/{*rest}", any(dispatch))
        .route("/api/tags", get(list_all_tags))
        .route("/api/tags/batch-delete", post(batch_delete_global))
}

/// The concrete repository endpoint a variable-length path resolves to.
enum Endpoint {
    Detail(String),
    Tags(String),
    Tag(String, String),
    BatchDeleteTags(String),
    Pulls(String),
    Permissions(String),
    Permission(String, i64),
    Manifest(String, String),
    ManifestReferences(String, String),
    LayerTree(String, String),
    LayerFile(String, String),
    LayerDownload(String, String),
}

/// Disambiguates a captured repository remainder into an [`Endpoint`].
///
/// Suffixes are resolved with `rfind`, mirroring the OCI router, so a nested
/// repository such as `alice/more/complicated/app` keeps all of its segments in
/// the repository portion. A repository path component that collides with a
/// suffix keyword (`tags`, `manifests`, `layers`, `permissions`, `pulls`) cannot
/// be addressed unambiguously; the concrete case is documented in the API.
fn parse_endpoint(rest: &str) -> Endpoint {
    if let Some(repo) = rest.strip_suffix("/tags/batch-delete") {
        return Endpoint::BatchDeleteTags(repo.to_string());
    }
    if let Some(repo) = rest.strip_suffix("/tags") {
        return Endpoint::Tags(repo.to_string());
    }
    if let Some(index) = rest.rfind("/tags/") {
        let repo = &rest[..index];
        let tag = &rest[index + "/tags/".len()..];
        if !tag.is_empty() && !tag.contains('/') {
            return Endpoint::Tag(repo.to_string(), tag.to_string());
        }
    }
    if let Some(repo) = rest.strip_suffix("/pulls") {
        return Endpoint::Pulls(repo.to_string());
    }
    if let Some(repo) = rest.strip_suffix("/permissions") {
        return Endpoint::Permissions(repo.to_string());
    }
    if let Some(index) = rest.rfind("/permissions/") {
        let repo = &rest[..index];
        let id = &rest[index + "/permissions/".len()..];
        if let Ok(id) = id.parse::<i64>() {
            return Endpoint::Permission(repo.to_string(), id);
        }
    }
    if let Some(index) = rest.rfind("/manifests/") {
        let repo = &rest[..index];
        let tail = &rest[index + "/manifests/".len()..];
        if let Some(reference) = tail.strip_suffix("/references") {
            if !reference.is_empty() && !reference.contains('/') {
                return Endpoint::ManifestReferences(repo.to_string(), reference.to_string());
            }
        }
        if !tail.is_empty() && !tail.contains('/') {
            return Endpoint::Manifest(repo.to_string(), tail.to_string());
        }
    }
    if let Some(index) = rest.rfind("/layers/") {
        let repo = &rest[..index];
        let tail = &rest[index + "/layers/".len()..];
        for (suffix, kind) in [
            ("/tree", 0u8),
            ("/file", 1),
            ("/download", 2),
        ] {
            if let Some(digest) = tail.strip_suffix(suffix) {
                if !digest.is_empty() && !digest.contains('/') {
                    return match kind {
                        0 => Endpoint::LayerTree(repo.to_string(), digest.to_string()),
                        1 => Endpoint::LayerFile(repo.to_string(), digest.to_string()),
                        _ => Endpoint::LayerDownload(repo.to_string(), digest.to_string()),
                    };
                }
            }
        }
    }
    Endpoint::Detail(rest.to_string())
}

fn full_name(namespace: &str, path: &str) -> ApiResult<String> {
    if path.is_empty() || namespace.is_empty() {
        return Err(ApiError::not_found("repository not found"));
    }
    Ok(format!("{namespace}/{path}"))
}

/// `any` handler for `/api/repositories/{namespace}/{*rest}`.
async fn dispatch(
    State(state): State<AppState>,
    auth: Auth,
    Path((namespace, rest)): Path<(String, String)>,
    method: Method,
    req: Request,
) -> ApiResult<Response> {
    let actor = auth.0;
    match parse_endpoint(&rest) {
        Endpoint::Detail(path) => {
            let name = full_name(&namespace, &path)?;
            match method {
                Method::GET => detail(state, actor, name).await,
                Method::PATCH => {
                    let payload: PatchRepository = read_json(req).await?;
                    patch(state, actor, name, payload).await
                }
                Method::DELETE => delete(state, actor, name).await,
                _ => Err(method_not_allowed()),
            }
        }
        Endpoint::Tags(path) => {
            let name = full_name(&namespace, &path)?;
            require_method(&method, Method::GET)?;
            list_tags(state, actor, name, req).await
        }
        Endpoint::Tag(path, tag) => {
            let name = full_name(&namespace, &path)?;
            match method {
                Method::GET => tag_detail(state, actor, name, tag).await,
                Method::DELETE => delete_tag(state, actor, name, tag).await,
                _ => Err(method_not_allowed()),
            }
        }
        Endpoint::BatchDeleteTags(path) => {
            let name = full_name(&namespace, &path)?;
            require_method(&method, Method::POST)?;
            let payload: BatchTags = read_json(req).await?;
            batch_delete_tags(state, actor, name, payload).await
        }
        Endpoint::Pulls(path) => {
            let name = full_name(&namespace, &path)?;
            require_method(&method, Method::GET)?;
            let query = crate::oci::query_pairs(req.uri().query());
            pulls(state, actor, name, &query).await
        }
        Endpoint::Permissions(path) => {
            let name = full_name(&namespace, &path)?;
            match method {
                Method::GET => delegations::list_repository_permissions(state, actor, name).await,
                Method::POST => {
                    let payload: delegations::CreateGrant = read_json(req).await?;
                    delegations::add_repository_permission(state, actor, name, payload).await
                }
                _ => Err(method_not_allowed()),
            }
        }
        Endpoint::Permission(path, id) => {
            let name = full_name(&namespace, &path)?;
            require_method(&method, Method::DELETE)?;
            delegations::revoke_repository_permission(state, actor, name, id).await
        }
        Endpoint::Manifest(path, reference) => {
            let name = full_name(&namespace, &path)?;
            require_method(&method, Method::GET)?;
            layers::manifest(state, actor, name, reference).await
        }
        Endpoint::ManifestReferences(path, reference) => {
            let name = full_name(&namespace, &path)?;
            require_method(&method, Method::GET)?;
            layers::manifest_references(state, actor, name, reference).await
        }
        Endpoint::LayerTree(path, digest) => {
            let name = full_name(&namespace, &path)?;
            require_method(&method, Method::GET)?;
            let query = crate::oci::query_pairs(req.uri().query());
            let path = crate::oci::query_first(&query, "path").map(str::to_string);
            layers::layer_tree(state, actor, name, digest, path).await
        }
        Endpoint::LayerFile(path, digest) => {
            let name = full_name(&namespace, &path)?;
            require_method(&method, Method::GET)?;
            let query = crate::oci::query_pairs(req.uri().query());
            let path = crate::oci::query_first(&query, "path").map(str::to_string);
            layers::layer_file(state, actor, name, digest, path).await
        }
        Endpoint::LayerDownload(path, digest) => {
            let name = full_name(&namespace, &path)?;
            require_method(&method, Method::GET)?;
            layers::layer_download(state, actor, name, digest).await
        }
    }
}

fn method_not_allowed() -> ApiError {
    ApiError::new(
        StatusCode::METHOD_NOT_ALLOWED,
        "method_not_allowed",
        "method not allowed for this resource",
    )
}

fn require_method(method: &Method, expected: Method) -> ApiResult<()> {
    if method == expected {
        Ok(())
    } else {
        Err(method_not_allowed())
    }
}

async fn read_json<T: serde::de::DeserializeOwned>(req: Request) -> ApiResult<T> {
    let bytes = read_body(req).await?;
    serde_json::from_slice(&bytes).map_err(|_| ApiError::bad_request("invalid JSON body"))
}

async fn read_body(req: Request) -> ApiResult<Bytes> {
    axum::body::to_bytes(req.into_body(), 1 << 20)
        .await
        .map_err(|_| ApiError::bad_request("request body exceeds the 1 MiB limit"))
}

// ---------------------------------------------------------------------------
// Data loading helpers
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
struct PatchRepository {
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    is_public: Option<bool>,
}

#[derive(Debug, Deserialize)]
struct BatchTags {
    #[serde(default)]
    tags: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct BatchItem {
    repository: String,
    tag: String,
}

#[derive(Debug, Deserialize)]
struct BatchItems {
    #[serde(default)]
    items: Vec<BatchItem>,
}

#[derive(Debug, Deserialize)]
struct AllTagsQuery {
    #[serde(default)]
    sort: Option<String>,
    #[serde(default)]
    order: Option<String>,
    #[serde(default)]
    namespace: Option<String>,
    #[serde(default)]
    page: Option<i64>,
    #[serde(default)]
    per_page: Option<i64>,
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

async fn tags_for_repository(state: &AppState, repository_id: i64) -> ApiResult<Vec<Tag>> {
    let tags = sqlx::query_as::<_, Tag>(
        "SELECT * FROM tags WHERE repository_id = ? ORDER BY name",
    )
    .bind(repository_id)
    .fetch_all(&state.db)
    .await?;
    Ok(tags)
}

async fn manifests_by_ids(
    db: &crate::db::Db,
    ids: &[i64],
) -> ApiResult<HashMap<i64, Manifest>> {
    let mut map = HashMap::new();
    if ids.is_empty() {
        return Ok(map);
    }
    let mut unique: Vec<i64> = ids.to_vec();
    unique.sort_unstable();
    unique.dedup();
    let placeholders = vec!["?"; unique.len()].join(",");
    let sql = format!("SELECT * FROM manifests WHERE id IN ({placeholders})");
    let mut query = sqlx::query_as::<_, Manifest>(sqlx::AssertSqlSafe(sql));
    for id in &unique {
        query = query.bind(*id);
    }
    for manifest in query.fetch_all(db).await? {
        map.insert(manifest.id, manifest);
    }
    Ok(map)
}

async fn repo_tag_counts(state: &AppState) -> ApiResult<HashMap<i64, i64>> {
    let rows = sqlx::query_as::<_, (i64, i64)>(
        "SELECT repository_id, COUNT(*) FROM tags GROUP BY repository_id",
    )
    .fetch_all(&state.db)
    .await?;
    Ok(rows.into_iter().collect())
}

async fn repo_pull_counts(state: &AppState) -> ApiResult<HashMap<i64, i64>> {
    let rows = sqlx::query_as::<_, (i64, i64)>(
        "SELECT repository_id, COALESCE(SUM(pulls), 0) FROM pull_stats GROUP BY repository_id",
    )
    .fetch_all(&state.db)
    .await?;
    Ok(rows.into_iter().collect())
}

async fn repository_pull_count(state: &AppState, repository_id: i64) -> ApiResult<i64> {
    let count: i64 = sqlx::query_scalar(
        "SELECT COALESCE(SUM(pulls), 0) FROM pull_stats WHERE repository_id = ?",
    )
    .bind(repository_id)
    .fetch_one(&state.db)
    .await?;
    Ok(count)
}

async fn blob_json(state: &AppState, digest: &str) -> Option<Value> {
    let digest = Digest::parse(digest).ok()?;
    let mut file = state.storage.open_blob(&digest).await.ok()??;
    use tokio::io::AsyncReadExt;
    let mut buffer = Vec::new();
    let mut limited = (&mut file).take(4 * 1024 * 1024);
    limited.read_to_end(&mut buffer).await.ok()?;
    serde_json::from_slice(&buffer).ok()
}

/// Resolves a tag's platform list: index children for an index, or a single
/// entry derived from the image config for a plain manifest.
pub(crate) async fn platforms_for(
    state: &AppState,
    manifest_id: i64,
    media_type: &str,
    digest: &str,
    size: i64,
) -> ApiResult<Vec<Platform>> {
    if media_types::is_index_type(media_type) {
        let rows = sqlx::query_as::<_, (String, i64, Option<String>, Option<String>, Option<String>)>(
            "SELECT m.digest, m.size, mc.platform_os, mc.platform_architecture, mc.platform_variant \
             FROM manifest_children mc JOIN manifests m ON m.id = mc.child_manifest_id \
             WHERE mc.parent_manifest_id = ? ORDER BY m.digest",
        )
        .bind(manifest_id)
        .fetch_all(&state.db)
        .await?;
        return Ok(rows
            .into_iter()
            .map(|(digest, size, os, architecture, variant)| Platform {
                os: os.unwrap_or_default(),
                architecture: architecture.unwrap_or_default(),
                variant,
                digest,
                size,
            })
            .collect());
    }

    let config_digest: Option<String> = sqlx::query_scalar(
        "SELECT b.digest FROM manifest_blobs mb JOIN blobs b ON b.id = mb.blob_id \
         WHERE mb.manifest_id = ? AND mb.role = 'config' LIMIT 1",
    )
    .bind(manifest_id)
    .fetch_optional(&state.db)
    .await?;

    let mut os = None;
    let mut architecture = None;
    let mut variant = None;
    if let Some(config_digest) = &config_digest {
        if let Some(config) = blob_json(state, config_digest).await {
            os = config.get("os").and_then(Value::as_str).map(str::to_string);
            architecture = config
                .get("architecture")
                .and_then(Value::as_str)
                .map(str::to_string);
            variant = config
                .get("variant")
                .and_then(Value::as_str)
                .map(str::to_string);
        }
    }

    // Fall back to a parent index's platform metadata when the config is absent.
    if os.is_none() && architecture.is_none() {
        let parent = sqlx::query_as::<_, (Option<String>, Option<String>, Option<String>)>(
            "SELECT platform_os, platform_architecture, platform_variant \
             FROM manifest_children WHERE child_manifest_id = ? LIMIT 1",
        )
        .bind(manifest_id)
        .fetch_optional(&state.db)
        .await?;
        if let Some((parent_os, parent_arch, parent_variant)) = parent {
            os = parent_os;
            architecture = parent_arch;
            variant = parent_variant;
        }
    }

    let _ = size;
    Ok(vec![Platform {
        os: os.unwrap_or_default(),
        architecture: architecture.unwrap_or_default(),
        variant,
        digest: digest.to_string(),
        size,
    }])
}

async fn manifest_blob_edges(
    state: &AppState,
    manifest_id: i64,
) -> ApiResult<Vec<(String, Option<String>, i64, String)>> {
    let edges = sqlx::query_as::<_, (String, Option<String>, i64, String)>(
        "SELECT b.digest, b.media_type, b.size, mb.role \
         FROM manifest_blobs mb JOIN blobs b ON b.id = mb.blob_id \
         WHERE mb.manifest_id = ? ORDER BY mb.role = 'config' DESC, b.digest",
    )
    .bind(manifest_id)
    .fetch_all(&state.db)
    .await?;
    Ok(edges)
}

async fn config_and_layers(
    state: &AppState,
    edges: Vec<(String, Option<String>, i64, String)>,
) -> (Option<Value>, Vec<LayerInfo>) {
    let mut layers = Vec::with_capacity(edges.len());
    let mut config: Option<Value> = None;
    for (digest, media_type, size, role) in edges {
        if role == "config" && config.is_none() {
            if let Some(value) = blob_json(state, &digest).await {
                config = Some(value);
            }
        }
        layers.push(LayerInfo {
            digest,
            media_type,
            size,
            role,
        });
    }
    (config, layers)
}

async fn build_tag_summary(
    state: &AppState,
    repository_id: i64,
    tag: &Tag,
    manifest: &Manifest,
    sizes: &HashMap<i64, (i64, i64)>,
) -> ApiResult<TagSummary> {
    let (total, _unique) = sizes.get(&tag.id).copied().unwrap_or((0, 0));
    let pull_count: i64 = sqlx::query_scalar(
        "SELECT COALESCE(SUM(pulls), 0) FROM pull_stats \
         WHERE repository_id = ? AND COALESCE(tag_name, '') = COALESCE(?, '')",
    )
    .bind(repository_id)
    .bind(&tag.name)
    .fetch_one(&state.db)
    .await?;
    let platforms = platforms_for(
        state,
        manifest.id,
        &manifest.media_type,
        &manifest.digest,
        manifest.size,
    )
    .await?;

    Ok(TagSummary {
        name: tag.name.clone(),
        digest: manifest.digest.clone(),
        media_type: manifest.media_type.clone(),
        size: total,
        compressed_size: total,
        platforms,
        pull_count,
        updated_at: tag.updated_at,
    })
}

async fn build_tag_detail(
    state: &AppState,
    repository_id: i64,
    tag: &Tag,
    manifest: &Manifest,
    sizes: &HashMap<i64, (i64, i64)>,
    access: authz::Access,
) -> ApiResult<TagDetail> {
    let summary = build_tag_summary(state, repository_id, tag, manifest, sizes).await?;

    let edges = sqlx::query_as::<_, (String, Option<String>, i64, String)>(
        "WITH RECURSIVE reach(manifest_id) AS ( \
             SELECT ? \
             UNION \
             SELECT mc.child_manifest_id FROM reach r \
             JOIN manifest_children mc ON mc.parent_manifest_id = r.manifest_id \
         ) \
         SELECT b.digest, b.media_type, b.size, mb.role \
         FROM reach r JOIN manifest_blobs mb ON mb.manifest_id = r.manifest_id \
         JOIN blobs b ON b.id = mb.blob_id \
         GROUP BY b.id ORDER BY mb.role = 'config' DESC, b.digest",
    )
    .bind(manifest.id)
    .fetch_all(&state.db)
    .await?;

    let (config, layers) = config_and_layers(state, edges).await;

    let manifest_json =
        serde_json::from_slice(&manifest.content).unwrap_or(Value::Null);

    let platform_details = if media_types::is_index_type(&manifest.media_type) {
        let mut details = Vec::with_capacity(summary.platforms.len());
        for platform in &summary.platforms {
            let digest = Digest::parse(&platform.digest).map_err(ApiError::from)?;
            let Some(child) = state.registry.manifest(&digest).await? else {
                continue;
            };
            let child_edges = manifest_blob_edges(state, child.id).await?;
            let (child_config, child_layers) = config_and_layers(state, child_edges).await;
            let child_manifest =
                serde_json::from_slice(&child.content).unwrap_or(Value::Null);
            details.push(PlatformDetail {
                os: platform.os.clone(),
                architecture: platform.architecture.clone(),
                variant: platform.variant.clone(),
                digest: child.digest.clone(),
                media_type: child.media_type.clone(),
                size: child.size,
                manifest: child_manifest,
                config: child_config,
                layers: child_layers,
            });
        }
        details
    } else {
        let platform = summary.platforms.first();
        let tag_edges = manifest_blob_edges(state, manifest.id).await?;
        let (tag_config, tag_layers) = config_and_layers(state, tag_edges).await;
        vec![PlatformDetail {
            os: platform.map(|p| p.os.clone()).unwrap_or_default(),
            architecture: platform.map(|p| p.architecture.clone()).unwrap_or_default(),
            variant: platform.and_then(|p| p.variant.clone()),
            digest: manifest.digest.clone(),
            media_type: manifest.media_type.clone(),
            size: manifest.size,
            manifest: manifest_json.clone(),
            config: tag_config,
            layers: tag_layers,
        }]
    };

    Ok(TagDetail {
        summary,
        manifest: manifest_json,
        config,
        layers,
        platform_details,
        can_pull: access.can_pull,
        can_push: access.can_push,
    })
}

async fn build_detail(
    state: &AppState,
    actor: &AuthContext,
    repository: &Repository,
) -> ApiResult<RepositoryDetail> {
    let namespace = namespace_by_id(&state.db, repository.namespace_id)
        .await?
        .ok_or_else(|| ApiError::not_found("namespace not found"))?;

    let tags = tags_for_repository(state, repository.id).await?;
    let manifest_ids: Vec<i64> = tags.iter().map(|tag| tag.manifest_id).collect();
    let manifests = manifests_by_ids(&state.db, &manifest_ids).await?;

    let mut platforms: HashSet<(String, String, Option<String>)> = HashSet::new();
    for tag in &tags {
        if let Some(manifest) = manifests.get(&tag.manifest_id) {
            for platform in platforms_for(
                state,
                manifest.id,
                &manifest.media_type,
                &manifest.digest,
                manifest.size,
            )
            .await?
            {
                platforms.insert((platform.os, platform.architecture, platform.variant));
            }
        }
    }

    let rowset = super::load_tag_blob_rows(state).await?;
    let (total_size, unique_size) = repository_blob_totals(&rowset, repository.id);

    let tag_count = tags.len() as i64;
    let manifest_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM manifest_repositories WHERE repository_id = ?")
            .bind(repository.id)
            .fetch_one(&state.db)
            .await?;
    let pull_count = repository_pull_count(state, repository.id).await?;

    let created_by = match repository.created_by {
        Some(created_by) => user_summary(&state.db, created_by).await?,
        None => None,
    };

    let access = authz::repository_access(state, actor, &repository.name).await?;
    let permissions = if access.can_push {
        delegations::repository_grants(state, repository.id).await?
    } else {
        Vec::new()
    };

    Ok(RepositoryDetail {
        summary: RepositorySummary::new(
            repository,
            namespace.name,
            tag_count,
            total_size,
            pull_count,
        ),
        manifest_count,
        platform_count: platforms.len() as i64,
        total_size,
        unique_size,
        shared_size: total_size - unique_size,
        created_at: repository.created_at,
        created_by,
        permissions,
        can_pull: access.can_pull,
        can_push: access.can_push,
    })
}

/// Summaries for a namespace's repositories, filtered to what the caller may
/// pull. Aggregates are computed from a single graph scan.
pub(crate) async fn namespace_repository_summaries(
    state: &AppState,
    actor: &AuthContext,
    namespace_id: i64,
) -> ApiResult<Vec<RepositorySummary>> {
    let namespace = namespace_by_id(&state.db, namespace_id)
        .await?
        .ok_or_else(|| ApiError::not_found("namespace not found"))?;
    let repositories = sqlx::query_as::<_, Repository>(
        "SELECT * FROM repositories WHERE namespace_id = ? ORDER BY name COLLATE NOCASE",
    )
    .bind(namespace_id)
    .fetch_all(&state.db)
    .await?;

    let rowset = super::load_tag_blob_rows(state).await?;
    let sizes = super::repository_size_map(&rowset);
    let tag_counts = repo_tag_counts(state).await?;
    let pull_counts = repo_pull_counts(state).await?;

    let mut summaries = Vec::new();
    for repository in repositories {
        if !authz::repository_access(state, actor, &repository.name)
            .await?
            .can_pull
        {
            continue;
        }
        let size = sizes.get(&repository.id).map(|value| value.0).unwrap_or(0);
        summaries.push(RepositorySummary::new(
            &repository,
            namespace.name.clone(),
            tag_counts.get(&repository.id).copied().unwrap_or(0),
            size,
            pull_counts.get(&repository.id).copied().unwrap_or(0),
        ));
    }
    Ok(summaries)
}

async fn load_repo_and_namespace(
    state: &AppState,
    actor: &AuthContext,
    name: &str,
) -> ApiResult<(Repository, crate::models::Namespace)> {
    let repository = visible_repository(state, actor, name).await?;
    let namespace = namespace_by_id(&state.db, repository.namespace_id)
        .await?
        .ok_or_else(|| ApiError::not_found("repository not found"))?;
    Ok((repository, namespace))
}

fn assert_owner(actor: &AuthContext, namespace: &crate::models::Namespace) -> ApiResult<()> {
    if is_namespace_owner(actor, namespace) {
        Ok(())
    } else {
        Err(ApiError::forbidden("namespace owner required"))
    }
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

async fn detail(state: AppState, actor: AuthContext, name: String) -> ApiResult<Response> {
    let repository = visible_repository(&state, &actor, &name).await?;
    let detail = build_detail(&state, &actor, &repository).await?;
    Ok(Json(detail).into_response())
}

async fn patch(
    state: AppState,
    actor: AuthContext,
    name: String,
    payload: PatchRepository,
) -> ApiResult<Response> {
    let (repository, namespace) = load_repo_and_namespace(&state, &actor, &name).await?;
    assert_owner(&actor, &namespace)?;

    sqlx::query(
        "UPDATE repositories SET \
             description = COALESCE(?, description), \
             is_public = COALESCE(?, is_public), \
             updated_at = ? \
         WHERE id = ?",
    )
    .bind(normalize(payload.description))
    .bind(payload.is_public)
    .bind(Utc::now())
    .bind(repository.id)
    .execute(&state.db)
    .await?;

    let repository = sqlx::query_as::<_, Repository>("SELECT * FROM repositories WHERE id = ?")
        .bind(repository.id)
        .fetch_one(&state.db)
        .await?;

    super::record_activity(
        &state.db,
        actor.user_id,
        Some(namespace.id),
        Some(repository.id),
        "repository.updated",
        &format!("updated repository {}", repository.name),
        Some(json!({ "repository": repository.name })),
        repository.is_public,
    )
    .await;

    let detail = build_detail(&state, &actor, &repository).await?;
    Ok(Json(detail).into_response())
}

async fn delete(state: AppState, actor: AuthContext, name: String) -> ApiResult<Response> {
    let (repository, namespace) = load_repo_and_namespace(&state, &actor, &name).await?;
    assert_owner(&actor, &namespace)?;

    sqlx::query("DELETE FROM repositories WHERE id = ?")
        .bind(repository.id)
        .execute(&state.db)
        .await?;

    super::record_activity(
        &state.db,
        actor.user_id,
        Some(namespace.id),
        None,
        "manifest.deleted",
        &format!("deleted repository {}", repository.name),
        Some(json!({ "repository": repository.name })),
        false,
    )
    .await;

    if let Ok(report) = crate::storage::gc::collect(&state.registry, &state.storage, false).await {
        for digest in &report.deleted_digests {
            state.layer_cache.invalidate(digest);
        }
        tracing::info!(
            blobs = report.blobs_deleted,
            manifests = report.manifests_deleted,
            "repository delete gc"
        );
    }

    Ok(StatusCode::NO_CONTENT.into_response())
}

async fn list_tags(
    state: AppState,
    actor: AuthContext,
    name: String,
    req: Request,
) -> ApiResult<Response> {
    let repository = visible_repository(&state, &actor, &name).await?;
    let query = crate::oci::query_pairs(req.uri().query());
    let page = PageQuery {
        page: crate::oci::query_first(&query, "page").and_then(|v| v.parse().ok()),
        per_page: crate::oci::query_first(&query, "per_page").and_then(|v| v.parse().ok()),
    };
    let pagination = Pagination::from_query(&page);

    let tags = tags_for_repository(&state, repository.id).await?;
    let manifest_ids: Vec<i64> = tags.iter().map(|tag| tag.manifest_id).collect();
    let manifests = manifests_by_ids(&state.db, &manifest_ids).await?;
    let rowset = super::load_tag_blob_rows(&state).await?;
    let sizes = tag_size_map(&rowset);

    let mut items = Vec::with_capacity(tags.len());
    for tag in &tags {
        if let Some(manifest) = manifests.get(&tag.manifest_id) {
            items.push(build_tag_summary(&state, repository.id, tag, manifest, &sizes).await?);
        }
    }
    let total = items.len() as i64;
    let windowed = pagination.window(items);
    Ok(Json(pagination.envelope(windowed, total)).into_response())
}

async fn tag_detail(
    state: AppState,
    actor: AuthContext,
    name: String,
    tag: String,
) -> ApiResult<Response> {
    let repository = visible_repository(&state, &actor, &name).await?;
    let Some(digest) = state
        .registry
        .resolve_tag(repository.id, &tag)
        .await
        .map_err(ApiError::from)?
    else {
        return Err(ApiError::not_found("tag not found"));
    };
    let Some(manifest) = state
        .registry
        .manifest(&digest)
        .await
        .map_err(ApiError::from)?
    else {
        return Err(ApiError::not_found("tag not found"));
    };
    let tag_row = sqlx::query_as::<_, Tag>(
        "SELECT * FROM tags WHERE repository_id = ? AND name = ? LIMIT 1",
    )
    .bind(repository.id)
    .bind(&tag)
    .fetch_one(&state.db)
    .await?;

    let rowset = super::load_tag_blob_rows(&state).await?;
    let sizes = tag_size_map(&rowset);
    let access = authz::repository_access(&state, &actor, &name).await?;
    let detail =
        build_tag_detail(&state, repository.id, &tag_row, &manifest, &sizes, access).await?;
    Ok(Json(detail).into_response())
}

async fn delete_tag(
    state: AppState,
    actor: AuthContext,
    name: String,
    tag: String,
) -> ApiResult<Response> {
    let repository = super::mutable_repository(&state, &actor, &name).await?;
    let manifest_id: Option<i64> =
        sqlx::query_scalar("SELECT manifest_id FROM tags WHERE repository_id = ? AND name = ? LIMIT 1")
            .bind(repository.id)
            .bind(&tag)
            .fetch_optional(&state.db)
            .await?;
    let Some(manifest_id) = manifest_id else {
        return Err(ApiError::not_found("tag not found"));
    };
    let digest: Option<String> =
        sqlx::query_scalar("SELECT digest FROM manifests WHERE id = ?")
            .bind(manifest_id)
            .fetch_optional(&state.db)
            .await?;

    let removed = state
        .registry
        .delete_tag(repository.id, &tag)
        .await
        .map_err(ApiError::from)?;
    if !removed {
        return Err(ApiError::not_found("tag not found"));
    }

    super::record_activity(
        &state.db,
        actor.user_id,
        Some(repository.namespace_id),
        Some(repository.id),
        "tag.deleted",
        &format!("deleted tag {}:{}", repository.name, tag),
        Some(json!({ "repository": repository.name, "tag": tag, "digest": digest })),
        false,
    )
    .await;

    if let Ok(report) = crate::storage::gc::collect(&state.registry, &state.storage, false).await {
        for digest in &report.deleted_digests {
            state.layer_cache.invalidate(digest);
        }
        tracing::info!(blobs = report.blobs_deleted, "tag delete gc");
    }
    Ok(StatusCode::NO_CONTENT.into_response())
}

async fn batch_delete_tags(
    state: AppState,
    actor: AuthContext,
    name: String,
    payload: BatchTags,
) -> ApiResult<Response> {
    let repository = super::mutable_repository(&state, &actor, &name).await?;
    let mut deleted = 0i64;
    for tag in payload.tags {
        if state
            .registry
            .delete_tag(repository.id, &tag)
            .await
            .map_err(ApiError::from)?
        {
            deleted += 1;
        }
    }
    if deleted > 0 {
        super::record_activity(
            &state.db,
            actor.user_id,
            Some(repository.namespace_id),
            Some(repository.id),
            "tag.deleted",
            &format!("deleted {deleted} tags from {}", repository.name),
            Some(json!({ "repository": repository.name, "deleted": deleted })),
            false,
        )
        .await;
    }
    if let Ok(report) = crate::storage::gc::collect(&state.registry, &state.storage, false).await {
        for digest in &report.deleted_digests {
            state.layer_cache.invalidate(digest);
        }
        tracing::info!(blobs = report.blobs_deleted, "batch tag delete gc");
    }
    Ok(Json(json!({ "deleted": deleted })).into_response())
}

async fn batch_delete_global(
    State(state): State<AppState>,
    Authenticated(ctx): Authenticated,
    Json(payload): Json<BatchItems>,
) -> ApiResult<Response> {
    let mut deleted = 0i64;
    for item in payload.items {
        let Ok(repository) = super::mutable_repository(&state, &ctx, &item.repository).await else {
            continue;
        };
        if state
            .registry
            .delete_tag(repository.id, &item.tag)
            .await
            .map_err(ApiError::from)?
        {
            deleted += 1;
            super::record_activity(
                &state.db,
                ctx.user_id,
                Some(repository.namespace_id),
                Some(repository.id),
                "tag.deleted",
                &format!("deleted tag {}:{}", repository.name, item.tag),
                Some(json!({ "repository": repository.name, "tag": item.tag })),
                false,
            )
            .await;
        }
    }
    if let Ok(report) = crate::storage::gc::collect(&state.registry, &state.storage, false).await {
        for digest in &report.deleted_digests {
            state.layer_cache.invalidate(digest);
        }
        tracing::info!(blobs = report.blobs_deleted, "global tag delete gc");
    }
    Ok(Json(json!({ "deleted": deleted })).into_response())
}

#[derive(sqlx::FromRow)]
struct GlobalTagRow {
    tag_id: i64,
    tag_name: String,
    tag_updated_at: DateTime<Utc>,
    repository_id: i64,
    repository_name: String,
    namespace_name: String,
    manifest_id: i64,
    digest: String,
    media_type: String,
    manifest_size: i64,
}

async fn list_all_tags(
    State(state): State<AppState>,
    auth: Auth,
    Query(query): Query<AllTagsQuery>,
) -> ApiResult<Response> {
    let actor = auth.0;
    let sort = query.sort.unwrap_or_else(|| "total_size".to_string());
    if sort != "total_size" && sort != "unique_size" {
        return Err(ApiError::bad_request(
            "sort must be `total_size` or `unique_size`",
        ));
    }
    let order = query.order.unwrap_or_else(|| "desc".to_string());
    if order != "asc" && order != "desc" {
        return Err(ApiError::bad_request("order must be `asc` or `desc`"));
    }

    let visible = visible_repository_ids(&state, &actor).await?;
    let namespace_filter = match query.namespace.as_deref() {
        Some(raw) if !raw.trim().is_empty() => {
            let namespace = load_namespace(&state.db, raw.trim())
                .await?
                .ok_or_else(|| ApiError::not_found("namespace not found"))?;
            if !super::namespace_visible(&state, &actor, &namespace).await? {
                return Err(ApiError::not_found("namespace not found"));
            }
            Some(namespace.id)
        }
        _ => None,
    };

    let rows = sqlx::query_as::<_, GlobalTagRow>(
        "SELECT t.id AS tag_id, t.name AS tag_name, t.updated_at AS tag_updated_at, \
                r.id AS repository_id, r.name AS repository_name, n.name AS namespace_name, \
                m.id AS manifest_id, m.digest AS digest, m.media_type AS media_type, \
                m.size AS manifest_size \
         FROM tags t \
         JOIN repositories r ON r.id = t.repository_id \
         JOIN namespaces n ON n.id = r.namespace_id \
         JOIN manifests m ON m.id = t.manifest_id",
    )
    .fetch_all(&state.db)
    .await?;

    let rowset = super::load_tag_blob_rows(&state).await?;
    let sizes = tag_size_map(&rowset);

    struct Candidate {
        row: GlobalTagRow,
        total: i64,
        unique: i64,
    }

    let mut candidates = Vec::new();
    for row in rows {
        if !visible.contains(&row.repository_id) {
            continue;
        }
        if let Some(namespace_id) = namespace_filter {
            let matches: Option<i64> = sqlx::query_scalar(
                "SELECT 1 FROM repositories WHERE id = ? AND namespace_id = ? LIMIT 1",
            )
            .bind(row.repository_id)
            .bind(namespace_id)
            .fetch_optional(&state.db)
            .await?;
            if matches.is_none() {
                continue;
            }
        }
        let (total, unique) = sizes.get(&row.tag_id).copied().unwrap_or((0, 0));
        candidates.push(Candidate {
            row,
            total,
            unique,
        });
    }

    if sort == "unique_size" {
        candidates.sort_by(|a, b| a.unique.cmp(&b.unique).then(a.row.repository_name.cmp(&b.row.repository_name)));
    } else {
        candidates.sort_by(|a, b| a.total.cmp(&b.total).then(a.row.repository_name.cmp(&b.row.repository_name)));
    }
    if order == "desc" {
        candidates.reverse();
    }

    let page = PageQuery {
        page: query.page,
        per_page: query.per_page,
    };
    let pagination = Pagination::from_query(&page);
    let total_count = candidates.len() as i64;
    let windowed: Vec<Candidate> = pagination.window(candidates);

    let mut items = Vec::with_capacity(windowed.len());
    for candidate in windowed {
        let platforms = platforms_for(
            &state,
            candidate.row.manifest_id,
            &candidate.row.media_type,
            &candidate.row.digest,
            candidate.row.manifest_size,
        )
        .await?;
        items.push(TagSizeEntry {
            repository: candidate.row.repository_name,
            namespace: candidate.row.namespace_name,
            tag: candidate.row.tag_name,
            total_size: candidate.total,
            unique_size: candidate.unique,
            shared_size: candidate.total - candidate.unique,
            platforms,
            updated_at: candidate.row.tag_updated_at,
        });
    }

    Ok(Json(pagination.envelope(items, total_count)).into_response())
}

async fn pulls(
    state: AppState,
    actor: AuthContext,
    name: String,
    query: &[(String, String)],
) -> ApiResult<Response> {
    let repository = visible_repository(&state, &actor, &name).await?;
    let days = crate::oci::query_first(query, "days")
        .and_then(|value| value.parse::<i64>().ok())
        .unwrap_or(30)
        .clamp(1, 365);
    let cutoff = (Utc::now() - Duration::days(days))
        .format("%Y-%m-%d")
        .to_string();

    let by_tag = sqlx::query_as::<_, (Option<String>, i64)>(
        "SELECT tag_name, COALESCE(SUM(pulls), 0) FROM pull_stats \
         WHERE repository_id = ? AND day >= ? GROUP BY tag_name ORDER BY tag_name",
    )
    .bind(repository.id)
    .bind(&cutoff)
    .fetch_all(&state.db)
    .await?;

    let by_day = sqlx::query_as::<_, (String, i64)>(
        "SELECT day, COALESCE(SUM(pulls), 0) FROM pull_stats \
         WHERE repository_id = ? AND day >= ? GROUP BY day ORDER BY day",
    )
    .bind(repository.id)
    .bind(&cutoff)
    .fetch_all(&state.db)
    .await?;

    let total: i64 = by_day.iter().map(|(_, pulls)| pulls).sum();
    let tags: Vec<Value> = by_tag
        .into_iter()
        .map(|(tag, pulls)| json!({ "tag": tag, "pulls": pulls }))
        .collect();
    let series: Vec<Value> = by_day
        .into_iter()
        .map(|(date, pulls)| json!({ "date": date, "pulls": pulls }))
        .collect();

    Ok(Json(json!({
        "repository": repository.name,
        "days": days,
        "total": total,
        "by_tag": tags,
        "by_day": series,
    }))
    .into_response())
}
