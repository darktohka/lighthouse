//! Blob upload lifecycle handlers (`/v2/<name>/blobs/uploads/`).

use axum::body::Body;
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::Response;
use tokio::io::AsyncReadExt;

use crate::error::{ErrorCode, RegistryError};
use crate::oci::digest::{Digest, Verifier};
use crate::permissions::Action;
use crate::state::{AppState, AuthContext};

use super::EventInfo;

pub async fn start(
    state: &AppState,
    actor: &AuthContext,
    _headers: &HeaderMap,
    info: &EventInfo,
    name: &str,
    query: &[(String, String)],
    body: &[u8],
) -> Result<Response, RegistryError> {
    super::validate_name(name)?;
    let repository = super::ensure_repo_for_push(state, actor, name).await?;
    let uploads = super::uploads_service(state);

    if let Some(mount) = super::query_first(query, "mount") {
        let digest = Digest::parse(mount)?;
        if let Some(from) = super::query_first(query, "from") {
            super::validate_name(from)?;
            super::ensure_authorized(state, actor, from, Action::Pull).await?;
            if let Some(source) = state.registry.find_repository(from).await?
                && state
                    .registry
                    .mount_blob(source.id, repository.id, &digest)
                    .await?
            {
                super::log_registry_event(
                    &state.db,
                    info,
                    actor,
                    "blob.mount",
                    Some(&repository),
                    Some(from),
                    Some(&digest.to_string()),
                    None,
                    201,
                )
                .await;
                return Ok(blob_created(name, &digest));
            }
        }
    }

    if let Some(raw) = super::query_first(query, "digest") {
        let digest = Digest::parse(raw)?;
        if !digest.verify(body) {
            return Err(RegistryError::digest_invalid(&digest.to_string()));
        }
        super::check_blob_size(state, 0, body.len() as u64)?;
        let upload = uploads.create(repository.id, actor.user_id).await?;
        if uploads.append(&upload.uuid, 0, body).await.is_err() {
            let _ = uploads.cancel(&upload.uuid).await;
            return Err(RegistryError::upload_invalid("failed to stage uploaded content"));
        }
        return match uploads.complete(&upload.uuid, &digest).await {
            Ok(_) => {
                super::log_registry_event(
                    &state.db,
                    info,
                    actor,
                    "blob.push",
                    Some(&repository),
                    Some(&digest.to_string()),
                    Some(&digest.to_string()),
                    None,
                    201,
                )
                .await;
                Ok(blob_created(name, &digest))
            }
            Err(err) => {
                let _ = uploads.cancel(&upload.uuid).await;
                Err(err)
            }
        };
    }

    let upload = uploads.create(repository.id, actor.user_id).await?;
    super::log_registry_event(
        &state.db,
        info,
        actor,
        "blob.upload.start",
        Some(&repository),
        None,
        None,
        None,
        202,
    )
    .await;

    let mut response = super::respond(StatusCode::ACCEPTED, Body::empty());
    response = super::set_header(response, "location", upload_location(name, &upload.uuid));
    response = super::set_header(response, "range", "0-0");
    response = super::set_header(response, "docker-upload-uuid", &upload.uuid);
    response = super::set_header(response, "content-length", "0");
    Ok(response)
}

pub async fn status(
    state: &AppState,
    actor: &AuthContext,
    _headers: &HeaderMap,
    _info: &EventInfo,
    name: &str,
    uuid: &str,
) -> Result<Response, RegistryError> {
    super::validate_name(name)?;
    let repository = super::find_repo(state, actor, name, Action::Pull).await?;
    let uploads = super::uploads_service(state);
    let Some(upload) = uploads.status(uuid).await? else {
        return Err(RegistryError::upload_unknown(uuid));
    };
    if upload.repository_id != repository.id {
        return Err(RegistryError::upload_unknown(uuid));
    }

    let size = upload.offset.max(0) as u64;
    let end = if size == 0 { 0 } else { size - 1 };
    let mut response = super::respond(StatusCode::NO_CONTENT, Body::empty());
    response = super::set_header(response, "location", upload_location(name, uuid));
    response = super::set_header(response, "range", format!("0-{end}"));
    response = super::set_header(response, "docker-upload-uuid", uuid);
    response = super::set_header(response, "content-length", "0");
    Ok(response)
}

pub async fn patch(
    state: &AppState,
    actor: &AuthContext,
    headers: &HeaderMap,
    info: &EventInfo,
    name: &str,
    uuid: &str,
    body: &[u8],
) -> Result<Response, RegistryError> {
    super::validate_name(name)?;
    let repository = super::find_repo(state, actor, name, Action::Push).await?;
    let uploads = super::uploads_service(state);
    let Some(upload) = uploads.status(uuid).await? else {
        return Err(RegistryError::upload_unknown(uuid));
    };
    if upload.repository_id != repository.id {
        return Err(RegistryError::upload_unknown(uuid));
    }

    if let Some(content_type) = headers
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
    {
        let base = content_type.split(';').next().unwrap_or("").trim();
        if !base.eq_ignore_ascii_case("application/octet-stream") {
            return Err(RegistryError::upload_invalid("unsupported content type"));
        }
    }

    let offset = upload.offset.max(0) as u64;
    let declared = match headers
        .get(header::CONTENT_RANGE)
        .and_then(|value| value.to_str().ok())
    {
        Some(raw) => {
            let (start, end) = parse_content_range(raw)?;
            if start != offset {
                return Err(RegistryError::range_invalid(
                    "content range does not match the current upload size",
                ));
            }
            Some(end - start + 1)
        }
        None => None,
    };

    if let Some(raw) = headers
        .get(header::CONTENT_LENGTH)
        .and_then(|value| value.to_str().ok())
    {
        let expected: u64 = raw
            .trim()
            .parse()
            .map_err(|_| RegistryError::size_invalid("invalid content length"))?;
        if expected != body.len() as u64 {
            return Err(RegistryError::size_invalid(
                "content length does not match the request body",
            ));
        }
        if let Some(declared) = declared
            && expected != declared
        {
            return Err(RegistryError::size_invalid(
                "content length does not match the content range",
            ));
        }
    }
    if let Some(declared) = declared
        && declared != body.len() as u64
    {
        return Err(RegistryError::size_invalid(
            "content range does not match the request body",
        ));
    }

    super::check_blob_size(state, offset, body.len() as u64)?;
    let new_size = uploads.append(uuid, offset, body).await?;

    super::log_registry_event(
        &state.db,
        info,
        actor,
        "blob.upload.chunk",
        Some(&repository),
        None,
        None,
        None,
        202,
    )
    .await;

    let end = if new_size == 0 { 0 } else { new_size - 1 };
    let mut response = super::respond(StatusCode::ACCEPTED, Body::empty());
    response = super::set_header(response, "location", upload_location(name, uuid));
    response = super::set_header(response, "range", format!("0-{end}"));
    response = super::set_header(response, "docker-upload-uuid", uuid);
    response = super::set_header(response, "content-length", "0");
    Ok(response)
}

pub async fn complete(
    state: &AppState,
    actor: &AuthContext,
    headers: &HeaderMap,
    info: &EventInfo,
    name: &str,
    uuid: &str,
    query: &[(String, String)],
    body: &[u8],
) -> Result<Response, RegistryError> {
    super::validate_name(name)?;
    let repository = super::find_repo(state, actor, name, Action::Push).await?;
    let Some(raw) = super::query_first(query, "digest") else {
        return Err(RegistryError::new(
            ErrorCode::DigestInvalid,
            "missing `digest` query parameter",
        ));
    };
    let digest = Digest::parse(raw)?;
    let uploads = super::uploads_service(state);
    let Some(upload) = uploads.status(uuid).await? else {
        return Err(RegistryError::upload_unknown(uuid));
    };
    if upload.repository_id != repository.id {
        return Err(RegistryError::upload_unknown(uuid));
    }

    let offset = upload.offset.max(0) as u64;
    if !body.is_empty() {
        if let Some(raw_range) = headers
            .get(header::CONTENT_RANGE)
            .and_then(|value| value.to_str().ok())
        {
            let (start, _) = parse_content_range(raw_range)?;
            if start != offset {
                return Err(RegistryError::range_invalid(
                    "content range does not match the current upload size",
                ));
            }
        }
        super::check_blob_size(state, offset, body.len() as u64)?;
        uploads.append(uuid, offset, body).await?;
    }

    match verify_upload(state, uuid, &digest).await {
        Ok(true) => {}
        Ok(false) => {
            let _ = uploads.cancel(uuid).await;
            return Err(RegistryError::digest_invalid(&digest.to_string()));
        }
        Err(err) => {
            let _ = uploads.cancel(uuid).await;
            return Err(err);
        }
    }

    match uploads.complete(uuid, &digest).await {
        Ok(_) => {
            super::log_registry_event(
                &state.db,
                info,
                actor,
                "blob.push",
                Some(&repository),
                Some(&digest.to_string()),
                Some(&digest.to_string()),
                None,
                201,
            )
            .await;
            Ok(blob_created(name, &digest))
        }
        Err(err) => {
            let _ = uploads.cancel(uuid).await;
            Err(err)
        }
    }
}

pub async fn cancel(
    state: &AppState,
    actor: &AuthContext,
    _headers: &HeaderMap,
    info: &EventInfo,
    name: &str,
    uuid: &str,
) -> Result<Response, RegistryError> {
    super::validate_name(name)?;
    let repository = super::find_repo(state, actor, name, Action::Delete).await?;
    let uploads = super::uploads_service(state);
    let Some(upload) = uploads.status(uuid).await? else {
        return Err(RegistryError::upload_unknown(uuid));
    };
    if upload.repository_id != repository.id {
        return Err(RegistryError::upload_unknown(uuid));
    }
    uploads.cancel(uuid).await?;

    super::log_registry_event(
        &state.db,
        info,
        actor,
        "blob.upload.cancel",
        Some(&repository),
        Some(uuid),
        None,
        None,
        204,
    )
    .await;

    let mut response = super::respond(StatusCode::NO_CONTENT, Body::empty());
    response = super::set_header(response, "docker-upload-uuid", uuid);
    response = super::set_header(response, "content-length", "0");
    Ok(response)
}

fn upload_location(name: &str, uuid: &str) -> String {
    format!("/v2/{name}/blobs/uploads/{uuid}")
}

fn blob_created(name: &str, digest: &Digest) -> Response {
    let mut response = super::respond(StatusCode::CREATED, Body::empty());
    response = super::set_header(response, "location", format!("/v2/{name}/blobs/{digest}"));
    response = super::set_header(response, "docker-content-digest", digest.to_string());
    response = super::set_header(response, "content-length", "0");
    response
}

fn parse_content_range(raw: &str) -> Result<(u64, u64), RegistryError> {
    let raw = raw.trim();
    let raw = raw.strip_prefix("bytes ").unwrap_or(raw);
    let range = raw.split('/').next().unwrap_or(raw);
    let (start, end) = range
        .split_once('-')
        .ok_or_else(|| RegistryError::range_invalid("malformed content range"))?;
    let start: u64 = start
        .trim()
        .parse()
        .map_err(|_| RegistryError::range_invalid("malformed content range"))?;
    let end: u64 = end
        .trim()
        .parse()
        .map_err(|_| RegistryError::range_invalid("malformed content range"))?;
    if end < start {
        return Err(RegistryError::range_invalid(
            "content range end precedes its start",
        ));
    }
    Ok((start, end))
}

async fn verify_upload(
    state: &AppState,
    uuid: &str,
    digest: &Digest,
) -> Result<bool, RegistryError> {
    let path = state
        .storage
        .upload_data_path(uuid)
        .map_err(super::storage_failure)?;
    let mut file = tokio::fs::File::open(&path).await?;
    let mut verifier = digest.verifier();
    let mut buffer = vec![0u8; 64 * 1024];
    loop {
        let read = file.read(&mut buffer).await?;
        if read == 0 {
            break;
        }
        verifier.update(&buffer[..read]);
    }
    Ok(verifier.matches())
}
