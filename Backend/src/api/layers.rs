//! Manifest / config JSON and the layer filesystem browser.
//!
//! Layer archives are decompressed transparently based on the blob media type
//! (`tar`, `tar+gzip`, `tar+zstd`). Nothing is ever extracted to disk: the tar
//! is read as an in-memory stream in a blocking task, with strict caps on the
//! number of entries, decompressed bytes and the size of a single browsed file.
//! Requested paths are sanitized (no absolute paths, no `..`, no NUL, no
//! symlink traversal) before any lookup. See `docs/LAYER_BROWSER.md`.

use std::collections::HashMap;
use std::io::{self, Read};

use axum::Json;
use axum::Router;
use axum::body::Body;
use axum::extract::{Path, State};
use axum::http::{HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use serde::Serialize;
use serde_json::Value;
use tar::EntryType;

use crate::auth::middleware::Auth;
use crate::error::{ApiError, ApiResult};
use crate::models::Blob;
use crate::oci::digest::Digest;
use crate::oci::media_types;
use crate::permissions;
use crate::state::{AppState, AuthContext};
use crate::oci::reference;
use tokio_util::io::ReaderStream;

use super::visible_repository;

/// Maximum number of tar entries inspected per request.
const MAX_ENTRIES: u64 = 100_000;
/// Maximum decompressed bytes read per request (512 MiB).
const MAX_SCAN_BYTES: u64 = 512 * 1024 * 1024;
/// Maximum size of a single file returned by the `file` endpoint (5 MiB).
const MAX_FILE_BYTES: u64 = 5 * 1024 * 1024;
/// Maximum entries returned by a single `tree` level.
const MAX_TREE_ENTRIES: usize = 10_000;
/// Maximum accepted path length.
const MAX_PATH_LEN: usize = 4096;
/// Marker for a decompression cap violation, mapped to `413`.
const CAP_MESSAGE: &str = "layer exceeds the decompression cap";

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/blobs/{digest}", get(blob))
        .route("/api/blobs/{digest}/json", get(blob_json))
}

// ---------------------------------------------------------------------------
// Response helpers
// ---------------------------------------------------------------------------

fn bytes_response(status: StatusCode, body: Vec<u8>, content_type: &str) -> Response {
    let mut response = Response::new(Body::from(body));
    *response.status_mut() = status;
    apply_headers(response, content_type, None)
}

fn apply_headers(
    mut response: Response,
    content_type: &str,
    content_length: Option<u64>,
) -> Response {
    let value = HeaderValue::from_str(content_type)
        .unwrap_or_else(|_| HeaderValue::from_static("application/octet-stream"));
    response.headers_mut().insert(header::CONTENT_TYPE, value);
    if let Some(length) = content_length.and_then(|length| HeaderValue::from_str(&length.to_string()).ok()) {
        response.headers_mut().insert(header::CONTENT_LENGTH, length);
    }
    response
}

fn stream_blob(file: tokio::fs::File, size: u64, content_type: &str) -> Response {
    let stream = ReaderStream::new(file);
    let mut response = Response::new(Body::from_stream(stream));
    *response.status_mut() = StatusCode::OK;
    apply_headers(
        response,
        content_type,
        Some(size),
    )
}

fn too_large(message: &str) -> ApiError {
    ApiError::new(StatusCode::PAYLOAD_TOO_LARGE, "payload_too_large", message)
}

fn not_found(message: &str) -> ApiError {
    ApiError::not_found(message)
}

// ---------------------------------------------------------------------------
// Path sanitization
// ---------------------------------------------------------------------------

/// Sanitizes a requested path: no NUL, no absolute path, no `..`, no empty
/// components. Returns the normalized relative path.
fn sanitize_request_path(raw: &str) -> ApiResult<String> {
    if raw.contains('\0') {
        return Err(ApiError::bad_request("path contains a NUL byte"));
    }
    if raw.len() > MAX_PATH_LEN {
        return Err(ApiError::bad_request("path is too long"));
    }
    if raw.is_empty() || raw.chars().all(|c| c == '/' || c == '.') {
        return Ok(String::new());
    }
    if raw.starts_with('/') {
        return Err(ApiError::bad_request("absolute paths are not allowed"));
    }
    let mut components = Vec::new();
    for component in raw.split('/') {
        if component.is_empty() || component == "." {
            continue;
        }
        if component == ".." {
            return Err(ApiError::bad_request("path traversal is not allowed"));
        }
        components.push(component);
    }
    Ok(components.join("/"))
}

/// Sanitizes a path read from a tar header, rejecting traversal and absolute
/// paths. Returns `None` for entries that should be skipped.
fn sanitize_entry_path(raw: &str) -> Option<String> {
    if raw.contains('\0') || raw.starts_with('/') || raw.len() > MAX_PATH_LEN {
        return None;
    }
    let mut components = Vec::new();
    for component in raw.split('/') {
        if component.is_empty() || component == "." {
            continue;
        }
        if component == ".." {
            return None;
        }
        components.push(component);
    }
    Some(components.join("/"))
}

fn parent_of(path: &str) -> String {
    match path.rsplit_once('/') {
        Some((parent, _)) => parent.to_string(),
        None => String::new(),
    }
}

// ---------------------------------------------------------------------------
// Tar reading
// ---------------------------------------------------------------------------

enum Compression {
    Plain,
    Gzip,
    Zstd,
}

fn compression_for(media_type: Option<&str>) -> Compression {
    let media_type = media_type.unwrap_or("").to_ascii_lowercase();
    if media_type.contains("zstd") {
        Compression::Zstd
    } else if media_type.contains("gzip") {
        Compression::Gzip
    } else {
        Compression::Plain
    }
}

fn decompressed(file: std::fs::File, media_type: Option<&str>) -> ApiResult<Box<dyn Read + Send>> {
    match compression_for(media_type) {
        Compression::Plain => Ok(Box::new(file)),
        Compression::Gzip => Ok(Box::new(flate2::read::GzDecoder::new(file))),
        Compression::Zstd => {
            let decoder = zstd::stream::read::Decoder::new(file)
                .map_err(|err| ApiError::internal(format!("zstd decoder: {err}")))?;
            Ok(Box::new(decoder))
        }
    }
}

/// A reader that fails once more than `remaining` bytes have been produced.
struct CappedReader<R> {
    inner: R,
    remaining: u64,
}

impl<R: Read> Read for CappedReader<R> {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        if self.remaining == 0 {
            let mut probe = [0u8; 1];
            return match self.inner.read(&mut probe) {
                Ok(0) => Ok(0),
                Ok(_) => Err(io::Error::other(CAP_MESSAGE)),
                Err(err) => Err(err),
            };
        }
        let allowed = buffer.len().min(self.remaining as usize);
        let read = self.inner.read(&mut buffer[..allowed])?;
        self.remaining -= read as u64;
        Ok(read)
    }
}

fn map_io(err: io::Error) -> ApiError {
    if err.to_string().contains(CAP_MESSAGE) {
        too_large(CAP_MESSAGE)
    } else {
        ApiError::internal(format!("layer read failed: {err}"))
    }
}

#[derive(Clone)]
struct EntryData {
    kind: String,
    size: i64,
    mode: u32,
    link_target: Option<String>,
}

impl EntryData {
    fn directory() -> Self {
        Self {
            kind: "dir".to_string(),
            size: 0,
            mode: 0o755,
            link_target: None,
        }
    }
}

fn entry_kind(entry_type: EntryType) -> String {
    if entry_type.is_dir() {
        "dir".to_string()
    } else if entry_type.is_symlink() {
        "symlink".to_string()
    } else {
        "file".to_string()
    }
}

/// Reads the archive through the entry stream, applying the whiteout and
/// repeated-path rules, and returns the merged path map.
fn merge_entries(reader: Box<dyn Read + Send>) -> ApiResult<HashMap<String, EntryData>> {
    let mut archive = tar::Archive::new(CappedReader {
        inner: reader,
        remaining: MAX_SCAN_BYTES,
    });
    let mut merged: HashMap<String, EntryData> = HashMap::new();
    let mut pending_long_name: Option<Vec<u8>> = None;
    let mut count = 0u64;

    for entry in archive.entries().map_err(map_io)? {
        let mut entry = entry.map_err(map_io)?;
        count += 1;
        if count > MAX_ENTRIES {
            return Err(too_large("layer has too many entries"));
        }

        let entry_type = entry.header().entry_type();
        match entry_type {
            EntryType::GNULongName => {
                let mut buffer = Vec::new();
                let _ = (&mut entry).take(4096).read_to_end(&mut buffer);
                pending_long_name = Some(buffer);
                continue;
            }
            EntryType::XHeader | EntryType::XGlobalHeader | EntryType::GNULongLink => continue,
            _ => {}
        }

        let raw_path = match pending_long_name.take() {
            Some(bytes) => match String::from_utf8(bytes) {
                Ok(path) => path,
                Err(_) => continue,
            },
            None => match entry.path() {
                Ok(path) => path.to_string_lossy().to_string(),
                Err(_) => continue,
            },
        };
        let Some(path) = sanitize_entry_path(&raw_path) else {
            continue;
        };
        if path.is_empty() {
            continue;
        }

        let name = path.rsplit('/').next().unwrap_or(path.as_str()).to_string();
        if name == ".wh..wh..opq" {
            let directory = parent_of(&path);
            let stale: Vec<String> = merged
                .keys()
                .filter(|key| parent_of(key) == directory)
                .cloned()
                .collect();
            for key in stale {
                merged.remove(&key);
            }
            continue;
        }
        if let Some(stripped) = name.strip_prefix(".wh.") {
            let directory = parent_of(&path);
            let target = if directory.is_empty() {
                stripped.to_string()
            } else {
                format!("{directory}/{stripped}")
            };
            merged.remove(&target);
            continue;
        }

        let size = entry.header().size().unwrap_or(0) as i64;
        let mode = entry.header().mode().unwrap_or(0);
        let link_target = if entry_type.is_symlink() || entry_type.is_hard_link() {
            entry
                .link_name()
                .ok()
                .flatten()
                .map(|target| target.to_string_lossy().to_string())
        } else {
            None
        };
        merged.insert(
            path,
            EntryData {
                kind: entry_kind(entry_type),
                size,
                mode,
                link_target,
            },
        );
    }

    Ok(merged)
}

#[derive(Serialize)]
struct LayerTreeEntry {
    name: String,
    path: String,
    kind: String,
    size: i64,
    mode: u32,
    link_target: Option<String>,
}

fn list_level(
    merged: &HashMap<String, EntryData>,
    path: &str,
) -> ApiResult<Vec<LayerTreeEntry>> {
    let prefix = if path.is_empty() {
        String::new()
    } else {
        format!("{path}/")
    };
    let mut children: HashMap<String, EntryData> = HashMap::new();
    for (entry_path, data) in merged {
        if entry_path == path {
            continue;
        }
        let rest = if path.is_empty() {
            entry_path.as_str()
        } else if let Some(rest) = entry_path.strip_prefix(&prefix) {
            rest
        } else {
            continue;
        };
        if rest.is_empty() {
            continue;
        }
        match rest.split_once('/') {
            Some((head, _)) => {
                children.entry(head.to_string()).or_insert_with(EntryData::directory);
            }
            None => {
                children.insert(rest.to_string(), data.clone());
            }
        }
    }

    let mut names: Vec<String> = children.keys().cloned().collect();
    names.sort();
    if names.len() > MAX_TREE_ENTRIES {
        return Err(too_large("directory has too many entries"));
    }

    Ok(names
        .into_iter()
        .filter_map(|name| {
            let data = children.get(&name)?;
            let full_path = if path.is_empty() {
                name.clone()
            } else {
                format!("{path}/{name}")
            };
            Some(LayerTreeEntry {
                name,
                path: full_path,
                kind: data.kind.clone(),
                size: data.size,
                mode: data.mode,
                link_target: data.link_target.clone(),
            })
        })
        .collect())
}

fn find_file(
    reader: Box<dyn Read + Send>,
    target: &str,
) -> ApiResult<(Vec<u8>, String)> {
    let mut archive = tar::Archive::new(CappedReader {
        inner: reader,
        remaining: MAX_SCAN_BYTES,
    });
    let mut pending_long_name: Option<Vec<u8>> = None;
    let mut count = 0u64;

    for entry in archive.entries().map_err(map_io)? {
        let mut entry = entry.map_err(map_io)?;
        count += 1;
        if count > MAX_ENTRIES {
            return Err(too_large("layer has too many entries"));
        }
        let entry_type = entry.header().entry_type();
        match entry_type {
            EntryType::GNULongName => {
                let mut buffer = Vec::new();
                let _ = (&mut entry).take(4096).read_to_end(&mut buffer);
                pending_long_name = Some(buffer);
                continue;
            }
            EntryType::XHeader | EntryType::XGlobalHeader | EntryType::GNULongLink => continue,
            _ => {}
        }
        let raw_path = match pending_long_name.take() {
            Some(bytes) => match String::from_utf8(bytes) {
                Ok(path) => path,
                Err(_) => continue,
            },
            None => match entry.path() {
                Ok(path) => path.to_string_lossy().to_string(),
                Err(_) => continue,
            },
        };
        let Some(path) = sanitize_entry_path(&raw_path) else {
            continue;
        };
        if path != target {
            continue;
        }
        if entry_type.is_dir() {
            return Err(ApiError::bad_request("path is a directory"));
        }
        if entry_type.is_symlink() {
            return Err(ApiError::bad_request("cannot read a symlink"));
        }
        if !entry_type.is_file() && !entry_type.is_contiguous() {
            return Err(ApiError::bad_request("unsupported layer entry type"));
        }

        let mut buffer = Vec::new();
        (&mut entry)
            .take(MAX_FILE_BYTES + 1)
            .read_to_end(&mut buffer)
            .map_err(map_io)?;
        if buffer.len() as u64 > MAX_FILE_BYTES {
            return Err(too_large("file exceeds the 5 MiB browse limit"));
        }
        let content_type = mime_guess::from_path(&path)
            .first_or_octet_stream()
            .to_string();
        return Ok((buffer, content_type));
    }

    Err(not_found("path not found in layer"))
}

async fn run_blocking<T, F>(job: F) -> ApiResult<T>
where
    T: Send + 'static,
    F: FnOnce() -> ApiResult<T> + Send + 'static,
{
    tokio::task::spawn_blocking(job)
        .await
        .map_err(|err| {
            tracing::error!(error = %err, "layer task panicked");
            ApiError::internal("layer reader failed")
        })?
}

// ---------------------------------------------------------------------------
// Blob resolution
// ---------------------------------------------------------------------------

async fn visible_blob(state: &AppState, actor: &AuthContext, digest: &str) -> ApiResult<Blob> {
    let parsed = Digest::parse(digest).map_err(|_| ApiError::not_found("blob not found"))?;
    let Some(blob) = state.registry.blob_stat(&parsed).await.map_err(ApiError::from)? else {
        return Err(not_found("blob not found"));
    };

    let repository_names = sqlx::query_scalar::<_, String>(
        "SELECT r.name FROM blob_repositories br \
         JOIN repositories r ON r.id = br.repository_id \
         WHERE br.blob_id = ?",
    )
    .bind(blob.id)
    .fetch_all(&state.db)
    .await?;

    for name in repository_names {
        if permissions::repository_access(state, actor, &name)
            .await?
            .can_pull
        {
            return Ok(blob);
        }
    }
    Err(not_found("blob not found"))
}

async fn open_blob(state: &AppState, digest: &Digest) -> ApiResult<tokio::fs::File> {
    state
        .storage
        .open_blob(digest)
        .await
        .map_err(|_| ApiError::internal("storage error"))?
        .ok_or_else(|| not_found("blob not found"))
}

// ---------------------------------------------------------------------------
// Global blob endpoints
// ---------------------------------------------------------------------------

async fn blob(
    State(state): State<AppState>,
    auth: Auth,
    Path(digest): Path<String>,
) -> ApiResult<Response> {
    let blob = visible_blob(&state, &auth.0, &digest).await?;
    let parsed = Digest::parse(&digest).map_err(|_| not_found("blob not found"))?;
    let file = open_blob(&state, &parsed).await?;
    Ok(stream_blob(file, blob.size.max(0) as u64, "application/octet-stream"))
}

async fn blob_json(
    State(state): State<AppState>,
    auth: Auth,
    Path(digest): Path<String>,
) -> ApiResult<Response> {
    let blob = visible_blob(&state, &auth.0, &digest).await?;
    let parsed = Digest::parse(&digest).map_err(|_| not_found("blob not found"))?;
    let mut file = open_blob(&state, &parsed).await?;
    let mut buffer = Vec::new();
    {
        use tokio::io::AsyncReadExt;
        (&mut file)
            .take(MAX_FILE_BYTES + 1)
            .read_to_end(&mut buffer)
            .await
            .map_err(|_| ApiError::internal("storage error"))?;
    }
    if buffer.len() as u64 > MAX_FILE_BYTES {
        return Err(too_large("blob exceeds the 5 MiB browse limit"));
    }
    let _ = blob;
    let value: Value = serde_json::from_slice(&buffer)
        .map_err(|_| ApiError::bad_request("blob is not valid JSON"))?;
    Ok(Json(value).into_response())
}

// ---------------------------------------------------------------------------
// Repository-scoped manifest + layer endpoints
// ---------------------------------------------------------------------------

#[derive(Serialize)]
struct LayerReference {
    digest: String,
    media_type: Option<String>,
    size: i64,
    role: String,
}

async fn resolve_manifest(
    state: &AppState,
    actor: &AuthContext,
    repo_name: &str,
    reference: &str,
) -> ApiResult<(crate::models::Repository, crate::models::Manifest)> {
    let repository = visible_repository(state, actor, repo_name).await?;
    let digest = match Digest::parse(reference) {
        Ok(digest) => digest,
        Err(_) => {
            if !reference::validate_tag(reference) {
                return Err(not_found("manifest not found"));
            }
            state
                .registry
                .resolve_tag(repository.id, reference)
                .await
                .map_err(ApiError::from)?
                .ok_or_else(|| not_found("manifest not found"))?
        }
    };
    let manifest = state
        .registry
        .manifest(&digest)
        .await
        .map_err(ApiError::from)?
        .ok_or_else(|| not_found("manifest not found"))?;
    if !state
        .registry
        .manifest_in_repository(repository.id, &digest)
        .await
        .map_err(ApiError::from)?
    {
        return Err(not_found("manifest not found"));
    }
    Ok((repository, manifest))
}

pub async fn manifest(
    state: AppState,
    actor: AuthContext,
    repo_name: String,
    reference: String,
) -> ApiResult<Response> {
    let (_repository, manifest) = resolve_manifest(&state, &actor, &repo_name, &reference).await?;
    Ok(bytes_response(
        StatusCode::OK,
        manifest.content,
        &manifest.media_type,
    ))
}

pub async fn manifest_references(
    state: AppState,
    actor: AuthContext,
    repo_name: String,
    reference: String,
) -> ApiResult<Response> {
    let (_repository, manifest) = resolve_manifest(&state, &actor, &repo_name, &reference).await?;
    let mut references = Vec::new();
    let parsed: Option<Value> = serde_json::from_slice(&manifest.content).ok();

    if media_types::is_index_type(&manifest.media_type) {
        if let Some(entries) = parsed
            .as_ref()
            .and_then(|value| value.get("manifests"))
            .and_then(Value::as_array)
        {
            for entry in entries {
                if let Some(digest) = entry.get("digest").and_then(Value::as_str) {
                    references.push(LayerReference {
                        digest: digest.to_string(),
                        media_type: entry
                            .get("mediaType")
                            .and_then(Value::as_str)
                            .map(str::to_string),
                        size: entry.get("size").and_then(Value::as_u64).unwrap_or(0) as i64,
                        role: "manifest".to_string(),
                    });
                }
            }
        } else {
            let rows = sqlx::query_as::<_, (String, Option<String>, i64)>(
                "SELECT m.digest, m.media_type, m.size FROM manifest_children mc \
                 JOIN manifests m ON m.id = mc.child_manifest_id \
                 WHERE mc.parent_manifest_id = ? ORDER BY m.digest",
            )
            .bind(manifest.id)
            .fetch_all(&state.db)
            .await?;
            for (digest, media_type, size) in rows {
                references.push(LayerReference {
                    digest,
                    media_type,
                    size,
                    role: "manifest".to_string(),
                });
            }
        }
    } else if let Some(value) = parsed.as_ref() {
        for (field, role) in [("config", "config"), ("layers", "layer")] {
            match value.get(field) {
                Some(Value::Array(entries)) => {
                    for entry in entries {
                        if let Some(digest) = entry.get("digest").and_then(Value::as_str) {
                            references.push(LayerReference {
                                digest: digest.to_string(),
                                media_type: entry
                                    .get("mediaType")
                                    .and_then(Value::as_str)
                                    .map(str::to_string),
                                size: entry.get("size").and_then(Value::as_u64).unwrap_or(0)
                                    as i64,
                                role: role.to_string(),
                            });
                        }
                    }
                }
                Some(entry) if field == "config" => {
                    if let Some(digest) = entry.get("digest").and_then(Value::as_str) {
                        references.push(LayerReference {
                            digest: digest.to_string(),
                            media_type: entry
                                .get("mediaType")
                                .and_then(Value::as_str)
                                .map(str::to_string),
                            size: entry.get("size").and_then(Value::as_u64).unwrap_or(0) as i64,
                            role: "config".to_string(),
                        });
                    }
                }
                _ => {}
            }
        }
    } else {
        let rows = sqlx::query_as::<_, (String, Option<String>, i64, String)>(
            "SELECT b.digest, b.media_type, b.size, mb.role FROM manifest_blobs mb \
             JOIN blobs b ON b.id = mb.blob_id WHERE mb.manifest_id = ? ORDER BY mb.role, b.digest",
        )
        .bind(manifest.id)
        .fetch_all(&state.db)
        .await?;
        for (digest, media_type, size, role) in rows {
            references.push(LayerReference {
                digest,
                media_type,
                size,
                role,
            });
        }
    }

    Ok(Json(references).into_response())
}

async fn layer_blob(
    state: &AppState,
    actor: &AuthContext,
    repo_name: &str,
    digest: &str,
) -> ApiResult<(crate::models::Repository, Blob, Digest)> {
    let repository = visible_repository(state, actor, repo_name).await?;
    let parsed = Digest::parse(digest).map_err(|_| not_found("layer not found"))?;
    let Some(blob) = state.registry.blob_stat(&parsed).await.map_err(ApiError::from)? else {
        return Err(not_found("layer not found"));
    };
    if !state
        .registry
        .blob_in_repository(repository.id, &parsed)
        .await
        .map_err(ApiError::from)?
    {
        return Err(not_found("layer not found"));
    }
    Ok((repository, blob, parsed))
}

pub async fn layer_tree(
    state: AppState,
    actor: AuthContext,
    repo_name: String,
    digest: String,
    path: Option<String>,
) -> ApiResult<Response> {
    let (_repository, blob, parsed) = layer_blob(&state, &actor, &repo_name, &digest).await?;
    let path = sanitize_request_path(path.as_deref().unwrap_or_default())?;
    let file = open_blob(&state, &parsed).await?;
    let std_file = file.into_std().await;
    let media_type = blob.media_type.clone();

    let entries = run_blocking(move || {
        let reader = decompressed(std_file, media_type.as_deref())?;
        let merged = merge_entries(reader)?;
        list_level(&merged, &path)
    })
    .await?;

    Ok(Json(entries).into_response())
}

pub async fn layer_file(
    state: AppState,
    actor: AuthContext,
    repo_name: String,
    digest: String,
    path: Option<String>,
) -> ApiResult<Response> {
    let Some(raw_path) = path.as_deref().map(str::trim).filter(|value| !value.is_empty()) else {
        return Err(ApiError::bad_request("path is required"));
    };
    let target = sanitize_request_path(raw_path)?;
    if target.is_empty() {
        return Err(ApiError::bad_request("path is required"));
    }

    let (_repository, blob, parsed) = layer_blob(&state, &actor, &repo_name, &digest).await?;
    let file = open_blob(&state, &parsed).await?;
    let std_file = file.into_std().await;
    let media_type = blob.media_type.clone();

    let (content, content_type) = run_blocking(move || {
        let reader = decompressed(std_file, media_type.as_deref())?;
        find_file(reader, &target)
    })
    .await?;

    Ok(bytes_response(StatusCode::OK, content, &content_type))
}

pub async fn layer_download(
    state: AppState,
    actor: AuthContext,
    repo_name: String,
    digest: String,
) -> ApiResult<Response> {
    let (_repository, blob, parsed) = layer_blob(&state, &actor, &repo_name, &digest).await?;
    let file = open_blob(&state, &parsed).await?;
    let content_type = blob
        .media_type
        .clone()
        .unwrap_or_else(|| "application/octet-stream".to_string());
    Ok(stream_blob(file, blob.size.max(0) as u64, &content_type))
}
