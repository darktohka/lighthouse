//! Manifest / config JSON and the layer filesystem browser.
//!
//! Layer archives are decompressed transparently by sniffing the blob's leading
//! magic bytes (gzip, zstd, otherwise a plain tar); the stored media type is
//! client metadata and is not trusted for decompression. Nothing is ever
//! extracted to disk: the tar
//! is read as an in-memory stream in a blocking task, with strict caps on the
//! number of entries, decompressed bytes and the size of a single browsed file.
//! Requested paths are sanitized (no absolute paths, no `..`, no NUL, no
//! symlink traversal) before any lookup. See `docs/LAYER_BROWSER.md`.
//!
//! A full scan is expensive, so each layer's merged path map is memoized in
//! [`crate::layer_cache`]; only the first `tree` request for a digest pays the
//! decompression cost.

use std::collections::{HashMap, HashSet};
use std::io::{self, Read, Seek, SeekFrom};
use std::sync::Arc;

use axum::Json;
use axum::Router;
use axum::body::Body;
use axum::extract::{Path, State};
use axum::http::{HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use bytes::Bytes;
use serde::Serialize;
use serde_json::Value;
use tar::EntryType;

use crate::auth::middleware::Auth;
use crate::config::effective_layer_scan_bytes;
use crate::error::{ApiError, ApiResult};
use crate::layer_cache::{
    CachedEntry, Change, Changes, ComposedKey, ComposedLayer, EntryKind, LayerIndex, ListingKey,
    child_dir_sizes,
};
use crate::models::Blob;
use crate::oci::digest::Digest;
use crate::oci::media_types;
use crate::oci::reference;
use crate::permissions;
use crate::state::{AppState, AuthContext};
use tokio_util::io::ReaderStream;

use super::repositories::blob_json as config_blob_json;
use super::visible_repository;

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

/// A `200 application/json` response built from already-serialized bytes,
/// without copying them. Byte-identical to what `Json(entries)` would produce.
fn json_bytes_response(body: Bytes) -> Response {
    let mut response = Response::new(Body::from(body));
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/json"),
    );
    response
}

/// Serializes a listing level once, for caching and for the response body.
fn listing_body(entries: &[LayerTreeEntry]) -> ApiResult<Bytes> {
    serde_json::to_vec(entries)
        .map(Bytes::from)
        .map_err(|_| ApiError::internal("layer listing serialization failed"))
}

fn apply_headers(
    mut response: Response,
    content_type: &str,
    content_length: Option<u64>,
) -> Response {
    let value = HeaderValue::from_str(content_type)
        .unwrap_or_else(|_| HeaderValue::from_static("application/octet-stream"));
    response.headers_mut().insert(header::CONTENT_TYPE, value);
    if let Some(length) =
        content_length.and_then(|length| HeaderValue::from_str(&length.to_string()).ok())
    {
        response
            .headers_mut()
            .insert(header::CONTENT_LENGTH, length);
    }
    response
}

fn stream_blob(file: tokio::fs::File, size: u64, content_type: &str) -> Response {
    let stream = ReaderStream::new(file);
    let mut response = Response::new(Body::from_stream(stream));
    *response.status_mut() = StatusCode::OK;
    apply_headers(response, content_type, Some(size))
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

/// Selects the decompressor from the archive's leading magic bytes rather than
/// its media type: the media type is untrusted client metadata, while the blob
/// content is content-addressed and therefore authoritative. The file is
/// rewound afterwards because both gzip and zstd decoders read from the current
/// position.
fn compression_for(file: &mut std::fs::File) -> io::Result<Compression> {
    let mut magic = [0u8; 4];
    let mut filled = 0usize;
    while filled < magic.len() {
        match file.read(&mut magic[filled..])? {
            0 => break,
            read => filled += read,
        }
    }
    file.seek(SeekFrom::Start(0))?;
    Ok(classify_magic(&magic[..filled]))
}

fn classify_magic(magic: &[u8]) -> Compression {
    if magic.starts_with(&[0x1F, 0x8B]) {
        Compression::Gzip
    } else if magic.starts_with(&[0x28, 0xB5, 0x2F, 0xFD]) {
        Compression::Zstd
    } else if magic.len() >= 4
        && (0x50..=0x5F).contains(&magic[0])
        && magic[1] == 0x2A
        && magic[2] == 0x4D
        && magic[3] == 0x18
    {
        // zstd skippable frame; the decoder skips it and reads the real frame.
        Compression::Zstd
    } else {
        Compression::Plain
    }
}

/// The download filename extension implied by a blob's media type.
fn download_extension(media_type: Option<&str>) -> &'static str {
    let media_type = media_type.unwrap_or("").to_ascii_lowercase();
    if !media_type.contains("tar") {
        return "bin";
    }
    if media_type.contains("zstd") {
        "tar.zst"
    } else if media_type.contains("gzip") {
        "tar.gz"
    } else {
        "tar"
    }
}

/// A stable, filesystem-safe download name for a blob: its digest plus an
/// extension derived from the media type.
fn download_filename(digest: &Digest, media_type: Option<&str>) -> String {
    format!(
        "{}-{}.{}",
        digest.algorithm(),
        digest.encoded(),
        download_extension(media_type)
    )
}

fn decompressed(mut file: std::fs::File) -> ApiResult<Box<dyn Read + Send>> {
    match compression_for(&mut file).map_err(map_io)? {
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

fn entry_kind(entry_type: EntryType) -> EntryKind {
    if entry_type.is_dir() {
        EntryKind::Dir
    } else if entry_type.is_symlink() {
        EntryKind::Symlink
    } else {
        EntryKind::File
    }
}

/// Reads the archive through the entry stream, applying the whiteout and
/// repeated-path rules, and returns the merged path map plus the subtree roots
/// this layer removes.
fn merge_entries(
    reader: Box<dyn Read + Send>,
    max_scan_bytes: u64,
    max_entries: u64,
) -> ApiResult<(HashMap<String, CachedEntry>, HashSet<String>)> {
    let mut archive = tar::Archive::new(CappedReader {
        inner: reader,
        remaining: max_scan_bytes,
    });
    let mut merged: HashMap<String, CachedEntry> = HashMap::new();
    let mut deletes: HashSet<String> = HashSet::new();
    let mut pending_long_name: Option<Vec<u8>> = None;
    let mut count = 0u64;

    for entry in archive.entries().map_err(map_io)? {
        let mut entry = entry.map_err(map_io)?;
        count += 1;
        if count > max_entries {
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
            deletes.insert(directory.clone());
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
            deletes.insert(target.clone());
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
                .map(|target| target.to_string_lossy().into_owned().into_boxed_str())
        } else {
            None
        };
        merged.insert(
            path,
            CachedEntry {
                kind: entry_kind(entry_type),
                size,
                mode,
                link_target,
            },
        );
    }

    Ok((merged, deletes))
}

#[derive(Serialize)]
struct LayerTreeEntry {
    name: String,
    path: String,
    kind: String,
    /// Bytes for a file/symlink; recursive total of everything below it for a
    /// directory.
    size: i64,
    mode: u32,
    link_target: Option<String>,
    /// For a symlink: the normalized layer path it points at, when resolvable.
    link_resolved: Option<String>,
    /// For a symlink: the resolved entry's kind (`file`/`dir`); `None` when the
    /// link is dangling or cyclic.
    link_kind: Option<String>,
    /// `"new"`, `"modified"` or `"removed"` in a diff mode; `null` otherwise
    /// (including `single` and plain `aggregate`).
    change: Option<String>,
    /// Digest of the layer supplying the entry, for the aggregate modes; the
    /// frontend previews the file through `/layers/{source_digest}/file`.
    source_digest: Option<String>,
}

fn list_level(index: &LayerIndex, path: &str) -> ApiResult<Vec<LayerTreeEntry>> {
    let prefix = if path.is_empty() {
        String::new()
    } else {
        format!("{path}/")
    };
    let mut children: HashMap<String, CachedEntry> = HashMap::new();
    for (entry_path, data) in index.entries() {
        if entry_path.as_str() == path {
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
                children
                    .entry(head.to_string())
                    .or_insert_with(CachedEntry::directory);
            }
            None => {
                children.insert(rest.to_string(), data.clone());
            }
        }
    }
    let dir_sizes = child_dir_sizes(
        index
            .entries()
            .iter()
            .map(|(entry_path, entry)| (entry_path.as_str(), entry)),
        path,
    );

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
            let size = match data.kind {
                EntryKind::Dir => dir_sizes.get(&name).copied().unwrap_or(0),
                _ => data.size,
            };
            let (link_resolved, link_kind) = match data.kind {
                EntryKind::Symlink => {
                    match data
                        .link_target
                        .as_deref()
                        .and_then(|target| index.resolve_link(&full_path, target))
                    {
                        Some((resolved, kind)) => (Some(resolved), Some(kind.as_str().to_string())),
                        None => (None, None),
                    }
                }
                _ => (None, None),
            };
            Some(LayerTreeEntry {
                name,
                path: full_path,
                kind: data.kind.as_str().to_string(),
                size,
                mode: data.mode,
                link_target: data.link_target.as_deref().map(str::to_string),
                link_resolved,
                link_kind,
                change: None,
                source_digest: None,
            })
        })
        .collect())
}

fn find_file(
    reader: Box<dyn Read + Send>,
    target: &str,
    max_scan_bytes: u64,
    max_entries: u64,
) -> ApiResult<(Vec<u8>, String)> {
    let mut archive = tar::Archive::new(CappedReader {
        inner: reader,
        remaining: max_scan_bytes,
    });
    let mut pending_long_name: Option<Vec<u8>> = None;
    let mut count = 0u64;

    for entry in archive.entries().map_err(map_io)? {
        let mut entry = entry.map_err(map_io)?;
        count += 1;
        if count > max_entries {
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
    tokio::task::spawn_blocking(job).await.map_err(|err| {
        tracing::error!(error = %err, "layer task panicked");
        ApiError::internal("layer reader failed")
    })?
}

// ---------------------------------------------------------------------------
// Blob resolution
// ---------------------------------------------------------------------------

async fn visible_blob(state: &AppState, actor: &AuthContext, digest: &str) -> ApiResult<Blob> {
    let parsed = Digest::parse(digest).map_err(|_| ApiError::not_found("blob not found"))?;
    let Some(blob) = state
        .registry
        .blob_stat(&parsed)
        .await
        .map_err(ApiError::from)?
    else {
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
    Ok(stream_blob(
        file,
        blob.size.max(0) as u64,
        "application/octet-stream",
    ))
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
    created: Option<String>,
    created_by: Option<String>,
    comment: Option<String>,
}

/// Attaches the config's `history` to the `layer`-role references, in order.
/// Config and manifest references keep all three fields `None`.
fn attach_layer_history(references: &mut [LayerReference], config: Option<&Value>) {
    let history = super::layer_history(config);
    for (reference, entry) in references
        .iter_mut()
        .filter(|reference| reference.role == "layer")
        .zip(history)
    {
        reference.created = entry.created;
        reference.created_by = entry.created_by;
        reference.comment = entry.comment;
    }
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
                        created: None,
                        created_by: None,
                        comment: None,
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
                    created: None,
                    created_by: None,
                    comment: None,
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
                                size: entry.get("size").and_then(Value::as_u64).unwrap_or(0) as i64,
                                role: role.to_string(),
                                created: None,
                                created_by: None,
                                comment: None,
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
                            created: None,
                            created_by: None,
                            comment: None,
                        });
                    }
                }
                _ => {}
            }
        }
        let history_config = match value
            .get("config")
            .and_then(|config| config.get("digest"))
            .and_then(Value::as_str)
        {
            Some(digest) => config_blob_json(&state, digest).await,
            None => None,
        };
        attach_layer_history(&mut references, history_config.as_ref());
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
                created: None,
                created_by: None,
                comment: None,
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
    let Some(blob) = state
        .registry
        .blob_stat(&parsed)
        .await
        .map_err(ApiError::from)?
    else {
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

/// Query parameters accepted by the layer `tree` endpoint.
pub struct TreeQuery {
    pub path: Option<String>,
    pub mode: Option<String>,
    pub manifest: Option<String>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum TreeMode {
    Single,
    Aggregate,
    Diff,
    AggregateDiff,
}

impl TreeMode {
    fn parse(raw: Option<&str>) -> ApiResult<Self> {
        match raw.map(str::trim).filter(|value| !value.is_empty()) {
            None | Some("single") => Ok(TreeMode::Single),
            Some("aggregate") => Ok(TreeMode::Aggregate),
            Some("diff") => Ok(TreeMode::Diff),
            Some("aggregate-diff") => Ok(TreeMode::AggregateDiff),
            Some(_) => Err(ApiError::bad_request(
                "mode must be `single`, `aggregate`, `diff` or `aggregate-diff`",
            )),
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            TreeMode::Single => "single",
            TreeMode::Aggregate => "aggregate",
            TreeMode::Diff => "diff",
            TreeMode::AggregateDiff => "aggregate-diff",
        }
    }

    fn colors(self) -> bool {
        matches!(self, TreeMode::Diff | TreeMode::AggregateDiff)
    }

    fn shows_ghosts(self) -> bool {
        matches!(self, TreeMode::Diff | TreeMode::AggregateDiff)
    }

    fn filters_unchanged(self) -> bool {
        matches!(self, TreeMode::Diff)
    }
}

/// Builds (or returns the cached) merged index for one layer blob.
async fn build_layer_index(state: &AppState, digest: &Digest) -> ApiResult<Arc<LayerIndex>> {
    if let Some(index) = state.layer_cache.get(digest) {
        return Ok(index);
    }
    let file = open_blob(state, digest).await?;
    let std_file = file.into_std().await;
    let max_scan_bytes = effective_layer_scan_bytes(state.config.layer_max_scan_bytes);
    let max_entries = state.config.layer_max_entries;
    let digest = digest.clone();
    state
        .layer_cache
        .get_or_build(&digest, move || {
            let reader = decompressed(std_file)?;
            let (merged, deletes) = merge_entries(reader, max_scan_bytes, max_entries)?;
            Ok(Arc::new(LayerIndex::with_deletes(merged, deletes)))
        })
        .await
}

fn child_path(path: &str, name: &str) -> String {
    if path.is_empty() {
        name.to_string()
    } else {
        format!("{path}/{name}")
    }
}

/// The ordered layer digests declared by an image manifest body.
fn manifest_layer_digests(content: &[u8]) -> Vec<Digest> {
    let Ok(value) = serde_json::from_slice::<Value>(content) else {
        return Vec::new();
    };
    value
        .get("layers")
        .and_then(Value::as_array)
        .map(|layers| {
            layers
                .iter()
                .filter_map(|entry| entry.get("digest").and_then(Value::as_str))
                .filter_map(|digest| Digest::parse(digest).ok())
                .collect()
        })
        .unwrap_or_default()
}

/// The child manifest digests of an index, preferring the descriptor array in
/// the body and falling back to the recorded graph edges.
async fn index_children(
    state: &AppState,
    manifest: &crate::models::Manifest,
) -> ApiResult<Vec<Digest>> {
    if let Ok(value) = serde_json::from_slice::<Value>(&manifest.content) {
        if let Some(entries) = value.get("manifests").and_then(Value::as_array) {
            let digests: Vec<Digest> = entries
                .iter()
                .filter_map(|entry| entry.get("digest").and_then(Value::as_str))
                .filter_map(|digest| Digest::parse(digest).ok())
                .collect();
            if !digests.is_empty() {
                return Ok(digests);
            }
        }
    }
    let rows = sqlx::query_scalar::<_, String>(
        "SELECT m.digest FROM manifest_children mc \
         JOIN manifests m ON m.id = mc.child_manifest_id \
         WHERE mc.parent_manifest_id = ? ORDER BY m.digest",
    )
    .bind(manifest.id)
    .fetch_all(&state.db)
    .await?;
    let mut digests = Vec::new();
    for raw in rows {
        if let Ok(digest) = Digest::parse(&raw) {
            digests.push(digest);
        }
    }
    Ok(digests)
}

/// Resolves the ordered layer digests that place `target` within a manifest.
///
/// An image manifest owns one ordering. An index does not, so each child image
/// manifest is inspected and the one whose layer list contains `target` wins:
/// exactly one match is usable, several are ambiguous (`400`), and none is a
/// missing layer (`404`).
async fn resolve_ordered_layers(
    state: &AppState,
    manifest: &crate::models::Manifest,
    target: &Digest,
) -> ApiResult<Vec<Digest>> {
    if !media_types::is_index_type(&manifest.media_type) {
        let layers = manifest_layer_digests(&manifest.content);
        if !layers.contains(target) {
            return Err(not_found("layer not found in manifest"));
        }
        return Ok(layers);
    }

    let mut matches = Vec::new();
    for child_digest in index_children(state, manifest).await? {
        let Some(child) = state
            .registry
            .manifest(&child_digest)
            .await
            .map_err(ApiError::from)?
        else {
            continue;
        };
        let layers = manifest_layer_digests(&child.content);
        if layers.contains(target) {
            matches.push(layers);
        }
    }
    match matches.len() {
        0 => Err(not_found("layer not found in manifest")),
        1 => Ok(matches.pop().expect("exactly one match")),
        _ => Err(ApiError::bad_request(
            "layer is referenced by several manifests in the index; pass a single image manifest",
        )),
    }
}

/// Returns the cumulative overlay of `layers[0..=position]`, building and
/// caching each intermediate step.
async fn cumulative_overlay(
    state: &AppState,
    manifest_digest: &Digest,
    layers: &Arc<Vec<Digest>>,
    position: usize,
) -> ApiResult<Arc<ComposedLayer>> {
    let target: ComposedKey = (manifest_digest.clone(), position);
    if let Some(hit) = state.composed_cache.get(&target) {
        return Ok(hit);
    }
    let mut current: Arc<ComposedLayer> = Arc::new(ComposedLayer::empty());
    for (index, layer) in layers.iter().enumerate().take(position + 1) {
        let key: ComposedKey = (manifest_digest.clone(), index);
        if let Some(hit) = state.composed_cache.get(&key) {
            current = hit;
            continue;
        }
        let upper = build_layer_index(state, layer).await?;
        let base = Arc::clone(&current);
        let composed_layers = Arc::clone(layers);
        let position = index as u32;
        current = state
            .composed_cache
            .get_or_build(&key, move || {
                Ok(Arc::new(ComposedLayer::compose(
                    &base,
                    &upper,
                    position,
                    composed_layers,
                )))
            })
            .await?;
    }
    Ok(current)
}

struct Child {
    entry: CachedEntry,
    source: Option<u32>,
    removed: bool,
}

fn list_level_composed(
    final_layer: &ComposedLayer,
    lower_layer: &ComposedLayer,
    path: &str,
    changes: Option<&Changes>,
    mode: TreeMode,
) -> ApiResult<Vec<LayerTreeEntry>> {
    let prefix = if path.is_empty() {
        String::new()
    } else {
        format!("{path}/")
    };
    let mut children: HashMap<String, Child> = HashMap::new();

    for (entry_path, node) in final_layer.iter() {
        if entry_path == path {
            continue;
        }
        let rest = if path.is_empty() {
            entry_path
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
                children.entry(head.to_string()).or_insert_with(|| Child {
                    entry: CachedEntry::directory(),
                    source: None,
                    removed: false,
                });
            }
            None => {
                children.insert(
                    rest.to_string(),
                    Child {
                        entry: node.entry.clone(),
                        source: node.source,
                        removed: false,
                    },
                );
            }
        }
    }
    let dir_sizes = child_dir_sizes(final_layer.entries(), path);

    let mut lower_dir_sizes: HashMap<String, i64> = HashMap::new();
    if mode.shows_ghosts() {
        // Every direct child name present in the lower overlay, whether it was
        // an explicit entry or a directory synthesized from a deeper path.
        let mut lower_heads: HashMap<String, Option<(CachedEntry, Option<u32>)>> = HashMap::new();
        for (entry_path, node) in lower_layer.iter() {
            if entry_path == path {
                continue;
            }
            let rest = if path.is_empty() {
                entry_path
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
                    lower_heads.entry(head.to_string()).or_insert(None);
                }
                None => {
                    lower_heads.insert(rest.to_string(), Some((node.entry.clone(), node.source)));
                }
            }
        }
        lower_dir_sizes = child_dir_sizes(lower_layer.entries(), path);
        for (head, explicit) in lower_heads {
            if children.contains_key(&head) {
                continue;
            }
            let child = match explicit {
                Some((entry, source)) => Child {
                    entry,
                    source,
                    removed: true,
                },
                None => Child {
                    entry: CachedEntry::directory(),
                    source: None,
                    removed: true,
                },
            };
            children.entry(head).or_insert(child);
        }
    }

    let mut names: Vec<String> = children.keys().cloned().collect();
    names.sort();
    if names.len() > MAX_TREE_ENTRIES {
        return Err(too_large("directory has too many entries"));
    }

    let mut entries = Vec::new();
    for name in names {
        let Some(child) = children.get(&name) else {
            continue;
        };
        let full_path = child_path(path, &name);
        let change = if child.removed {
            Some(Change::Removed)
        } else if mode.colors() {
            changes.and_then(|changes| changes.get(&full_path))
        } else {
            None
        };
        if mode.filters_unchanged() && change.is_none() {
            continue;
        }
        let size = match child.entry.kind {
            EntryKind::Dir => {
                if child.removed {
                    lower_dir_sizes.get(&name).copied().unwrap_or(0)
                } else {
                    dir_sizes.get(&name).copied().unwrap_or(0)
                }
            }
            _ => child.entry.size,
        };
        let (link_resolved, link_kind) = match child.entry.kind {
            EntryKind::Symlink => {
                let resolver = if child.removed {
                    lower_layer
                } else {
                    final_layer
                };
                match child
                    .entry
                    .link_target
                    .as_deref()
                    .and_then(|target| resolver.resolve_link(&full_path, target))
                {
                    Some((resolved, kind)) => (Some(resolved), Some(kind.as_str().to_string())),
                    None => (None, None),
                }
            }
            _ => (None, None),
        };
        let source_digest = child
            .source
            .and_then(|index| {
                if child.removed {
                    lower_layer.source_at(index)
                } else {
                    final_layer.source_at(index)
                }
            })
            .map(ToString::to_string);
        entries.push(LayerTreeEntry {
            name,
            path: full_path,
            kind: child.entry.kind.as_str().to_string(),
            size,
            mode: child.entry.mode,
            link_target: child.entry.link_target.as_deref().map(str::to_string),
            link_resolved,
            link_kind,
            change: change.map(|change| change.as_str().to_string()),
            source_digest,
        });
    }
    Ok(entries)
}

pub async fn layer_tree(
    state: AppState,
    actor: AuthContext,
    repo_name: String,
    digest: String,
    query: TreeQuery,
) -> ApiResult<Response> {
    let mode = TreeMode::parse(query.mode.as_deref())?;
    let path = sanitize_request_path(query.path.as_deref().unwrap_or_default())?;
    let (_repository, _blob, parsed) = layer_blob(&state, &actor, &repo_name, &digest).await?;

    if mode == TreeMode::Single {
        let key = ListingKey {
            scope: parsed.to_string(),
            position: u32::MAX,
            path: path.clone(),
            mode: mode.as_str(),
        };
        if let Some(body) = state.listing_cache.get(&key) {
            return Ok(json_bytes_response(body));
        }
        let generation = state.listing_cache.generation_token();
        let index = build_layer_index(&state, &parsed).await?;
        let entries = run_blocking(move || list_level(&index, &path)).await?;
        let body = listing_body(&entries)?;
        state
            .listing_cache
            .insert_with_generation(key, body.clone(), generation);
        return Ok(json_bytes_response(body));
    }

    let manifest_ref = query
        .manifest
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| ApiError::bad_request("manifest is required for this mode"))?;
    let (repository, manifest) = resolve_manifest(&state, &actor, &repo_name, manifest_ref).await?;
    let layers = Arc::new(resolve_ordered_layers(&state, &manifest, &parsed).await?);
    let position = layers
        .iter()
        .position(|layer| layer == &parsed)
        .ok_or_else(|| not_found("layer not found in manifest"))?;

    for layer in &layers[..=position] {
        if !state
            .registry
            .blob_in_repository(repository.id, layer)
            .await
            .map_err(ApiError::from)?
        {
            return Err(not_found("layer not found in repository"));
        }
    }

    let manifest_digest = Digest::parse(&manifest.digest).map_err(ApiError::from)?;
    let key = ListingKey {
        scope: manifest_digest.to_string(),
        position: position as u32,
        path: path.clone(),
        mode: mode.as_str(),
    };
    if let Some(body) = state.listing_cache.get(&key) {
        return Ok(json_bytes_response(body));
    }
    let generation = state.listing_cache.generation_token();

    let final_layer = cumulative_overlay(&state, &manifest_digest, &layers, position).await?;
    let mut changes: Option<Arc<Changes>> = None;
    let lower_layer = if mode.shows_ghosts() {
        let lower = match position.checked_sub(1) {
            Some(lower) => cumulative_overlay(&state, &manifest_digest, &layers, lower).await?,
            None => Arc::new(ComposedLayer::empty()),
        };
        let change_key: ComposedKey = (manifest_digest.clone(), position);
        changes = Some(
            state
                .changes_cache
                .get_or_build(&change_key, {
                    let final_layer = Arc::clone(&final_layer);
                    let lower = Arc::clone(&lower);
                    move || Ok(Arc::new(crate::layer_cache::classify(&final_layer, &lower)))
                })
                .await?,
        );
        lower
    } else {
        Arc::new(ComposedLayer::empty())
    };

    let entries = run_blocking(move || {
        list_level_composed(&final_layer, &lower_layer, &path, changes.as_deref(), mode)
    })
    .await?;

    let body = listing_body(&entries)?;
    state
        .listing_cache
        .insert_with_generation(key, body.clone(), generation);
    Ok(json_bytes_response(body))
}

pub async fn layer_file(
    state: AppState,
    actor: AuthContext,
    repo_name: String,
    digest: String,
    path: Option<String>,
) -> ApiResult<Response> {
    let Some(raw_path) = path
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    else {
        return Err(ApiError::bad_request("path is required"));
    };
    let target = sanitize_request_path(raw_path)?;
    if target.is_empty() {
        return Err(ApiError::bad_request("path is required"));
    }

    let (_repository, _blob, parsed) = layer_blob(&state, &actor, &repo_name, &digest).await?;
    let file = open_blob(&state, &parsed).await?;
    let std_file = file.into_std().await;
    let max_scan_bytes = effective_layer_scan_bytes(state.config.layer_max_scan_bytes);
    let max_entries = state.config.layer_max_entries;

    let (content, content_type) = run_blocking(move || {
        let reader = decompressed(std_file)?;
        find_file(reader, &target, max_scan_bytes, max_entries)
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
    let mut response = stream_blob(file, blob.size.max(0) as u64, &content_type);
    let filename = download_filename(&parsed, blob.media_type.as_deref());
    if let Ok(value) = HeaderValue::from_str(&format!("attachment; filename=\"{filename}\"")) {
        response
            .headers_mut()
            .insert(header::CONTENT_DISPOSITION, value);
    }
    Ok(response)
}
