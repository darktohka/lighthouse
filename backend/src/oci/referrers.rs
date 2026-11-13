//! OCI 1.1 referrers API (`GET /v2/<name>/referrers/<digest>`).
//!
//! A referrer is a manifest whose `subject` points at another manifest: how
//! attestations, signatures and SBOMs attach to an image without changing it.
//! `put_manifest` records `subject_digest`, so the reverse lookup is a plain
//! indexed query.
//!
//! The response is an OCI image index whose `manifests` array holds a
//! descriptor per referrer, carrying the referrer's own `artifactType` and
//! annotations so a client can tell an attestation from a signature without
//! fetching each manifest. `OCI-Filters-Applied: artifactType` is advertised
//! because `?artifactType=` is supported.
//!
//! A `digest` with no referrers returns `200` with an empty index rather than
//! `404`: the spec treats "no referrers" as a successful empty listing, and
//! clients depend on that to avoid falling back to tag scanning.

use axum::body::Body;
use axum::http::{HeaderMap, StatusCode};
use axum::response::Response;
use serde_json::{Value, json};

use crate::error::RegistryError;
use crate::oci::digest::Digest;
use crate::oci::media_types;
use crate::permissions::Action;
use crate::state::{AppState, AuthContext};


/// The `artifactType=` filter query parameter name.
const FILTER_PARAM: &str = "artifactType";

pub async fn get(
    state: &AppState,
    actor: &AuthContext,
    _headers: &HeaderMap,
    query: &[(String, String)],
    name: &str,
    digest_raw: &str,
) -> Result<Response, RegistryError> {
    let repository = super::find_repo(state, actor, name, Action::Pull).await?;
    let digest = Digest::parse(digest_raw)
        .map_err(|_| RegistryError::code(crate::error::ErrorCode::DigestInvalid))?;

    // The subject must exist *in this repository*, otherwise the caller learns
    // nothing and an unknown digest is indistinguishable from an unlinked one.
    if state.registry.manifest(&digest).await?.is_none()
        || !state
            .registry
            .manifest_in_repository(repository.id, &digest)
            .await?
    {
        return Err(RegistryError::manifest_unknown(digest_raw));
    }

    let filter = super::query_first(query, FILTER_PARAM).map(str::to_string);
    let rows = sqlx::query_as::<_, (String, String, i64, Vec<u8>, Option<String>)>(
        "SELECT m.digest, m.media_type, m.size, m.content, m.artifact_type \
         FROM manifests m \
         JOIN manifest_repositories mr ON mr.manifest_id = m.id \
         WHERE m.subject_digest = ? AND mr.repository_id = ? \
         ORDER BY m.id",
    )
    .bind(digest.to_string())
    .bind(repository.id)
    .fetch_all(&state.db)
    .await?;

    let mut descriptors = Vec::with_capacity(rows.len());
    for (referrer_digest, media_type, size, content, artifact_type) in rows {
        let body: Value = serde_json::from_slice(&content).unwrap_or(Value::Null);
        let resolved_type = artifact_type.or_else(|| {
            body.get("artifactType")
                .and_then(Value::as_str)
                .map(str::to_string)
        });
        if let Some(wanted) = &filter {
            let matches = resolved_type
                .as_deref()
                .is_some_and(|value| value.eq_ignore_ascii_case(wanted));
            if !matches {
                continue;
            }
        }
        let mut descriptor = json!({
            "mediaType": media_type,
            "digest": referrer_digest,
            "size": size,
        });
        if let Some(artifact_type) = &resolved_type {
            descriptor["artifactType"] = json!(artifact_type);
        }
        if let Some(annotations) = body.get("annotations") {
            if annotations.is_object() {
                descriptor["annotations"] = annotations.clone();
            }
        }
        descriptors.push(descriptor);
    }

    let index = json!({
        "schemaVersion": 2,
        "mediaType": media_types::OCI_IMAGE_INDEX,
        "manifests": descriptors,
    });
    let body = serde_json::to_vec(&index)
        .map_err(|_| RegistryError::internal("referrers serialization failed"))?;
    let length = body.len();

    let mut response = super::respond(StatusCode::OK, Body::from(body));
    response = super::set_header(response, "content-type", media_types::OCI_IMAGE_INDEX);
    response = super::set_header(response, "content-length", length.to_string());
    if filter.is_some() {
        response = super::set_header(response, "oci-filters-applied", FILTER_PARAM);
    }
    Ok(response)
}
