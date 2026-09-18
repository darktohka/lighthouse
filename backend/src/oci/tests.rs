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
    let challenge = header_str(&anonymous, "www-authenticate");
    assert!(challenge.starts_with("Bearer realm="), "{challenge}");
    assert!(challenge.contains("/api/auth/token"), "{challenge}");
    assert!(challenge.contains("service="), "{challenge}");

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
        &format!("/v2/darktohka/target/blobs/uploads/?mount={digest}&from=darktohka/source"),
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
        &format!("/v2/darktohka/target/blobs/uploads/?mount={unknown}&from=darktohka/source"),
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
    let digest = push_manifest(
        &app,
        "darktohka/site",
        "latest",
        media_types::OCI_IMAGE_MANIFEST,
        &body,
    )
    .await;

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
    assert_eq!(
        header_str(&get, "content-type"),
        media_types::OCI_IMAGE_MANIFEST
    );
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

async fn restricted_service_account(
    state: &AppState,
    owner: &crate::models::User,
) -> (i64, String) {
    let (account, token) = crate::auth::service_accounts::create(
        state,
        crate::auth::service_accounts::NewServiceAccount {
            owner_user_id: owner.id,
            name: "ci",
            username: "owner-ci",
            description: None,
        },
    )
    .await
    .expect("service account");
    crate::auth::ip_ranges::add(&state.db, account.id, "10.0.0.0/8")
        .await
        .expect("range");
    (account.id, token)
}

#[tokio::test]
async fn password_grant_checks_the_service_account_ip_allowlist() {
    let (_dir, state, app) = harness().await;
    let user = create_user(&state, "owner").await;
    let (_account_id, token) = restricted_service_account(&state, &user).await;

    let denied = send(
        &app,
        Method::POST,
        "/api/auth/token",
        None,
        &[
            ("content-type", "application/x-www-form-urlencoded"),
            ("x-forwarded-for", "192.168.1.1"),
        ],
        format!("grant_type=password&username=owner-ci&password={token}&service=registry.local")
            .as_bytes(),
    )
    .await;
    assert_eq!(denied.status(), StatusCode::BAD_REQUEST);
    assert_eq!(
        body_json(denied).await["error"],
        "invalid_grant",
        "a form password grant from outside the allowlist is rejected like a bad password"
    );

    let allowed = send(
        &app,
        Method::POST,
        "/api/auth/token",
        None,
        &[
            ("content-type", "application/x-www-form-urlencoded"),
            ("x-forwarded-for", "10.1.2.3"),
        ],
        format!("grant_type=password&username=owner-ci&password={token}&service=registry.local")
            .as_bytes(),
    )
    .await;
    assert_eq!(allowed.status(), StatusCode::OK);
    assert!(body_json(allowed).await["token"].as_str().is_some());
}

#[tokio::test]
async fn refresh_redemption_checks_the_service_account_ip_allowlist() {
    let (_dir, state, app) = harness().await;
    let user = create_user(&state, "owner").await;
    let (_account_id, token) = restricted_service_account(&state, &user).await;

    let granted = send(
        &app,
        Method::POST,
        "/api/auth/token",
        None,
        &[
            ("content-type", "application/x-www-form-urlencoded"),
            ("x-forwarded-for", "10.1.2.3"),
        ],
        format!(
            "grant_type=password&username=owner-ci&password={token}&service=registry.local\
             &access_type=offline"
        )
        .as_bytes(),
    )
    .await;
    assert_eq!(granted.status(), StatusCode::OK);
    let refresh = body_json(granted).await["refresh_token"]
        .as_str()
        .expect("access_type=offline returns a refresh_token")
        .to_string();

    let denied = send(
        &app,
        Method::POST,
        "/api/auth/token",
        None,
        &[
            ("content-type", "application/x-www-form-urlencoded"),
            ("x-forwarded-for", "192.168.1.1"),
        ],
        format!("grant_type=refresh_token&refresh_token={refresh}&service=registry.local")
            .as_bytes(),
    )
    .await;
    assert_eq!(denied.status(), StatusCode::UNAUTHORIZED);
    assert!(
        header_str(&denied, "www-authenticate").starts_with("Basic "),
        "a redemption rejected by the IP allowlist re-challenges with Basic"
    );
    assert_eq!(body_json(denied).await["error"], "invalid_grant");

    let allowed = send(
        &app,
        Method::POST,
        "/api/auth/token",
        None,
        &[
            ("content-type", "application/x-www-form-urlencoded"),
            ("x-forwarded-for", "10.1.2.3"),
        ],
        format!("grant_type=refresh_token&refresh_token={refresh}&service=registry.local")
            .as_bytes(),
    )
    .await;
    assert_eq!(
        allowed.status(),
        StatusCode::OK,
        "the denied redemption neither consumes nor rotates the secret"
    );
    assert!(body_json(allowed).await["token"].as_str().is_some());
}

#[tokio::test]
async fn registry_token_from_a_denied_ip_is_demoted_to_anonymous() {
    let (_dir, state, app) = harness().await;
    let user = create_user(&state, "owner").await;
    let (_account_id, token) = restricted_service_account(&state, &user).await;

    let granted = send(
        &app,
        Method::POST,
        "/api/auth/token",
        None,
        &[
            ("content-type", "application/x-www-form-urlencoded"),
            ("x-forwarded-for", "10.1.2.3"),
        ],
        format!("grant_type=password&username=owner-ci&password={token}&service=registry.local")
            .as_bytes(),
    )
    .await;
    assert_eq!(granted.status(), StatusCode::OK);
    let bearer = body_json(granted).await["token"]
        .as_str()
        .expect("token")
        .to_string();

    let allowed = send(
        &app,
        Method::GET,
        "/v2/_catalog",
        None,
        &[
            ("authorization", &format!("Bearer {bearer}")),
            ("x-forwarded-for", "10.1.2.3"),
        ],
        &[],
    )
    .await;
    assert_eq!(allowed.status(), StatusCode::OK);

    let demoted = send(
        &app,
        Method::GET,
        "/v2/_catalog",
        None,
        &[
            ("authorization", &format!("Bearer {bearer}")),
            ("x-forwarded-for", "192.168.1.1"),
        ],
        &[],
    )
    .await;
    assert_eq!(
        demoted.status(),
        StatusCode::FORBIDDEN,
        "an out-of-range registry bearer token is demoted to an anonymous registry token"
    );
    assert!(
        demoted.headers().get("www-authenticate").is_none(),
        "a presented registry token is denied, not re-challenged"
    );
}

#[cfg(test)]
mod auth_flow {
    use super::*;
    use crate::auth::test_support::create_repository;

    async fn anonymous_token(app: &Router, scope: &str) -> String {
        let uri = format!("/api/auth/token?service=registry.local&scope={scope}");
        let response = send(app, Method::GET, &uri, None, &[], &[]).await;
        assert_eq!(response.status(), StatusCode::OK);
        let json = body_json(response).await;
        assert!(
            json.get("identity_token").is_none(),
            "the CLI must store credentials, not a token"
        );
        json["token"].as_str().expect("token").to_string()
    }

    async fn enable_totp(state: &AppState, user_id: i64) {
        crate::auth::two_factor::begin_setup(state, user_id, "Lighthouse")
            .await
            .expect("setup");
        let secret: String = sqlx::query_scalar("SELECT totp_secret FROM users WHERE id = ?")
            .bind(user_id)
            .fetch_one(&state.db)
            .await
            .expect("secret");
        let bytes = crate::auth::totp::base32_decode(&secret).expect("decode");
        let code = crate::auth::totp::format_code(crate::auth::totp::totp(
            &bytes,
            chrono::Utc::now().timestamp(),
        ));
        crate::auth::two_factor::verify_setup(state, user_id, &code, chrono::Utc::now())
            .await
            .expect("verify");
        crate::auth::two_factor::confirm_enable(state, user_id, None, chrono::Utc::now())
            .await
            .expect("enable");
    }

    fn basic_with(user: &str, password: &str) -> String {
        format!(
            "Basic {}",
            base64::engine::general_purpose::STANDARD.encode(format!("{user}:{password}"))
        )
    }

    async fn challenge_harness(
        mode: crate::config::RegistryAuthChallenge,
    ) -> (tempfile::TempDir, AppState, Router) {
        let (dir, state) = crate::auth::test_support::test_state_with(|config| {
            config.registry_auth_challenge = mode;
        })
        .await;
        create_user(&state, USER).await;
        let app = crate::routes::build(state.clone());
        (dir, state, app)
    }

    #[tokio::test]
    async fn offline_login_issues_a_reusable_refresh_token() {
        let (_dir, _state, app) = harness().await;

        let offline = send(
            &app,
            Method::GET,
            "/api/auth/token?service=registry.local&scope=repository:darktohka/public:pull\
             &offline_token=true&client_id=docker",
            Some(USER),
            &[],
            &[],
        )
        .await;
        assert_eq!(offline.status(), StatusCode::OK);
        let refresh = body_json(offline).await["refresh_token"]
            .as_str()
            .expect("offline login returns a refresh_token")
            .to_string();

        // The Docker daemon reuses the same identitytoken on every refresh, so
        // two consecutive exchanges with it must both succeed and the secret
        // must not change.
        for _ in 0..2 {
            let exchanged = send(
                &app,
                Method::POST,
                "/api/auth/token",
                None,
                &[("content-type", "application/x-www-form-urlencoded")],
                format!(
                    "grant_type=refresh_token&refresh_token={refresh}&service=registry.local\
                     &scope=repository:darktohka/public:pull&client_id=docker"
                )
                .as_bytes(),
            )
            .await;
            assert_eq!(exchanged.status(), StatusCode::OK);
            let body = body_json(exchanged).await;
            assert!(body["token"].as_str().is_some());
            assert!(
                body.get("refresh_token").is_none(),
                "the secret is stable, so no replacement is returned"
            );
        }

        let rejected = send(
            &app,
            Method::POST,
            "/api/auth/token",
            None,
            &[("content-type", "application/x-www-form-urlencoded")],
            b"grant_type=refresh_token&refresh_token=not-a-token&service=registry.local",
        )
        .await;
        assert_eq!(rejected.status(), StatusCode::UNAUTHORIZED);
        assert!(header_str(&rejected, "www-authenticate").starts_with("Basic "));
        assert_eq!(body_json(rejected).await["error"], "invalid_grant");
    }

    #[tokio::test]
    async fn password_grant_accepts_form_credentials() {
        let (_dir, _state, app) = harness().await;

        let response = send(
            &app,
            Method::POST,
            "/api/auth/token",
            None,
            &[("content-type", "application/x-www-form-urlencoded")],
            format!(
                "grant_type=password&username={USER}&password={PASSWORD}&service=registry.local\
                 &scope=repository:darktohka/public:pull&access_type=offline"
            )
            .as_bytes(),
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);
        let body = body_json(response).await;
        assert!(body["token"].as_str().is_some());
        assert!(
            body["refresh_token"].as_str().is_some(),
            "access_type=offline must return a refresh token"
        );

        let bad = send(
            &app,
            Method::POST,
            "/api/auth/token",
            None,
            &[("content-type", "application/x-www-form-urlencoded")],
            b"grant_type=password&username=darktohka&password=wrong&service=registry.local",
        )
        .await;
        assert_eq!(bad.status(), StatusCode::BAD_REQUEST);
        assert_eq!(body_json(bad).await["error"], "invalid_grant");
    }

    #[tokio::test]
    async fn challenge_mode_controls_the_advertised_scheme() {
        use crate::config::RegistryAuthChallenge;

        let (_dir, _state, app) = challenge_harness(RegistryAuthChallenge::Basic).await;
        let response = send(&app, Method::GET, "/v2/", None, &[], &[]).await;
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        assert!(header_str(&response, "www-authenticate").starts_with("Basic "));

        let (_dir, _state, app) = challenge_harness(RegistryAuthChallenge::Both).await;
        let response = send(&app, Method::GET, "/v2/", None, &[], &[]).await;
        let values: Vec<String> = response
            .headers()
            .get_all("www-authenticate")
            .iter()
            .filter_map(|value| value.to_str().ok())
            .map(str::to_string)
            .collect();
        assert_eq!(values.len(), 2, "both schemes are advertised separately");
        assert!(values[0].starts_with("Bearer "));
        assert!(values[1].starts_with("Basic "));

        let (_dir, _state, app) = harness().await;
        let response = send(&app, Method::GET, "/v2/", None, &[], &[]).await;
        assert!(header_str(&response, "www-authenticate").starts_with("Bearer "));
        assert_eq!(
            response
                .headers()
                .get_all("www-authenticate")
                .iter()
                .count(),
            1
        );
    }

    #[tokio::test]
    async fn anonymous_token_pulls_public_and_is_denied_on_private() {
        let (_dir, state, app) = harness().await;
        sqlx::query("UPDATE namespaces SET is_public = 1 WHERE name = ? COLLATE NOCASE")
            .bind(USER)
            .execute(&state.db)
            .await
            .expect("public namespace");

        let bob = create_user(&state, "bob").await;
        let bob_namespace: i64 =
            sqlx::query_scalar("SELECT id FROM namespaces WHERE name = 'bob' LIMIT 1")
                .fetch_one(&state.db)
                .await
                .expect("namespace");
        create_repository(&state, bob_namespace, "bob/private", false, Some(bob.id)).await;

        let config = b"cfg".to_vec();
        let layer = b"layer".to_vec();
        let config_digest = push_blob(&app, "darktohka/public", &config).await;
        let layer_digest = push_blob(&app, "darktohka/public", &layer).await;
        let body = oci_manifest(&config_digest, config.len(), &layer_digest, layer.len());
        push_manifest(
            &app,
            "darktohka/public",
            "latest",
            media_types::OCI_IMAGE_MANIFEST,
            &body,
        )
        .await;

        let denied = send(
            &app,
            Method::GET,
            "/v2/bob/private/manifests/latest",
            None,
            &[("accept", media_types::OCI_IMAGE_MANIFEST)],
            &[],
        )
        .await;
        assert_eq!(denied.status(), StatusCode::UNAUTHORIZED);
        assert!(
            header_str(&denied, "www-authenticate")
                .contains("scope=\"repository:bob/private:pull\"")
        );

        let token = anonymous_token(&app, "repository:darktohka/public:pull").await;

        let ping = send(
            &app,
            Method::GET,
            "/v2/",
            None,
            &[("authorization", &format!("Bearer {token}"))],
            &[],
        )
        .await;
        assert_eq!(ping.status(), StatusCode::OK);

        let pulled = send(
            &app,
            Method::GET,
            "/v2/darktohka/public/manifests/latest",
            None,
            &[
                ("authorization", &format!("Bearer {token}")),
                ("accept", media_types::OCI_IMAGE_MANIFEST),
            ],
            &[],
        )
        .await;
        assert_eq!(pulled.status(), StatusCode::OK);
        assert_eq!(body_bytes(pulled).await, body);

        let private = anonymous_token(&app, "repository:bob/private:pull").await;
        let forbidden = send(
            &app,
            Method::GET,
            "/v2/bob/private/manifests/latest",
            None,
            &[
                ("authorization", &format!("Bearer {private}")),
                ("accept", media_types::OCI_IMAGE_MANIFEST),
            ],
            &[],
        )
        .await;
        assert_eq!(
            forbidden.status(),
            StatusCode::FORBIDDEN,
            "a token was presented, so the registry denies instead of re-challenging"
        );
        assert!(forbidden.headers().get("www-authenticate").is_none());
    }

    #[tokio::test]
    async fn app_password_authenticates_basic_and_bypasses_totp() {
        let (_dir, state, app) = harness().await;
        let alice: i64 =
            sqlx::query_scalar("SELECT id FROM users WHERE username = ? COLLATE NOCASE")
                .bind(USER)
                .fetch_one(&state.db)
                .await
                .expect("user");
        enable_totp(&state, alice).await;

        let rejected = send(&app, Method::GET, "/v2/", Some(USER), &[], &[]).await;
        assert_eq!(
            rejected.status(),
            StatusCode::UNAUTHORIZED,
            "password alone cannot satisfy two-factor authentication"
        );

        let (_, secret) = crate::auth::app_passwords::create(&state, alice, "ci")
            .await
            .expect("app password");
        let credentials = basic_with(USER, &secret);
        let accepted = send(
            &app,
            Method::GET,
            "/v2/",
            None,
            &[("authorization", &credentials)],
            &[],
        )
        .await;
        assert_eq!(accepted.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn registry_tokens_cannot_authenticate_web_endpoints() {
        let (_dir, state, app) = harness().await;
        let alice: i64 =
            sqlx::query_scalar("SELECT id FROM users WHERE username = ? COLLATE NOCASE")
                .bind(USER)
                .fetch_one(&state.db)
                .await
                .expect("user");
        let (_, secret) = crate::auth::app_passwords::create(&state, alice, "ci")
            .await
            .expect("app password");

        let response = send(
            &app,
            Method::GET,
            "/api/auth/token?service=registry.local&scope=repository:darktohka/site:pull,push",
            None,
            &[("authorization", &basic_with(USER, &secret))],
            &[],
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);
        let token = body_json(response).await["token"]
            .as_str()
            .expect("token")
            .to_string();

        let me = send(
            &app,
            Method::GET,
            "/api/auth/me",
            None,
            &[("authorization", &format!("Bearer {token}"))],
            &[],
        )
        .await;
        assert_eq!(
            me.status(),
            StatusCode::UNAUTHORIZED,
            "a registry token must not act as a web session"
        );
    }

    #[tokio::test]
    async fn token_endpoint_rejects_invalid_basic_credentials() {
        let (_dir, _state, app) = harness().await;
        let response = send(
            &app,
            Method::GET,
            "/api/auth/token?service=registry.local&scope=repository:darktohka/site:pull",
            None,
            &[("authorization", &basic_with(USER, "not-the-password"))],
            &[],
        )
        .await;
        assert_eq!(
            response.status(),
            StatusCode::UNAUTHORIZED,
            "docker login must not falsely succeed"
        );
        assert!(header_str(&response, "www-authenticate").starts_with("Basic realm="));
    }
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
    let digest = push_manifest(
        &app,
        "darktohka/site",
        "v1",
        media_types::OCI_IMAGE_MANIFEST,
        &body,
    )
    .await;

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
    push_manifest(
        &app,
        "darktohka/site",
        "multi",
        media_types::OCI_IMAGE_INDEX,
        &index,
    )
    .await;

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
    assert_eq!(
        header_str(&get, "content-type"),
        media_types::OCI_IMAGE_INDEX
    );
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
        push_manifest(
            &app,
            "darktohka/site",
            tag,
            media_types::OCI_IMAGE_MANIFEST,
            &body,
        )
        .await;
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
    assert!(header_str(&anonymous, "www-authenticate").contains("Bearer realm="));

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
async fn catalog_hides_repositories_the_actor_cannot_pull() {
    let (_dir, state, app) = harness().await;
    push_blob(&app, "darktohka/alpha", b"a").await;
    sqlx::query("UPDATE namespaces SET is_public = 0 WHERE name = ? COLLATE NOCASE")
        .bind(USER)
        .execute(&state.db)
        .await
        .expect("private namespace");
    create_user(&state, "bob").await;

    let owner = send(&app, Method::GET, "/v2/_catalog", Some(USER), &[], &[]).await;
    assert_eq!(owner.status(), StatusCode::OK);
    assert_eq!(
        body_json(owner).await["repositories"],
        serde_json::json!(["darktohka/alpha"])
    );

    let stranger = send(&app, Method::GET, "/v2/_catalog", Some("bob"), &[], &[]).await;
    assert_eq!(stranger.status(), StatusCode::OK);
    assert_eq!(
        body_json(stranger).await["repositories"],
        serde_json::json!([])
    );
}

#[tokio::test]
async fn private_repository_requires_authentication() {
    let (_dir, state, app) = harness().await;
    sqlx::query("UPDATE namespaces SET is_public = 0 WHERE name = ? COLLATE NOCASE")
        .bind(USER)
        .execute(&state.db)
        .await
        .expect("private namespace");
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
    let challenge = header_str(&anonymous, "www-authenticate");
    assert!(challenge.contains("Bearer realm="), "{challenge}");
    assert!(
        challenge.contains("scope=\"repository:darktohka/site:pull\""),
        "{challenge}"
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
    assert_eq!(
        header_str(&get_manifest, "docker-content-digest"),
        manifest_digest
    );

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

    let events: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM registry_events WHERE action = 'blob.pull'")
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

    let pushes: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM registry_events WHERE action = 'blob.push'")
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
