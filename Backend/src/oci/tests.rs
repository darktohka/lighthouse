use axum::Router;
use axum::body::Body;
use axum::http::{Method, Request, StatusCode};
use base64::Engine as _;
use tower::ServiceExt;

use crate::auth::test_support::{body_json, create_user, test_state};
use crate::oci::digest::Digest;
use crate::oci::media_types;
use crate::state::AppState;

const USER: &str = "darktohka";
const PASSWORD: &str = "correct-horse-battery";

async fn harness() -> (tempfile::TempDir, AppState, Router) {
    let (dir, state) = test_state().await;
    create_user(&state, USER).await;
    let app = crate::routes::build(state.clone());
    (dir, state, app)
}

fn basic(user: &str) -> String {
    format!(
        "Basic {}",
        base64::engine::general_purpose::STANDARD.encode(format!("{user}:{PASSWORD}"))
    )
}

async fn send(
    app: &Router,
    method: Method,
    uri: &str,
    user: Option<&str>,
    headers: &[(&str, &str)],
    body: &[u8],
) -> axum::response::Response {
    let mut builder = Request::builder().method(method).uri(uri);
    if let Some(user) = user {
        builder = builder.header("authorization", basic(user));
    }
    for (name, value) in headers {
        builder = builder.header(*name, *value);
    }
    let request = builder.body(Body::from(body.to_vec())).expect("request");
    app.clone().oneshot(request).await.expect("response")
}

async fn body_bytes(response: axum::response::Response) -> Vec<u8> {
    axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body")
        .to_vec()
}

fn header_str(response: &axum::response::Response, name: &str) -> String {
    response
        .headers()
        .get(name)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default()
        .to_string()
}

fn assert_api_version(response: &axum::response::Response) {
    assert_eq!(
        header_str(response, "docker-distribution-api-version"),
        "registry/2.0"
    );
}

async fn push_blob(app: &Router, name: &str, content: &[u8]) -> String {
    let digest = Digest::from_bytes(content).to_string();
    let start = send(
        app,
        Method::POST,
        &format!("/v2/{name}/blobs/uploads/"),
        Some(USER),
        &[],
        &[],
    )
    .await;
    assert_eq!(start.status(), StatusCode::ACCEPTED);
    let location = header_str(&start, "location");
    let patch = send(
        app,
        Method::PATCH,
        &location,
        Some(USER),
        &[("content-type", "application/octet-stream")],
        content,
    )
    .await;
    assert_eq!(patch.status(), StatusCode::ACCEPTED);
    let location = header_str(&patch, "location");
    let put = send(
        app,
        Method::PUT,
        &format!("{location}?digest={digest}"),
        Some(USER),
        &[],
        &[],
    )
    .await;
    assert_eq!(put.status(), StatusCode::CREATED);
    digest
}

async fn push_manifest(
    app: &Router,
    name: &str,
    reference: &str,
    media_type: &str,
    body: &[u8],
) -> String {
    let response = send(
        app,
        Method::PUT,
        &format!("/v2/{name}/manifests/{reference}"),
        Some(USER),
        &[("content-type", media_type)],
        body,
    )
    .await;
    assert_eq!(response.status(), StatusCode::CREATED);
    assert!(response.headers().contains_key("location"));
    assert!(response.headers().contains_key("docker-content-digest"));
    Digest::from_bytes(body).to_string()
}

fn manifest_body(
    config: &str,
    config_size: usize,
    layer: &str,
    layer_size: usize,
    media_type: &str,
    config_type: &str,
    layer_type: &str,
) -> Vec<u8> {
    serde_json::json!({
        "schemaVersion": 2,
        "mediaType": media_type,
        "config": { "mediaType": config_type, "digest": config, "size": config_size },
        "layers": [ { "mediaType": layer_type, "digest": layer, "size": layer_size } ]
    })
    .to_string()
    .into_bytes()
}

fn oci_manifest(config: &str, config_size: usize, layer: &str, layer_size: usize) -> Vec<u8> {
    manifest_body(
        config,
        config_size,
        layer,
        layer_size,
        media_types::OCI_IMAGE_MANIFEST,
        media_types::OCI_IMAGE_CONFIG,
        media_types::OCI_IMAGE_LAYER_GZIP,
    )
}

#[tokio::test]
async fn base_endpoint_reports_api_version() {
    let (_dir, _state, app) = harness().await;

    let anonymous = send(&app, Method::GET, "/v2/", None, &[], &[]).await;
    assert_eq!(anonymous.status(), StatusCode::UNAUTHORIZED);
    assert_api_version(&anonymous);
    assert!(header_str(&anonymous, "www-authenticate").starts_with("Basic realm="));

    let response = send(&app, Method::GET, "/v2/", Some(USER), &[], &[]).await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_api_version(&response);
    let json = body_json(response).await;
    assert_eq!(json, serde_json::json!({}));

    let post = send(&app, Method::POST, "/v2/", Some(USER), &[], &[]).await;
    assert_eq!(post.status(), StatusCode::OK);
}

#[tokio::test]
async fn full_blob_push_and_pull_lifecycle() {
    let (_dir, _state, app) = harness().await;
    let content = b"layer-content-0123456789".to_vec();
    let digest = push_blob(&app, "darktohka/site", &content).await;

    let get = send(
        &app,
        Method::GET,
        &format!("/v2/darktohka/site/blobs/{digest}"),
        Some(USER),
        &[],
        &[],
    )
    .await;
    assert_eq!(get.status(), StatusCode::OK);
    assert_api_version(&get);
    assert_eq!(
        header_str(&get, "content-length"),
        content.len().to_string()
    );
    assert_eq!(header_str(&get, "docker-content-digest"), digest);
    assert_eq!(header_str(&get, "etag"), format!("\"{digest}\""));
    assert_eq!(header_str(&get, "accept-ranges"), "bytes");
    assert_eq!(body_bytes(get).await, content);

    let head = send(
        &app,
        Method::HEAD,
        &format!("/v2/darktohka/site/blobs/{digest}"),
        Some(USER),
        &[],
        &[],
    )
    .await;
    assert_eq!(head.status(), StatusCode::OK);
    assert_eq!(
        header_str(&head, "content-length"),
        content.len().to_string()
    );
    assert!(body_bytes(head).await.is_empty());

    let range = send(
        &app,
        Method::GET,
        &format!("/v2/darktohka/site/blobs/{digest}"),
        Some(USER),
        &[("range", "bytes=6-11")],
        &[],
    )
    .await;
    assert_eq!(range.status(), StatusCode::PARTIAL_CONTENT);
    assert_eq!(
        header_str(&range, "content-range"),
        format!("bytes 6-11/{}", content.len())
    );
    assert_eq!(body_bytes(range).await, &content[6..12]);

    let not_modified = send(
        &app,
        Method::GET,
        &format!("/v2/darktohka/site/blobs/{digest}"),
        Some(USER),
        &[("if-none-match", &format!("\"{digest}\""))],
        &[],
    )
    .await;
    assert_eq!(not_modified.status(), StatusCode::NOT_MODIFIED);
}

#[tokio::test]
async fn monolithic_blob_push() {
    let (_dir, _state, app) = harness().await;
    let content = b"monolithic-body".to_vec();
    let digest = Digest::from_bytes(&content).to_string();
    let response = send(
        &app,
        Method::POST,
        &format!("/v2/darktohka/site/blobs/uploads/?digest={digest}"),
        Some(USER),
        &[],
        &content,
    )
    .await;
    assert_eq!(response.status(), StatusCode::CREATED);
    assert_eq!(header_str(&response, "docker-content-digest"), digest);

    let get = send(
        &app,
        Method::GET,
        &format!("/v2/darktohka/site/blobs/{digest}"),
        Some(USER),
        &[],
        &[],
    )
    .await;
    assert_eq!(body_bytes(get).await, content);
}

#[tokio::test]
async fn monolithic_blob_digest_mismatch_is_rejected() {
    let (_dir, _state, app) = harness().await;
    let wrong = Digest::from_bytes(b"different").to_string();
    let response = send(
        &app,
        Method::POST,
        &format!("/v2/darktohka/site/blobs/uploads/?digest={wrong}"),
        Some(USER),
        &[],
        b"actual",
    )
    .await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let json = body_json(response).await;
    assert_eq!(json["errors"][0]["code"], "DIGEST_INVALID");
}

#[tokio::test]
async fn chunked_upload_resumes() {
    let (_dir, _state, app) = harness().await;
    let content = b"chunk-one-chunk-two".to_vec();
    let digest = Digest::from_bytes(&content).to_string();

    let start = send(
        &app,
        Method::POST,
        "/v2/darktohka/site/blobs/uploads/",
        Some(USER),
        &[],
        &[],
    )
    .await;
    assert_eq!(start.status(), StatusCode::ACCEPTED);
    assert_eq!(header_str(&start, "range"), "0-0");
    assert!(!header_str(&start, "docker-upload-uuid").is_empty());
    let location = header_str(&start, "location");

    let first = send(
        &app,
        Method::PATCH,
        &location,
        Some(USER),
        &[
            ("content-type", "application/octet-stream"),
            ("content-range", "0-8"),
        ],
        &content[..9],
    )
    .await;
    assert_eq!(first.status(), StatusCode::ACCEPTED);
    assert_eq!(header_str(&first, "range"), "0-8");

    let status = send(&app, Method::GET, &location, Some(USER), &[], &[]).await;
    assert_eq!(status.status(), StatusCode::NO_CONTENT);
    assert_eq!(header_str(&status, "range"), "0-8");

    let second = send(
        &app,
        Method::PATCH,
        &location,
        Some(USER),
        &[
            ("content-type", "application/octet-stream"),
            ("content-range", "9-18"),
        ],
        &content[9..],
    )
    .await;
    assert_eq!(second.status(), StatusCode::ACCEPTED);

    let put = send(
        &app,
        Method::PUT,
        &format!("{location}?digest={digest}"),
        Some(USER),
        &[],
        &[],
    )
    .await;
    assert_eq!(put.status(), StatusCode::CREATED);
    assert_eq!(header_str(&put, "docker-content-digest"), digest);
}

#[tokio::test]
async fn out_of_order_range_is_rejected() {
    let (_dir, _state, app) = harness().await;
    let start = send(
        &app,
        Method::POST,
        "/v2/darktohka/site/blobs/uploads/",
        Some(USER),
        &[],
        &[],
    )
    .await;
    let location = header_str(&start, "location");
    let response = send(
        &app,
        Method::PATCH,
        &location,
        Some(USER),
        &[
            ("content-type", "application/octet-stream"),
            ("content-range", "5-9"),
        ],
        b"abcde",
    )
    .await;
    assert_eq!(response.status(), StatusCode::RANGE_NOT_SATISFIABLE);
    let json = body_json(response).await;
    assert_eq!(json["errors"][0]["code"], "RANGE_INVALID");
}

#[tokio::test]
async fn content_length_mismatch_is_rejected() {
    let (_dir, _state, app) = harness().await;
    let start = send(
        &app,
        Method::POST,
        "/v2/darktohka/site/blobs/uploads/",
        Some(USER),
        &[],
        &[],
    )
    .await;
    let location = header_str(&start, "location");
    let response = send(
        &app,
        Method::PATCH,
        &location,
        Some(USER),
        &[
            ("content-type", "application/octet-stream"),
            ("content-length", "99"),
        ],
        b"short",
    )
    .await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let json = body_json(response).await;
    assert_eq!(json["errors"][0]["code"], "SIZE_INVALID");
}

#[tokio::test]
async fn cross_repository_blob_mount() {
    let (_dir, _state, app) = harness().await;
    let content = b"mountable-layer".to_vec();
    let digest = push_blob(&app, "darktohka/source", &content).await;

    let mount = send(
        &app,
        Method::POST,
        &format!(
            "/v2/darktohka/target/blobs/uploads/?mount={digest}&from=darktohka/source"
        ),
        Some(USER),
        &[],
        &[],
    )
    .await;
    assert_eq!(mount.status(), StatusCode::CREATED);
    assert_eq!(header_str(&mount, "docker-content-digest"), digest);

    let get = send(
        &app,
        Method::GET,
        &format!("/v2/darktohka/target/blobs/{digest}"),
        Some(USER),
        &[],
        &[],
    )
    .await;
    assert_eq!(get.status(), StatusCode::OK);

    let unknown = Digest::from_bytes(b"never-pushed").to_string();
    let fallback = send(
        &app,
        Method::POST,
        &format!(
            "/v2/darktohka/target/blobs/uploads/?mount={unknown}&from=darktohka/source"
        ),
        Some(USER),
        &[],
        &[],
    )
    .await;
    assert_eq!(fallback.status(), StatusCode::ACCEPTED);
}

#[tokio::test]
async fn upload_cancel_then_status_is_unknown() {
    let (_dir, _state, app) = harness().await;
    let start = send(
        &app,
        Method::POST,
        "/v2/darktohka/site/blobs/uploads/",
        Some(USER),
        &[],
        &[],
    )
    .await;
    let location = header_str(&start, "location");
    let uuid = header_str(&start, "docker-upload-uuid");

    let cancel = send(&app, Method::DELETE, &location, Some(USER), &[], &[]).await;
    assert_eq!(cancel.status(), StatusCode::NO_CONTENT);
    assert_eq!(header_str(&cancel, "docker-upload-uuid"), uuid);

    let status = send(&app, Method::GET, &location, Some(USER), &[], &[]).await;
    assert_eq!(status.status(), StatusCode::NOT_FOUND);
    let json = body_json(status).await;
    assert_eq!(json["errors"][0]["code"], "BLOB_UPLOAD_UNKNOWN");
}

#[tokio::test]
async fn oci_manifest_push_and_fetch() {
    let (_dir, _state, app) = harness().await;
    let config = b"config-bytes".to_vec();
    let layer = b"layer-bytes".to_vec();
    let config_digest = push_blob(&app, "darktohka/site", &config).await;
    let layer_digest = push_blob(&app, "darktohka/site", &layer).await;
    let body = oci_manifest(&config_digest, config.len(), &layer_digest, layer.len());
    let digest = push_manifest(&app, "darktohka/site", "latest", media_types::OCI_IMAGE_MANIFEST, &body).await;

    let get = send(
        &app,
        Method::GET,
        "/v2/darktohka/site/manifests/latest",
        Some(USER),
        &[("accept", media_types::OCI_IMAGE_MANIFEST)],
        &[],
    )
    .await;
    assert_eq!(get.status(), StatusCode::OK);
    assert_eq!(header_str(&get, "content-type"), media_types::OCI_IMAGE_MANIFEST);
    assert_eq!(header_str(&get, "content-length"), body.len().to_string());
    assert_eq!(header_str(&get, "docker-content-digest"), digest);
    assert_eq!(header_str(&get, "etag"), format!("\"{digest}\""));
    assert_eq!(body_bytes(get).await, body);

    let by_digest = send(
        &app,
        Method::GET,
        &format!("/v2/darktohka/site/manifests/{digest}"),
        Some(USER),
        &[("accept", media_types::OCI_IMAGE_MANIFEST)],
        &[],
    )
    .await;
    assert_eq!(by_digest.status(), StatusCode::OK);

    let head = send(
        &app,
        Method::HEAD,
        "/v2/darktohka/site/manifests/latest",
        Some(USER),
        &[("accept", media_types::OCI_IMAGE_MANIFEST)],
        &[],
    )
    .await;
    assert_eq!(head.status(), StatusCode::OK);
    assert!(body_bytes(head).await.is_empty());

    let not_modified = send(
        &app,
        Method::GET,
        "/v2/darktohka/site/manifests/latest",
        Some(USER),
        &[
            ("accept", media_types::OCI_IMAGE_MANIFEST),
            ("if-none-match", &format!("\"{digest}\"")),
        ],
        &[],
    )
    .await;
    assert_eq!(not_modified.status(), StatusCode::NOT_MODIFIED);
}

#[tokio::test]
async fn unknown_manifest_tag_is_not_found() {
    let (_dir, _state, app) = harness().await;
    push_blob(&app, "darktohka/site", b"seed").await;
    let response = send(
        &app,
        Method::GET,
        "/v2/darktohka/site/manifests/absent",
        Some(USER),
        &[("accept", media_types::OCI_IMAGE_MANIFEST)],
        &[],
    )
    .await;
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    let json = body_json(response).await;
    assert_eq!(json["errors"][0]["code"], "MANIFEST_UNKNOWN");
}

#[tokio::test]
async fn manifest_referencing_missing_blob_is_rejected() {
    let (_dir, _state, app) = harness().await;
    let config = b"cfg".to_vec();
    let config_digest = push_blob(&app, "darktohka/site", &config).await;
    push_blob(&app, "darktohka/site", b"present-layer").await;
    let missing = Digest::from_bytes(b"missing-layer").to_string();
    let body = oci_manifest(&config_digest, config.len(), &missing, 13);
    let response = send(
        &app,
        Method::PUT,
        "/v2/darktohka/site/manifests/broken",
        Some(USER),
        &[("content-type", media_types::OCI_IMAGE_MANIFEST)],
        &body,
    )
    .await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let json = body_json(response).await;
    assert_eq!(json["errors"][0]["code"], "MANIFEST_BLOB_UNKNOWN");
}

#[tokio::test]
async fn manifest_digest_reference_mismatch_is_rejected() {
    let (_dir, _state, app) = harness().await;
    let config = b"cfg".to_vec();
    let layer = b"lyr".to_vec();
    let config_digest = push_blob(&app, "darktohka/site", &config).await;
    let layer_digest = push_blob(&app, "darktohka/site", &layer).await;
    let body = oci_manifest(&config_digest, config.len(), &layer_digest, layer.len());
    let wrong = Digest::from_bytes(b"not-the-body").to_string();
    let response = send(
        &app,
        Method::PUT,
        &format!("/v2/darktohka/site/manifests/{wrong}"),
        Some(USER),
        &[("content-type", media_types::OCI_IMAGE_MANIFEST)],
        &body,
    )
    .await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let json = body_json(response).await;
    assert_eq!(json["errors"][0]["code"], "DIGEST_INVALID");
}

#[tokio::test]
async fn manifest_delete_by_tag_and_digest() {
    let (_dir, _state, app) = harness().await;
    let config = b"cfg".to_vec();
    let layer = b"lyr".to_vec();
    let config_digest = push_blob(&app, "darktohka/site", &config).await;
    let layer_digest = push_blob(&app, "darktohka/site", &layer).await;
    let body = oci_manifest(&config_digest, config.len(), &layer_digest, layer.len());
    let digest = push_manifest(&app, "darktohka/site", "v1", media_types::OCI_IMAGE_MANIFEST, &body).await;

    let delete_tag = send(
        &app,
        Method::DELETE,
        "/v2/darktohka/site/manifests/v1",
        Some(USER),
        &[],
        &[],
    )
    .await;
    assert_eq!(delete_tag.status(), StatusCode::ACCEPTED);

    let gone = send(
        &app,
        Method::GET,
        "/v2/darktohka/site/manifests/v1",
        Some(USER),
        &[("accept", media_types::OCI_IMAGE_MANIFEST)],
        &[],
    )
    .await;
    assert_eq!(gone.status(), StatusCode::NOT_FOUND);

    let delete_digest = send(
        &app,
        Method::DELETE,
        &format!("/v2/darktohka/site/manifests/{digest}"),
        Some(USER),
        &[],
        &[],
    )
    .await;
    assert_eq!(delete_digest.status(), StatusCode::ACCEPTED);
}

#[tokio::test]
async fn multi_arch_index_round_trips() {
    let (_dir, _state, app) = harness().await;
    let config = b"cfg".to_vec();
    let config_digest = push_blob(&app, "darktohka/site", &config).await;

    let mut children = Vec::new();
    for arch in ["amd64", "arm64"] {
        let layer = format!("layer-{arch}").into_bytes();
        let layer_digest = push_blob(&app, "darktohka/site", &layer).await;
        let body = oci_manifest(&config_digest, config.len(), &layer_digest, layer.len());
        let digest = push_manifest(
            &app,
            "darktohka/site",
            &format!("child-{arch}"),
            media_types::OCI_IMAGE_MANIFEST,
            &body,
        )
        .await;
        children.push((digest, body.len(), arch));
    }

    let manifests: Vec<serde_json::Value> = children
        .iter()
        .map(|(digest, size, arch)| {
            serde_json::json!({
                "mediaType": media_types::OCI_IMAGE_MANIFEST,
                "digest": digest,
                "size": size,
                "platform": { "os": "linux", "architecture": arch }
            })
        })
        .collect();
    let index = serde_json::json!({
        "schemaVersion": 2,
        "mediaType": media_types::OCI_IMAGE_INDEX,
        "manifests": manifests
    })
    .to_string()
    .into_bytes();
    push_manifest(&app, "darktohka/site", "multi", media_types::OCI_IMAGE_INDEX, &index).await;

    let get = send(
        &app,
        Method::GET,
        "/v2/darktohka/site/manifests/multi",
        Some(USER),
        &[("accept", media_types::OCI_IMAGE_INDEX)],
        &[],
    )
    .await;
    assert_eq!(get.status(), StatusCode::OK);
    assert_eq!(header_str(&get, "content-type"), media_types::OCI_IMAGE_INDEX);
    assert_eq!(body_bytes(get).await, index);

    for (digest, _, _) in &children {
        let child = send(
            &app,
            Method::GET,
            &format!("/v2/darktohka/site/manifests/{digest}"),
            Some(USER),
            &[("accept", media_types::OCI_IMAGE_MANIFEST)],
            &[],
        )
        .await;
        assert_eq!(child.status(), StatusCode::OK);
    }
}

#[tokio::test]
async fn docker_schema2_and_manifest_list_are_accepted() {
    let (_dir, _state, app) = harness().await;
    let config = b"docker-config".to_vec();
    let config_digest = push_blob(&app, "darktohka/site", &config).await;

    let mut children: Vec<(String, Vec<u8>)> = Vec::new();
    for arch in ["amd64", "arm64"] {
        let layer = format!("docker-layer-{arch}").into_bytes();
        let layer_digest = push_blob(&app, "darktohka/site", &layer).await;
        let body = manifest_body(
            &config_digest,
            config.len(),
            &layer_digest,
            layer.len(),
            media_types::DOCKER_MANIFEST_V2,
            media_types::DOCKER_CONFIG_V1,
            media_types::DOCKER_LAYER_GZIP,
        );
        let digest = push_manifest(
            &app,
            "darktohka/site",
            &format!("dchild-{arch}"),
            media_types::DOCKER_MANIFEST_V2,
            &body,
        )
        .await;
        children.push((digest, body));
    }

    let manifests: Vec<serde_json::Value> = children
        .iter()
        .enumerate()
        .map(|(index, (digest, body))| {
            serde_json::json!({
                "mediaType": media_types::DOCKER_MANIFEST_V2,
                "digest": digest,
                "size": body.len(),
                "platform": {
                    "os": "linux",
                    "architecture": if index == 0 { "amd64" } else { "arm64" }
                }
            })
        })
        .collect();
    let list = serde_json::json!({
        "schemaVersion": 2,
        "mediaType": media_types::DOCKER_MANIFEST_LIST_V2,
        "manifests": manifests
    })
    .to_string()
    .into_bytes();
    push_manifest(
        &app,
        "darktohka/site",
        "docker-list",
        media_types::DOCKER_MANIFEST_LIST_V2,
        &list,
    )
    .await;

    let list_get = send(
        &app,
        Method::GET,
        "/v2/darktohka/site/manifests/docker-list",
        Some(USER),
        &[("accept", media_types::DOCKER_MANIFEST_LIST_V2)],
        &[],
    )
    .await;
    assert_eq!(list_get.status(), StatusCode::OK);
    assert_eq!(
        header_str(&list_get, "content-type"),
        media_types::DOCKER_MANIFEST_LIST_V2
    );

    let fallback = send(
        &app,
        Method::GET,
        "/v2/darktohka/site/manifests/docker-list",
        Some(USER),
        &[("accept", media_types::DOCKER_MANIFEST_V2)],
        &[],
    )
    .await;
    assert_eq!(fallback.status(), StatusCode::OK);
    assert_eq!(
        header_str(&fallback, "content-type"),
        media_types::DOCKER_MANIFEST_V2
    );
    assert_eq!(body_bytes(fallback).await, children[0].1);
}

#[tokio::test]
async fn tags_list_is_sorted_and_paginated() {
    let (_dir, _state, app) = harness().await;
    let config = b"cfg".to_vec();
    let layer = b"lyr".to_vec();
    let config_digest = push_blob(&app, "darktohka/site", &config).await;
    let layer_digest = push_blob(&app, "darktohka/site", &layer).await;
    let body = oci_manifest(&config_digest, config.len(), &layer_digest, layer.len());
    for tag in ["alpha", "beta", "gamma"] {
        push_manifest(&app, "darktohka/site", tag, media_types::OCI_IMAGE_MANIFEST, &body).await;
    }

    let all = send(
        &app,
        Method::GET,
        "/v2/darktohka/site/tags/list",
        Some(USER),
        &[],
        &[],
    )
    .await;
    assert_eq!(all.status(), StatusCode::OK);
    let json = body_json(all).await;
    assert_eq!(json["name"], "darktohka/site");
    assert_eq!(json["tags"], serde_json::json!(["alpha", "beta", "gamma"]));

    let first = send(
        &app,
        Method::GET,
        "/v2/darktohka/site/tags/list?n=1",
        Some(USER),
        &[],
        &[],
    )
    .await;
    assert!(header_str(&first, "link").contains("rel=\"next\""));
    let json = body_json(first).await;
    assert_eq!(json["tags"], serde_json::json!(["alpha"]));

    let second = send(
        &app,
        Method::GET,
        "/v2/darktohka/site/tags/list?n=1&last=alpha",
        Some(USER),
        &[],
        &[],
    )
    .await;
    let json = body_json(second).await;
    assert_eq!(json["tags"], serde_json::json!(["beta"]));

    let zero = send(
        &app,
        Method::GET,
        "/v2/darktohka/site/tags/list?n=0",
        Some(USER),
        &[],
        &[],
    )
    .await;
    assert_eq!(header_str(&zero, "link"), "");
    let json = body_json(zero).await;
    assert_eq!(json["tags"], serde_json::json!([]));

    let invalid = send(
        &app,
        Method::GET,
        "/v2/darktohka/site/tags/list?n=abc",
        Some(USER),
        &[],
        &[],
    )
    .await;
    assert_eq!(invalid.status(), StatusCode::BAD_REQUEST);
    let json = body_json(invalid).await;
    assert_eq!(json["errors"][0]["code"], "PAGINATION_NUMBER_INVALID");
}

#[tokio::test]
async fn catalog_requires_auth_and_paginates() {
    let (_dir, _state, app) = harness().await;
    push_blob(&app, "darktohka/alpha", b"a").await;
    push_blob(&app, "darktohka/beta", b"b").await;

    let anonymous = send(&app, Method::GET, "/v2/_catalog", None, &[], &[]).await;
    assert_eq!(anonymous.status(), StatusCode::UNAUTHORIZED);
    assert!(header_str(&anonymous, "www-authenticate").contains("Basic realm="));

    let all = send(&app, Method::GET, "/v2/_catalog", Some(USER), &[], &[]).await;
    assert_eq!(all.status(), StatusCode::OK);
    assert_api_version(&all);
    let json = body_json(all).await;
    assert_eq!(
        json["repositories"],
        serde_json::json!(["darktohka/alpha", "darktohka/beta"])
    );

    let first = send(&app, Method::GET, "/v2/_catalog?n=1", Some(USER), &[], &[]).await;
    assert!(header_str(&first, "link").contains("rel=\"next\""));
    let json = body_json(first).await;
    assert_eq!(json["repositories"], serde_json::json!(["darktohka/alpha"]));

    let invalid = send(
        &app,
        Method::GET,
        "/v2/_catalog?n=xyz",
        Some(USER),
        &[],
        &[],
    )
    .await;
    assert_eq!(invalid.status(), StatusCode::BAD_REQUEST);
    let json = body_json(invalid).await;
    assert_eq!(json["errors"][0]["code"], "PAGINATION_NUMBER_INVALID");
}

#[tokio::test]
async fn private_repository_requires_authentication() {
    let (_dir, state, app) = harness().await;
    let content = b"secret-blob".to_vec();
    let digest = push_blob(&app, "darktohka/site", &content).await;

    let anonymous = send(
        &app,
        Method::GET,
        &format!("/v2/darktohka/site/blobs/{digest}"),
        None,
        &[],
        &[],
    )
    .await;
    assert_eq!(anonymous.status(), StatusCode::UNAUTHORIZED);
    assert!(
        header_str(&anonymous, "www-authenticate").contains("Basic realm=\"Lighthouse Registry\"")
    );

    create_user(&state, "bob").await;
    let forbidden = send(
        &app,
        Method::GET,
        &format!("/v2/darktohka/site/blobs/{digest}"),
        Some("bob"),
        &[],
        &[],
    )
    .await;
    assert_eq!(forbidden.status(), StatusCode::FORBIDDEN);
    let json = body_json(forbidden).await;
    assert_eq!(json["errors"][0]["code"], "DENIED");
}

#[tokio::test]
async fn unmatched_v2_path_returns_oci_envelope() {
    let (_dir, _state, app) = harness().await;
    let response = send(
        &app,
        Method::GET,
        "/v2/darktohka/something/unknown/endpoint",
        Some(USER),
        &[],
        &[],
    )
    .await;
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    assert_api_version(&response);
    let json = body_json(response).await;
    assert_eq!(json["errors"][0]["code"], "NAME_UNKNOWN");
}

#[tokio::test]
async fn nested_repository_names_route_correctly() {
    let (_dir, _state, app) = harness().await;
    let name = "darktohka/more/complicated/project2";
    let content = b"nested-layer".to_vec();
    let digest = push_blob(&app, name, &content).await;

    let get = send(
        &app,
        Method::GET,
        &format!("/v2/{name}/blobs/{digest}"),
        Some(USER),
        &[],
        &[],
    )
    .await;
    assert_eq!(get.status(), StatusCode::OK);
    assert_eq!(body_bytes(get).await, content);

    let config = b"nested-config".to_vec();
    let layer = b"nested-layer-2".to_vec();
    let config_digest = push_blob(&app, name, &config).await;
    let layer_digest = push_blob(&app, name, &layer).await;
    let body = oci_manifest(&config_digest, config.len(), &layer_digest, layer.len());
    let manifest_digest =
        push_manifest(&app, name, "v2-tag", media_types::OCI_IMAGE_MANIFEST, &body).await;

    let get_manifest = send(
        &app,
        Method::GET,
        &format!("/v2/{name}/manifests/v2-tag"),
        Some(USER),
        &[("accept", media_types::OCI_IMAGE_MANIFEST)],
        &[],
    )
    .await;
    assert_eq!(get_manifest.status(), StatusCode::OK);
    assert_eq!(header_str(&get_manifest, "docker-content-digest"), manifest_digest);

    let tags = send(
        &app,
        Method::GET,
        &format!("/v2/{name}/tags/list"),
        Some(USER),
        &[],
        &[],
    )
    .await;
    assert_eq!(tags.status(), StatusCode::OK);
    let json = body_json(tags).await;
    assert_eq!(json["name"], name);
    assert_eq!(json["tags"], serde_json::json!(["v2-tag"]));
}

#[tokio::test]
async fn pulls_and_pushes_are_audited() {
    let (_dir, state, app) = harness().await;
    let content = b"audited-blob".to_vec();
    let digest = push_blob(&app, "darktohka/site", &content).await;
    let _ = send(
        &app,
        Method::GET,
        &format!("/v2/darktohka/site/blobs/{digest}"),
        Some(USER),
        &[],
        &[],
    )
    .await;

    let events: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM registry_events WHERE action = 'blob.pull'",
    )
    .fetch_one(&state.db)
    .await
    .expect("registry events");
    assert!(events >= 1);

    let pulls: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM pull_events")
        .fetch_one(&state.db)
        .await
        .expect("pull events");
    assert!(pulls >= 1);

    let stats: i64 = sqlx::query_scalar("SELECT COALESCE(SUM(pulls), 0) FROM pull_stats")
        .fetch_one(&state.db)
        .await
        .expect("pull stats");
    assert!(stats >= 1);

    let pushes: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM registry_events WHERE action = 'blob.push'",
    )
    .fetch_one(&state.db)
    .await
    .expect("push events");
    assert!(pushes >= 1);
}

async fn limited_harness(limit: u64) -> (tempfile::TempDir, AppState, Router) {
    let (dir, state) = crate::auth::test_support::test_state_with(|config| {
        config.max_blob_size = Some(limit);
    })
    .await;
    create_user(&state, USER).await;
    let app = crate::routes::build(state.clone());
    (dir, state, app)
}

#[tokio::test]
async fn blob_delete_removes_link_and_file() {
    let (_dir, _state, app) = harness().await;
    let content = b"deletable-blob".to_vec();
    let digest = push_blob(&app, "darktohka/site", &content).await;

    let delete = send(
        &app,
        Method::DELETE,
        &format!("/v2/darktohka/site/blobs/{digest}"),
        Some(USER),
        &[],
        &[],
    )
    .await;
    assert_eq!(delete.status(), StatusCode::ACCEPTED);
    assert_eq!(header_str(&delete, "content-length"), "0");

    let get = send(
        &app,
        Method::GET,
        &format!("/v2/darktohka/site/blobs/{digest}"),
        Some(USER),
        &[],
        &[],
    )
    .await;
    assert_eq!(get.status(), StatusCode::NOT_FOUND);
    let json = body_json(get).await;
    assert_eq!(json["errors"][0]["code"], "BLOB_UNKNOWN");
}

#[tokio::test]
async fn invalid_manifest_tag_is_rejected() {
    let (_dir, _state, app) = harness().await;
    let config = b"cfg".to_vec();
    let layer = b"lyr".to_vec();
    let config_digest = push_blob(&app, "darktohka/site", &config).await;
    let layer_digest = push_blob(&app, "darktohka/site", &layer).await;
    let body = oci_manifest(&config_digest, config.len(), &layer_digest, layer.len());
    let response = send(
        &app,
        Method::PUT,
        "/v2/darktohka/site/manifests/.hidden",
        Some(USER),
        &[("content-type", media_types::OCI_IMAGE_MANIFEST)],
        &body,
    )
    .await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let json = body_json(response).await;
    assert_eq!(json["errors"][0]["code"], "TAG_INVALID");
}

#[tokio::test]
async fn unknown_repository_read_is_not_found() {
    let (_dir, _state, app) = harness().await;
    let digest = Digest::from_bytes(b"whatever").to_string();
    let response = send(
        &app,
        Method::GET,
        &format!("/v2/darktohka/absent/blobs/{digest}"),
        Some(USER),
        &[],
        &[],
    )
    .await;
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    let json = body_json(response).await;
    assert_eq!(json["errors"][0]["code"], "NAME_UNKNOWN");
}

#[tokio::test]
async fn invalid_repository_name_is_rejected() {
    let (_dir, _state, app) = harness().await;
    let digest = Digest::from_bytes(b"x").to_string();
    let response = send(
        &app,
        Method::GET,
        &format!("/v2/darktohka/BadName/blobs/{digest}"),
        Some(USER),
        &[],
        &[],
    )
    .await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let json = body_json(response).await;
    assert_eq!(json["errors"][0]["code"], "NAME_INVALID");
}

#[tokio::test]
async fn blob_size_limit_is_enforced() {
    let (_dir, _state, app) = limited_harness(4).await;
    let start = send(
        &app,
        Method::POST,
        "/v2/darktohka/site/blobs/uploads/",
        Some(USER),
        &[],
        &[],
    )
    .await;
    assert_eq!(start.status(), StatusCode::ACCEPTED);
    let location = header_str(&start, "location");
    let response = send(
        &app,
        Method::PATCH,
        &location,
        Some(USER),
        &[("content-type", "application/octet-stream")],
        b"way too long",
    )
    .await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let json = body_json(response).await;
    assert_eq!(json["errors"][0]["code"], "SIZE_INVALID");
}
