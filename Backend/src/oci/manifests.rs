//! Manifest handlers (`/v2/<name>/manifests/<reference>`).

use axum::body::Body;
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::Response;
use serde_json::Value;

use crate::error::{ErrorCode, RegistryError};
use crate::models::Manifest;
use crate::oci::digest::Digest;
use crate::oci::{media_types, reference};
use crate::permissions::Action;
use crate::state::{AppState, AuthContext};

use super::EventInfo;

pub async fn get(
    state: &AppState,
    actor: &AuthContext,
    headers: &HeaderMap,
    info: &EventInfo,
    name: &str,
    reference_raw: &str,
    head_only: bool,
) -> Result<Response, RegistryError> {
    super::validate_name(name)?;
    let repository = super::find_repo(state, actor, name, Action::Pull).await?;
    let digest = resolve_reference(state, repository.id, reference_raw).await?;

    let Some(manifest) = state.registry.manifest(&digest).await? else {
        return Err(RegistryError::manifest_unknown(reference_raw));
    };
    if !state
        .registry
        .manifest_in_repository(repository.id, &digest)
        .await?
    {
        return Err(RegistryError::manifest_unknown(reference_raw));
    }

    let stored_type = manifest.media_type.clone();
    let accept_values: Vec<&str> = headers
        .get_all(header::ACCEPT)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .collect();
    let accept = media_types::parse_accept_values(accept_values);

    let served = if media_types::negotiate(&stored_type, &accept) {
        manifest
    } else if stored_type == media_types::DOCKER_MANIFEST_LIST_V2 {
        match default_child(state, repository.id, manifest.id).await? {
            Some(child) => child,
            None => {
                return Err(RegistryError::manifest_unknown(reference_raw).with_detail(
                    serde_json::json!({ "reason": "no acceptable child manifest" }),
                ));
            }
        }
    } else {
        return Err(RegistryError::manifest_unknown(reference_raw).with_detail(
            serde_json::json!({ "reason": "manifest media type is not acceptable" }),
        ));
    };

    let served_digest = served.digest.clone();
    let served_type = served.media_type.clone();
    let served_size = served.size.max(0) as u64;
    let etag = format!("\"{served_digest}\"");
    let tag = if reference::is_digest_reference(reference_raw) {
        None
    } else {
        Some(reference_raw)
    };

    super::log_pull(
        state,
        actor,
        info,
        "manifest.pull",
        &repository,
        tag,
        &served_digest,
        Some(&served_type),
        Some(served.id),
    )
    .await;

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
            response = super::set_header(response, "etag", &etag);
            response = super::set_header(response, "docker-content-digest", &served_digest);
            return Ok(response);
        }
    }

    let body = if head_only {
        Body::empty()
    } else {
        Body::from(served.content)
    };
    let mut response = super::respond(StatusCode::OK, body);
    response = super::set_header(response, "content-type", &served_type);
    response = super::set_header(response, "content-length", served_size.to_string());
    response = super::set_header(response, "docker-content-digest", &served_digest);
    response = super::set_header(response, "etag", &etag);
    Ok(response)
}

pub async fn put(
    state: &AppState,
    actor: &AuthContext,
    headers: &HeaderMap,
    info: &EventInfo,
    name: &str,
    reference_raw: &str,
    body: &[u8],
) -> Result<Response, RegistryError> {
    super::validate_name(name)?;
    let repository = super::ensure_repo_for_push(state, actor, name).await?;

    let value: Value = serde_json::from_slice(body)
        .map_err(|err| RegistryError::manifest_invalid(format!("invalid manifest JSON: {err}")))?;
    if value.get("schemaVersion").and_then(Value::as_i64) != Some(2) {
        return Err(RegistryError::manifest_invalid(
            "manifest schemaVersion must be 2",
        ));
    }

    let media_type = determine_media_type(headers, &value);
    let digest = Digest::from_bytes(body);

    let tag = if reference::is_digest_reference(reference_raw) {
        let expected = Digest::parse(reference_raw)
            .map_err(|_| RegistryError::manifest_invalid("invalid digest reference"))?;
        if expected != digest {
            return Err(RegistryError::digest_invalid(&expected.to_string()));
        }
        None
    } else {
        if !reference::validate_tag(reference_raw) {
            return Err(RegistryError::tag_invalid(format!(
                "invalid tag `{reference_raw}`"
            )));
        }
        Some(reference_raw.to_string())
    };

    validate_references(state, repository.id, &media_type, &value).await?;

    let manifest = state
        .registry
        .put_manifest(repository.id, &media_type, body)
        .await?;
    let digest_text = manifest.digest.clone();

    if let Some(tag) = &tag {
        state.registry.set_tag(repository.id, tag, &digest).await?;
        super::log_activity(
            &state.db,
            actor,
            &repository,
            "tag.pushed",
            &format!("Pushed tag {tag}"),
            Some(serde_json::json!({ "tag": tag, "digest": digest_text })),
        )
        .await;
    } else {
        super::log_activity(
            &state.db,
            actor,
            &repository,
            "manifest.pushed",
            &format!("Pushed manifest {digest_text}"),
            Some(serde_json::json!({ "digest": digest_text })),
        )
        .await;
    }
    super::log_registry_event(
        &state.db,
        info,
        actor,
        "manifest.push",
        Some(&repository),
        tag.as_deref().or(Some(digest_text.as_str())),
        Some(&digest_text),
        Some(&media_type),
        201,
    )
    .await;

    let mut response = super::respond(StatusCode::CREATED, Body::empty());
    response = super::set_header(response, "location", format!("/v2/{name}/manifests/{digest_text}"));
    response = super::set_header(response, "docker-content-digest", &digest_text);
    response = super::set_header(response, "content-length", "0");
    Ok(response)
}

pub async fn delete(
    state: &AppState,
    actor: &AuthContext,
    _headers: &HeaderMap,
    info: &EventInfo,
    name: &str,
    reference_raw: &str,
) -> Result<Response, RegistryError> {
    super::validate_name(name)?;
    let repository = super::find_repo(state, actor, name, Action::Delete).await?;

    if reference::is_digest_reference(reference_raw) {
        let digest = Digest::parse(reference_raw)
            .map_err(|_| RegistryError::manifest_unknown(reference_raw))?;
        if !state.registry.delete_manifest(repository.id, &digest).await? {
            return Err(RegistryError::manifest_unknown(reference_raw));
        }
        super::log_activity(
            &state.db,
            actor,
            &repository,
            "manifest.deleted",
            &format!("Deleted manifest {digest}"),
            Some(serde_json::json!({ "digest": digest.to_string() })),
        )
        .await;
        super::log_registry_event(
            &state.db,
            info,
            actor,
            "manifest.delete",
            Some(&repository),
            Some(reference_raw),
            Some(&digest.to_string()),
            None,
            202,
        )
        .await;
    } else {
        if !reference::validate_tag(reference_raw) {
            return Err(RegistryError::manifest_unknown(reference_raw));
        }
        if !state.registry.delete_tag(repository.id, reference_raw).await? {
            return Err(RegistryError::manifest_unknown(reference_raw));
        }
        super::log_activity(
            &state.db,
            actor,
            &repository,
            "tag.deleted",
            &format!("Deleted tag {reference_raw}"),
            Some(serde_json::json!({ "tag": reference_raw })),
        )
        .await;
        super::log_registry_event(
            &state.db,
            info,
            actor,
            "tag.delete",
            Some(&repository),
            Some(reference_raw),
            None,
            None,
            202,
        )
        .await;
    }

    Ok(super::respond(StatusCode::ACCEPTED, Body::empty()))
}

async fn resolve_reference(
    state: &AppState,
    repository_id: i64,
    reference_raw: &str,
) -> Result<Digest, RegistryError> {
    if reference::is_digest_reference(reference_raw) {
        return Digest::parse(reference_raw)
            .map_err(|_| RegistryError::manifest_unknown(reference_raw));
    }
    if !reference::validate_tag(reference_raw) {
        return Err(RegistryError::manifest_unknown(reference_raw));
    }
    match state.registry.resolve_tag(repository_id, reference_raw).await? {
        Some(digest) => Ok(digest),
        None => Err(RegistryError::manifest_unknown(reference_raw)),
    }
}

async fn default_child(
    state: &AppState,
    repository_id: i64,
    parent_id: i64,
) -> Result<Option<Manifest>, RegistryError> {
    let child = sqlx::query_as::<_, Manifest>(
        "SELECT m.* FROM manifest_children mc \
         JOIN manifests m ON m.id = mc.child_manifest_id \
         WHERE mc.parent_manifest_id = ? \
           AND mc.platform_os = 'linux' AND mc.platform_architecture = 'amd64' \
         ORDER BY m.id LIMIT 1",
    )
    .bind(parent_id)
    .fetch_optional(&state.db)
    .await?;

    let Some(child) = child else {
        return Ok(None);
    };
    let Ok(digest) = Digest::parse(&child.digest) else {
        return Ok(None);
    };
    if !state
        .registry
        .manifest_in_repository(repository_id, &digest)
        .await?
    {
        return Ok(None);
    }
    Ok(Some(child))
}

async fn validate_references(
    state: &AppState,
    repository_id: i64,
    media_type: &str,
    value: &Value,
) -> Result<(), RegistryError> {
    if media_types::is_index_type(media_type) {
        let manifests = value
            .get("manifests")
            .and_then(Value::as_array)
            .ok_or_else(|| RegistryError::manifest_invalid("`manifests` must be an array"))?;
        let mut missing = Vec::new();
        for entry in manifests {
            let digest = descriptor_digest(entry)?;
            let present = state.registry.manifest(&digest).await?.is_some()
                && state
                    .registry
                    .manifest_in_repository(repository_id, &digest)
                    .await?;
            if !present {
                missing.push(digest.to_string());
            }
        }
        if !missing.is_empty() {
            return Err(RegistryError::code(ErrorCode::ManifestBlobUnknown)
                .with_detail(serde_json::json!({ "digests": missing })));
        }
        return Ok(());
    }

    if media_types::is_manifest_type(media_type) {
        let config = value
            .get("config")
            .ok_or_else(|| RegistryError::manifest_invalid("`config` is required"))?;
        let layers = value
            .get("layers")
            .and_then(Value::as_array)
            .ok_or_else(|| RegistryError::manifest_invalid("`layers` must be an array"))?;
        let mut digests = Vec::with_capacity(layers.len() + 1);
        digests.push(descriptor_digest(config)?);
        for layer in layers {
            digests.push(descriptor_digest(layer)?);
        }
        let mut missing = Vec::new();
        for digest in digests {
            let present = state.registry.blob_stat(&digest).await?.is_some()
                && state
                    .registry
                    .blob_in_repository(repository_id, &digest)
                    .await?;
            if !present {
                missing.push(digest.to_string());
            }
        }
        if !missing.is_empty() {
            return Err(RegistryError::code(ErrorCode::ManifestBlobUnknown)
                .with_detail(serde_json::json!({ "digests": missing })));
        }
    }

    Ok(())
}

fn determine_media_type(headers: &HeaderMap, value: &Value) -> String {
    if let Some(content_type) = headers
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
    {
        let base = content_type.split(';').next().unwrap_or("").trim();
        if media_types::is_manifest_or_index(base) {
            return base.to_string();
        }
    }
    if let Some(media_type) = value.get("mediaType").and_then(Value::as_str)
        && media_types::is_manifest_or_index(media_type)
    {
        return media_type.to_string();
    }
    if value.get("manifests").is_some() {
        media_types::OCI_IMAGE_INDEX.to_string()
    } else {
        media_types::OCI_IMAGE_MANIFEST.to_string()
    }
}

fn descriptor_digest(value: &Value) -> Result<Digest, RegistryError> {
    let raw = value
        .get("digest")
        .and_then(Value::as_str)
        .ok_or_else(|| RegistryError::manifest_invalid("descriptor is missing `digest`"))?;
    Digest::parse(raw)
        .map_err(|_| RegistryError::manifest_invalid(format!("invalid descriptor digest `{raw}`")))
}
