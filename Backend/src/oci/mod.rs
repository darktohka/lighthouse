//! OCI Distribution API (`/v2/*`).
//!
//! The whole registry surface is owned by this module. Because repository
//! names may contain `/` (for example `darktohka/more/complicated/project2`)
//! and matchit wildcards may only appear at the end of a route, the nested
//! `/v2` router registers a single catch-all which is disambiguated by
//! [`dispatch`] into the concrete endpoint before the submodules take over.
//! `/` and `/_catalog` are registered as static routes so they take priority
//! over the catch-all.

pub mod blobs;
pub mod catalog;
pub mod digest;
pub mod manifests;
pub mod media_types;
pub mod reference;
pub mod tags;
pub mod uploads;

use axum::Router;
use axum::body::Body;
use axum::extract::{Path, Request, State};
use axum::http::{HeaderName, HeaderValue, Method, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{any, get};
use chrono::Utc;
use serde_json::Value;
use tower_http::cors::CorsLayer;
use tower_http::set_header::SetResponseHeaderLayer;

use crate::auth::middleware::{Auth, BASIC_REALM, challenge_headers};
use crate::db::Db;
use crate::error::{ErrorCode, RegistryError};
use crate::logging;
use crate::models::Repository;
use crate::permissions::{self, Action};
use crate::state::{AppState, AuthContext};

/// Value of the `Docker-Distribution-API-Version` header on every `/v2`
/// response.
pub const API_VERSION: &str = "registry/2.0";

/// Default page size for tag and catalog listings.
const PAGE_DEFAULT: usize = 100;
/// Largest accepted page size.
const PAGE_MAX: usize = 1000;
/// Manifest bodies are capped so a malicious push cannot exhaust memory.
const MANIFEST_MAX_BYTES: usize = 4 * 1024 * 1024;

/// Builds the `/v2/*` router.
///
/// Every response — success, error or fallback — carries the
/// `Docker-Distribution-API-Version` header through a response-header layer.
pub fn router() -> Router<AppState> {
    let v2 = Router::new()
        .route("/", any(base))
        .route("/_catalog", get(catalog::get))
        .route("/{*rest}", any(dispatch))
        .fallback(oci_not_found)
        .layer(CorsLayer::permissive())
        .layer(SetResponseHeaderLayer::overriding(
            HeaderName::from_static("docker-distribution-api-version"),
            HeaderValue::from_static(API_VERSION),
        ));
    Router::new().nest("/v2/", v2)
}

/// `GET /v2/` — the API version check and the credential probe used by
/// `docker login`. Anonymous callers receive `401` with the Basic challenge so
/// the client knows to send credentials; authenticated callers receive `200 {}`.
async fn base(auth: Auth) -> Response {
    if !auth.0.is_authenticated() {
        let mut response = error_response(RegistryError::code(ErrorCode::Unauthorized));
        for (name, value) in challenge_headers() {
            response.headers_mut().insert(name, value);
        }
        return response;
    }

    json_response(StatusCode::OK, serde_json::json!({}))
}

/// Unmatched `/v2/*` paths never reach the SPA; they render the OCI envelope.
async fn oci_not_found() -> Response {
    error_response(RegistryError::code(ErrorCode::NameUnknown))
}

/// Per-request audit metadata, resolved before the body is consumed.
#[derive(Clone, Debug, Default)]
pub struct EventInfo {
    pub ip: Option<String>,
    pub user_agent: Option<String>,
}

/// The shape a `/v2` path resolves to.
enum Endpoint {
    TagsList { name: String },
    Manifest { name: String, reference: String },
    Blob { name: String, digest: String },
    UploadStart { name: String },
    UploadSession { name: String, uuid: String },
}

/// Disambiguates a captured `/v2` remainder into an [`Endpoint`].
///
/// Only the last occurrence of a marker is considered so that repository names
/// which themselves contain `manifests` or `blobs` still resolve correctly.
fn parse_endpoint(rest: &str) -> Option<Endpoint> {
    if let Some(name) = rest.strip_suffix("/tags/list") {
        if !name.is_empty() {
            return Some(Endpoint::TagsList { name: name.to_string() });
        }
    }
    if let Some(index) = rest.rfind("/manifests/") {
        let name = &rest[..index];
        let reference = &rest[index + "/manifests/".len()..];
        if !name.is_empty() && !reference.is_empty() && !reference.contains('/') {
            return Some(Endpoint::Manifest {
                name: name.to_string(),
                reference: reference.to_string(),
            });
        }
    }
    if let Some(name) = rest.strip_suffix("/blobs/uploads/") {
        if !name.is_empty() {
            return Some(Endpoint::UploadStart { name: name.to_string() });
        }
    }
    if let Some(index) = rest.rfind("/blobs/uploads/") {
        let name = &rest[..index];
        let uuid = &rest[index + "/blobs/uploads/".len()..];
        if !name.is_empty() && !uuid.is_empty() && !uuid.contains('/') {
            return Some(Endpoint::UploadSession {
                name: name.to_string(),
                uuid: uuid.to_string(),
            });
        }
    }
    if let Some(name) = rest.strip_suffix("/blobs/uploads") {
        if !name.is_empty() {
            return Some(Endpoint::UploadStart { name: name.to_string() });
        }
    }
    if let Some(index) = rest.rfind("/blobs/") {
        let name = &rest[..index];
        let digest = &rest[index + "/blobs/".len()..];
        if !name.is_empty() && !digest.is_empty() && !digest.contains('/') {
            return Some(Endpoint::Blob {
                name: name.to_string(),
                digest: digest.to_string(),
            });
        }
    }
    None
}

/// Routes a disambiguated `/v2` request to its handler.
async fn dispatch(
    State(state): State<AppState>,
    auth: Auth,
    Path(rest): Path<String>,
    req: Request,
) -> Response {
    let actor = auth.0;
    let info = EventInfo {
        ip: logging::client_ip(&req, state.config.trust_proxy),
        user_agent: logging::user_agent(&req),
    };
    let method = req.method().clone();
    let headers = req.headers().clone();
    let query = query_pairs(req.uri().query());

    let endpoint = match parse_endpoint(&rest) {
        Some(endpoint) => endpoint,
        None => return error_response(RegistryError::code(ErrorCode::NameUnknown)),
    };

    let manifest_put = matches!(endpoint, Endpoint::Manifest { .. }) && method == Method::PUT;
    let limit = if manifest_put {
        MANIFEST_MAX_BYTES
    } else {
        state
            .config
            .max_blob_size
            .map(|value| value as usize)
            .unwrap_or(usize::MAX)
    };
    let body = match axum::body::to_bytes(req.into_body(), limit).await {
        Ok(bytes) => bytes,
        Err(_) => {
            return error_response(if manifest_put {
                RegistryError::manifest_invalid("manifest body exceeds the 4 MiB limit")
            } else {
                RegistryError::size_invalid("request body exceeds the maximum allowed size")
            });
        }
    };

    let result: Result<Response, RegistryError> = match (method, endpoint) {
        (Method::GET, Endpoint::TagsList { name }) => {
            tags::list(&state, &actor, &headers, &info, &name, &query).await
        }
        (Method::GET, Endpoint::Manifest { name, reference }) => {
            manifests::get(&state, &actor, &headers, &info, &name, &reference, false).await
        }
        (Method::HEAD, Endpoint::Manifest { name, reference }) => {
            manifests::get(&state, &actor, &headers, &info, &name, &reference, true).await
        }
        (Method::PUT, Endpoint::Manifest { name, reference }) => {
            manifests::put(&state, &actor, &headers, &info, &name, &reference, &body).await
        }
        (Method::DELETE, Endpoint::Manifest { name, reference }) => {
            manifests::delete(&state, &actor, &headers, &info, &name, &reference).await
        }
        (Method::GET, Endpoint::Blob { name, digest }) => {
            blobs::get(&state, &actor, &headers, &info, &name, &digest, false).await
        }
        (Method::HEAD, Endpoint::Blob { name, digest }) => {
            blobs::get(&state, &actor, &headers, &info, &name, &digest, true).await
        }
        (Method::DELETE, Endpoint::Blob { name, digest }) => {
            blobs::delete(&state, &actor, &info, &name, &digest).await
        }
        (Method::POST, Endpoint::UploadStart { name }) => {
            uploads::start(&state, &actor, &headers, &info, &name, &query, &body).await
        }
        (Method::GET, Endpoint::UploadSession { name, uuid }) => {
            uploads::status(&state, &actor, &headers, &info, &name, &uuid).await
        }
        (Method::HEAD, Endpoint::UploadSession { name, uuid }) => {
            uploads::status(&state, &actor, &headers, &info, &name, &uuid).await
        }
        (Method::PATCH, Endpoint::UploadSession { name, uuid }) => {
            uploads::patch(&state, &actor, &headers, &info, &name, &uuid, &body).await
        }
        (Method::PUT, Endpoint::UploadSession { name, uuid }) => {
            uploads::complete(&state, &actor, &headers, &info, &name, &uuid, &query, &body).await
        }
        (Method::DELETE, Endpoint::UploadSession { name, uuid }) => {
            uploads::cancel(&state, &actor, &headers, &info, &name, &uuid).await
        }
        _ => Err(RegistryError::unsupported("method not allowed for this resource")),
    };

    result.unwrap_or_else(error_response)
}

// ---- responses --------------------------------------------------------------

/// Builds a response from a status and body without panicking on bad values.
pub fn respond(status: StatusCode, body: Body) -> Response {
    let mut response = Response::new(body);
    *response.status_mut() = status;
    response
}

/// A JSON response with an explicit status.
pub fn json_response(status: StatusCode, value: Value) -> Response {
    let mut response = axum::Json(value).into_response();
    *response.status_mut() = status;
    response
}

/// Adds a header when the value is a valid header value.
pub fn set_header(response: Response, name: &'static str, value: impl AsRef<str>) -> Response {
    let mut response = response;
    if let Ok(value) = HeaderValue::from_str(value.as_ref()) {
        response
            .headers_mut()
            .insert(HeaderName::from_static(name), value);
    }
    response
}

/// Renders a [`RegistryError`], attaching the Basic challenge on `401`s.
pub fn error_response(err: RegistryError) -> Response {
    let unauthenticated = permissions::is_unauthenticated(&err);
    let mut response = err.into_response();
    if unauthenticated {
        response.headers_mut().insert(
            header::WWW_AUTHENTICATE,
            HeaderValue::from_static(BASIC_REALM),
        );
    }
    response
}

/// Maps a storage-layer failure to an internal registry error.
pub fn storage_failure(err: anyhow::Error) -> RegistryError {
    tracing::error!(error = ?err, "content store failure");
    RegistryError::internal("storage error")
}

// ---- request helpers --------------------------------------------------------

/// Validates repository-name grammar, producing `NAME_INVALID` otherwise.
pub fn validate_name(name: &str) -> Result<(), RegistryError> {
    if reference::validate_repository_name(name) {
        Ok(())
    } else {
        Err(RegistryError::name_invalid(name))
    }
}

/// Validates the name and enforces `action` for the actor.
pub async fn ensure_authorized(
    state: &AppState,
    actor: &AuthContext,
    name: &str,
    action: Action,
) -> Result<(), RegistryError> {
    validate_name(name)?;
    permissions::authorize_oci(state, actor, name, action).await?;
    Ok(())
}

/// Validates, authorizes and resolves an existing repository.
pub async fn find_repo(
    state: &AppState,
    actor: &AuthContext,
    name: &str,
    action: Action,
) -> Result<Repository, RegistryError> {
    validate_name(name)?;
    let Some(repository) = state.registry.find_repository(name).await? else {
        return Err(RegistryError::name_unknown(name));
    };
    permissions::authorize_oci(state, actor, name, action).await?;
    Ok(repository)
}

/// Validates, authorizes and creates a repository on demand for a push.
pub async fn ensure_repo_for_push(
    state: &AppState,
    actor: &AuthContext,
    name: &str,
) -> Result<Repository, RegistryError> {
    validate_name(name)?;
    permissions::authorize_oci(state, actor, name, Action::Push).await?;
    state.registry.ensure_repository(name).await
}

/// Parses an `?n=` page size, enforcing the default, bounds and grammar.
pub fn parse_page_size(query: &[(String, String)]) -> Result<usize, RegistryError> {
    match query_first(query, "n") {
        None => Ok(PAGE_DEFAULT),
        Some(raw) => {
            let parsed: i64 = raw
                .parse()
                .map_err(|_| RegistryError::pagination_invalid(format!("invalid value for `n`: {raw}")))?;
            if parsed < 0 {
                return Err(RegistryError::pagination_invalid(format!(
                    "invalid value for `n`: {raw}"
                )));
            }
            Ok((parsed as usize).min(PAGE_MAX))
        }
    }
}

/// Percent-decoded query-string pairs, preserving order.
pub fn query_pairs(raw: Option<&str>) -> Vec<(String, String)> {
    match raw {
        Some(raw) => url::form_urlencoded::parse(raw.as_bytes())
            .map(|(key, value)| (key.into_owned(), value.into_owned()))
            .collect(),
        None => Vec::new(),
    }
}

/// First value for `key` in a decoded query.
pub fn query_first<'a>(query: &'a [(String, String)], key: &str) -> Option<&'a str> {
    query
        .iter()
        .find(|(name, _)| name == key)
        .map(|(_, value)| value.as_str())
}

/// Enforces the optional `MAX_BLOB_SIZE` ceiling.
pub fn check_blob_size(
    state: &AppState,
    current: u64,
    additional: u64,
) -> Result<(), RegistryError> {
    if let Some(limit) = state.config.max_blob_size {
        if current.saturating_add(additional) > limit {
            return Err(RegistryError::size_invalid(
                "blob exceeds the configured maximum size",
            ));
        }
    }
    Ok(())
}

/// Builds the upload-session service over the shared state.
pub fn uploads_service(state: &AppState) -> crate::storage::upload::Uploads {
    crate::storage::upload::Uploads::new(
        state.db.clone(),
        std::sync::Arc::clone(&state.storage),
        std::sync::Arc::clone(&state.registry),
    )
}

/// Today's UTC day key used by `pull_stats`.
pub fn today() -> String {
    Utc::now().format("%Y-%m-%d").to_string()
}

// ---- audit logging ----------------------------------------------------------

/// Inserts a `registry_events` row. Best-effort: failures are logged only.
#[allow(clippy::too_many_arguments)]
pub async fn log_registry_event(
    db: &Db,
    info: &EventInfo,
    actor: &AuthContext,
    action: &str,
    repository: Option<&Repository>,
    reference: Option<&str>,
    digest: Option<&str>,
    media_type: Option<&str>,
    status: u16,
) {
    let result = sqlx::query(
        "INSERT INTO registry_events \
         (action, repository_id, repository_name, reference, digest, media_type, \
          user_id, service_account_id, is_anonymous, ip, user_agent, status, created_at) \
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(action)
    .bind(repository.map(|repo| repo.id))
    .bind(repository.map(|repo| repo.name.as_str()))
    .bind(reference)
    .bind(digest)
    .bind(media_type)
    .bind(actor.user_id)
    .bind(actor.service_account_id)
    .bind(!actor.is_authenticated())
    .bind(info.ip.as_deref())
    .bind(info.user_agent.as_deref())
    .bind(status as i64)
    .bind(Utc::now())
    .execute(db)
    .await;

    if let Err(err) = result {
        tracing::warn!(error = %err, "failed to record registry event");
    }
}

/// Records a pull: registry event, raw `pull_events` row and today's rollup.
#[allow(clippy::too_many_arguments)]
pub async fn log_pull(
    state: &AppState,
    actor: &AuthContext,
    info: &EventInfo,
    action: &str,
    repository: &Repository,
    tag: Option<&str>,
    digest: &str,
    media_type: Option<&str>,
    manifest_id: Option<i64>,
) {
    log_registry_event(
        &state.db,
        info,
        actor,
        action,
        Some(repository),
        tag.or(Some(digest)),
        Some(digest),
        media_type,
        200,
    )
    .await;

    let insert = sqlx::query(
        "INSERT INTO pull_events \
         (repository_id, manifest_id, tag_name, digest, user_id, is_anonymous, ip, user_agent, created_at) \
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(repository.id)
    .bind(manifest_id)
    .bind(tag)
    .bind(digest)
    .bind(actor.user_id)
    .bind(!actor.is_authenticated())
    .bind(info.ip.as_deref())
    .bind(info.user_agent.as_deref())
    .bind(Utc::now())
    .execute(&state.db)
    .await;
    if let Err(err) = insert {
        tracing::warn!(error = %err, "failed to record pull event");
    }

    let day = today();
    let update = sqlx::query(
        "UPDATE pull_stats SET pulls = pulls + 1 \
         WHERE repository_id = ? AND day = ? AND COALESCE(tag_name, '') = COALESCE(?, '')",
    )
    .bind(repository.id)
    .bind(&day)
    .bind(tag)
    .execute(&state.db)
    .await;

    match update {
        Ok(result) if result.rows_affected() == 0 => {
            let insert = sqlx::query(
                "INSERT INTO pull_stats (repository_id, tag_name, day, pulls) VALUES (?, ?, ?, 1)",
            )
            .bind(repository.id)
            .bind(tag)
            .bind(&day)
            .execute(&state.db)
            .await;
            if let Err(err) = insert {
                tracing::warn!(error = %err, "failed to roll up pull stats");
            }
        }
        Ok(_) => {}
        Err(err) => tracing::warn!(error = %err, "failed to update pull stats"),
    }
}

/// Inserts an `activity` row. Best-effort: failures are logged only.
pub async fn log_activity(
    db: &Db,
    actor: &AuthContext,
    repository: &Repository,
    kind: &str,
    summary: &str,
    metadata: Option<Value>,
) {
    let metadata = metadata.and_then(|value| serde_json::to_string(&value).ok());
    let result = sqlx::query(
        "INSERT INTO activity \
         (actor_user_id, namespace_id, repository_id, kind, summary, metadata, is_public, created_at) \
         VALUES (?, ?, ?, ?, ?, ?, 1, ?)",
    )
    .bind(actor.user_id)
    .bind(repository.namespace_id)
    .bind(repository.id)
    .bind(kind)
    .bind(summary)
    .bind(metadata)
    .bind(Utc::now())
    .execute(db)
    .await;

    if let Err(err) = result {
        tracing::warn!(error = %err, "failed to record activity");
    }
}

#[cfg(test)]
mod tests;
