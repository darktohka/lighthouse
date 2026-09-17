//! Blob download and deletion handlers (`/v2/<name>/blobs/<digest>`).

use axum::body::Body;
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::Response;
use tokio::io::{AsyncReadExt, AsyncSeekExt, SeekFrom};
use tokio_util::io::ReaderStream;

use crate::error::RegistryError;
use crate::oci::digest::Digest;
use crate::permissions::Action;
use crate::state::{AppState, AuthContext};

use super::EventInfo;

pub async fn get(
    state: &AppState,
    actor: &AuthContext,
    headers: &HeaderMap,
    info: &EventInfo,
    name: &str,
    digest_raw: &str,
    head_only: bool,
) -> Result<Response, RegistryError> {
    super::validate_name(name)?;
    let digest = Digest::parse(digest_raw)?;
    let repository = super::find_repo(state, actor, name, Action::Pull).await?;

    let Some(blob) = state.registry.blob_stat(&digest).await? else {
        return Err(RegistryError::blob_unknown(&digest.to_string()));
    };
    if !state
        .registry
        .blob_in_repository(repository.id, &digest)
        .await?
    {
        return Err(RegistryError::blob_unknown(&digest.to_string()));
    }

    let size = blob.size.max(0) as u64;
    let media_type = blob
        .media_type
        .clone()
        .unwrap_or_else(|| "application/octet-stream".to_string());
    let digest_text = digest.to_string();
    let etag = format!("\"{digest_text}\"");

    if let Some(value) = headers
        .get(header::IF_NONE_MATCH)
        .and_then(|value| value.to_str().ok())
    {
        let matches = value.split(',').any(|candidate| {
            let candidate = candidate.trim();
            candidate == "*"
                || candidate == etag
                || candidate.trim_start_matches("W/") == etag
        });
        if matches {
            let mut response = super::respond(StatusCode::NOT_MODIFIED, Body::empty());
            response = super::set_header(response, "docker-content-digest", &digest_text);
            response = super::set_header(response, "etag", &etag);
            return Ok(response);
        }
    }

    let range = headers
        .get(header::RANGE)
        .and_then(|value| value.to_str().ok())
        .map(|value| parse_range(value, size))
        .unwrap_or(RangeRequest::Full);

    super::log_pull(
        state,
        actor,
        info,
        "blob.pull",
        &repository,
        None,
        &digest_text,
        Some(&media_type),
        None,
    )
    .await;

    let (status, length, content_range) = match range {
        RangeRequest::Full => (StatusCode::OK, size, None),
        RangeRequest::Partial { start, end } => (
            StatusCode::PARTIAL_CONTENT,
            end - start + 1,
            Some(format!("bytes {start}-{end}/{size}")),
        ),
        RangeRequest::Unsatisfiable => {
            let mut response = super::respond(StatusCode::RANGE_NOT_SATISFIABLE, Body::empty());
            response = super::set_header(response, "content-range", format!("bytes */{size}"));
            response = super::set_header(response, "accept-ranges", "bytes");
            return Ok(response);
        }
    };

    let body = if head_only {
        Body::empty()
    } else {
        let file = state
            .storage
            .open_blob(&digest)
            .await
            .map_err(super::storage_failure)?
            .ok_or_else(|| RegistryError::blob_unknown(&digest_text))?;
        match range {
            RangeRequest::Partial { start, .. } => {
                let mut file = file;
                file.seek(SeekFrom::Start(start)).await?;
                Body::from_stream(ReaderStream::new(file.take(length)))
            }
            _ => Body::from_stream(ReaderStream::new(file)),
        }
    };

    let mut response = super::respond(status, body);
    response = super::set_header(response, "content-type", &media_type);
    response = super::set_header(response, "content-length", length.to_string());
    response = super::set_header(response, "docker-content-digest", &digest_text);
    response = super::set_header(response, "etag", &etag);
    response = super::set_header(response, "accept-ranges", "bytes");
    if let Some(content_range) = content_range {
        response = super::set_header(response, "content-range", content_range);
    }
    Ok(response)
}

pub async fn delete(
    state: &AppState,
    actor: &AuthContext,
    info: &EventInfo,
    name: &str,
    digest_raw: &str,
) -> Result<Response, RegistryError> {
    super::validate_name(name)?;
    let digest = Digest::parse(digest_raw)?;
    let repository = super::find_repo(state, actor, name, Action::Delete).await?;

    let Some(blob) = state.registry.blob_stat(&digest).await? else {
        return Err(RegistryError::blob_unknown(&digest.to_string()));
    };
    if !state
        .registry
        .blob_in_repository(repository.id, &digest)
        .await?
    {
        return Err(RegistryError::blob_unknown(&digest.to_string()));
    }

    sqlx::query("DELETE FROM blob_repositories WHERE blob_id = ? AND repository_id = ?")
        .bind(blob.id)
        .bind(repository.id)
        .execute(&state.db)
        .await?;

    let remaining_links: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM blob_repositories WHERE blob_id = ?")
            .bind(blob.id)
            .fetch_one(&state.db)
            .await?;
    let manifest_references: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM manifest_blobs WHERE blob_id = ?")
            .bind(blob.id)
            .fetch_one(&state.db)
            .await?;

    if remaining_links == 0 && manifest_references == 0 {
        sqlx::query("DELETE FROM blobs WHERE id = ?")
            .bind(blob.id)
            .execute(&state.db)
            .await?;
        state
            .storage
            .delete_blob(&digest)
            .await
            .map_err(super::storage_failure)?;
        state.layer_cache.invalidate(&digest);
    }

    let digest_text = digest.to_string();
    super::log_registry_event(
        &state.db,
        info,
        actor,
        "blob.delete",
        Some(&repository),
        Some(&digest_text),
        Some(&digest_text),
        None,
        202,
    )
    .await;

    Ok(super::respond(StatusCode::ACCEPTED, Body::empty()))
}

#[derive(Clone, Copy)]
enum RangeRequest {
    Full,
    Partial { start: u64, end: u64 },
    Unsatisfiable,
}

fn parse_range(value: &str, size: u64) -> RangeRequest {
    let Some(spec) = value.strip_prefix("bytes=") else {
        return RangeRequest::Full;
    };
    let Some(raw) = spec.split(',').next() else {
        return RangeRequest::Full;
    };
    let raw = raw.trim();
    let Some((start_raw, end_raw)) = raw.split_once('-') else {
        return RangeRequest::Full;
    };

    if start_raw.trim().is_empty() {
        let Ok(suffix_len) = end_raw.trim().parse::<u64>() else {
            return RangeRequest::Full;
        };
        if suffix_len == 0 || size == 0 {
            return RangeRequest::Unsatisfiable;
        }
        return RangeRequest::Partial {
            start: size.saturating_sub(suffix_len),
            end: size - 1,
        };
    }

    let Ok(start) = start_raw.trim().parse::<u64>() else {
        return RangeRequest::Full;
    };
    let end = if end_raw.trim().is_empty() {
        if size == 0 {
            return RangeRequest::Unsatisfiable;
        }
        size - 1
    } else {
        match end_raw.trim().parse::<u64>() {
            Ok(end) => end,
            Err(_) => return RangeRequest::Full,
        }
    };

    if start > end {
        return RangeRequest::Full;
    }
    if size == 0 || start >= size {
        return RangeRequest::Unsatisfiable;
    }
    RangeRequest::Partial {
        start,
        end: end.min(size - 1),
    }
}
