//! Integration tests for the control-plane API over a real temp SQLite
//! database and a temp `DATA_DIR`, seeded through the `Registry` service.

use std::io::Write;

use axum::Router;
use axum::body::Body;
use axum::http::{Method, Request, StatusCode};
use serde_json::{Value, json};
use tower::ServiceExt;

use crate::auth::test_support::{body_json, create_user, test_state};
use crate::models::User;
use crate::oci::digest::Digest;
use crate::oci::media_types;
use crate::state::{AppState, AuthContext};

fn actor(user: &User) -> AuthContext {
    AuthContext {
        user_id: Some(user.id),
        username: Some(user.username.clone()),
        service_account_id: None,
        is_admin: false,
    }
}

async fn harness() -> (tempfile::TempDir, AppState, Router) {
    let (dir, state) = test_state().await;
    let app = crate::routes::build(state.clone());
    (dir, state, app)
}

async fn call(
    app: &Router,
    method: Method,
    uri: &str,
    actor: Option<AuthContext>,
    body: Option<Value>,
) -> axum::response::Response {
    let mut builder = Request::builder().method(method).uri(uri);
    if let Some(actor) = actor {
        builder = builder.extension(actor);
    }
    let request = match body {
        Some(value) => builder
            .header("content-type", "application/json")
            .body(Body::from(value.to_string()))
            .expect("request"),
        None => builder.body(Body::empty()).expect("request"),
    };
    app.clone().oneshot(request).await.expect("response")
}

async fn body_bytes(response: axum::response::Response) -> Vec<u8> {
    axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body")
        .to_vec()
}

// ---------------------------------------------------------------------------
// Seeding
// ---------------------------------------------------------------------------

async fn push_blob(state: &AppState, repo_id: i64, bytes: &[u8], media_type: &str) -> Digest {
    let digest = Digest::from_bytes(bytes);
    let tmp = state
        .storage
        .tmp_dir()
        .join(format!("seed-{}.bin", digest.encoded()));
    tokio::fs::write(&tmp, bytes).await.expect("write blob");
    state
        .storage
        .put_blob_from_path(&tmp, &digest)
        .await
        .expect("put blob");
    state
        .registry
        .register_blob(&digest, bytes.len() as u64, Some(media_type))
        .await
        .expect("register blob");
    state
        .registry
        .link_blob(repo_id, &digest)
        .await
        .expect("link blob");
    digest
}

fn image_manifest(config: &str, config_size: i64, layer: &str, layer_size: i64) -> Vec<u8> {
    serde_json::json!({
        "schemaVersion": 2,
        "mediaType": media_types::OCI_IMAGE_MANIFEST,
        "config": {
            "mediaType": media_types::OCI_IMAGE_CONFIG,
            "digest": config,
            "size": config_size,
        },
        "layers": [{
            "mediaType": media_types::OCI_IMAGE_LAYER_GZIP,
            "digest": layer,
            "size": layer_size,
        }],
    })
    .to_string()
    .into_bytes()
}

async fn seed_image(
    state: &AppState,
    repo_name: &str,
    tag: &str,
    config: &[u8],
    layer: &[u8],
) -> (Digest, Digest) {
    let repo = state
        .registry
        .ensure_repository(repo_name)
        .await
        .expect("repository");
    let config_digest = push_blob(state, repo.id, config, media_types::OCI_IMAGE_CONFIG).await;
    let layer_digest = push_blob(state, repo.id, layer, media_types::OCI_IMAGE_LAYER_GZIP).await;
    let manifest = image_manifest(
        &config_digest.to_string(),
        config.len() as i64,
        &layer_digest.to_string(),
        layer.len() as i64,
    );
    let manifest_digest = Digest::from_bytes(&manifest);
    state
        .registry
        .put_manifest(repo.id, media_types::OCI_IMAGE_MANIFEST, &manifest)
        .await
        .expect("put manifest");
    state
        .registry
        .set_tag(repo.id, tag, &manifest_digest)
        .await
        .expect("set tag");
    (manifest_digest, layer_digest)
}

fn add_file<W: Write>(builder: &mut tar::Builder<W>, path: &str, data: &[u8], mode: u32) {
    let mut header = tar::Header::new_gnu();
    header.set_path(path).expect("path");
    header.set_size(data.len() as u64);
    header.set_mode(mode);
    header.set_cksum();
    builder.append(&header, data).expect("append file");
}

fn add_symlink<W: Write>(builder: &mut tar::Builder<W>, path: &str, target: &str) {
    let mut header = tar::Header::new_gnu();
    header.set_entry_type(tar::EntryType::Symlink);
    header.set_path(path).expect("path");
    header.set_link_name(target).expect("link target");
    header.set_size(0);
    header.set_mode(0o777);
    header.set_cksum();
    builder.append(&header, &[][..]).expect("append symlink");
}

/// A real gzip-compressed tar with nested files, a symlink and whiteouts.
fn layer_archive() -> Vec<u8> {
    let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    {
        let mut builder = tar::Builder::new(&mut encoder);
        add_file(&mut builder, "etc/config.txt", b"hello layer\n", 0o644);
        add_file(&mut builder, "usr/bin/run", b"#!/bin/sh\n", 0o755);
        add_file(&mut builder, "top.txt", b"root file\n", 0o644);
        add_file(&mut builder, "var/a.txt", b"old\n", 0o644);
        add_file(&mut builder, "var/.wh..wh..opq", b"", 0o644);
        add_file(&mut builder, "var/b.txt", b"new\n", 0o644);
        add_symlink(&mut builder, "usr/bin/link", "run");
        add_file(&mut builder, ".wh.top.txt", b"", 0o644);
        builder.finish().expect("finish tar");
    }
    encoder.finish().expect("finish gzip")
}

// ---------------------------------------------------------------------------
// Repositories and tags
// ---------------------------------------------------------------------------

#[tokio::test]
async fn repository_detail_tags_patch_and_delete() {
    let (_dir, state, app) = harness().await;
    let alice = create_user(&state, "alice").await;
    let (manifest_digest, _) = seed_image(
        &state,
        "alice/more/complicated/app",
        "v1",
        b"{\"cfg\":1}",
        b"layer-one",
    )
    .await;

    let response = call(
        &app,
        Method::GET,
        "/api/repositories/alice/more/complicated/app",
        Some(actor(&alice)),
        None,
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let detail = body_json(response).await;
    assert_eq!(detail["name"], "alice/more/complicated/app");
    assert_eq!(detail["namespace"], "alice");
    assert_eq!(detail["path"], "more/complicated/app");
    assert_eq!(detail["tag_count"], 1);
    assert_eq!(detail["total_size"], detail["size"]);

    let response = call(
        &app,
        Method::GET,
        "/api/repositories/alice/more/complicated/app/tags",
        Some(actor(&alice)),
        None,
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let tags = body_json(response).await;
    assert_eq!(tags["total"], 1);
    assert_eq!(tags["items"][0]["name"], "v1");
    assert_eq!(tags["items"][0]["digest"], manifest_digest.to_string());

    let response = call(
        &app,
        Method::GET,
        "/api/repositories/alice/more/complicated/app/tags/v1",
        Some(actor(&alice)),
        None,
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let tag = body_json(response).await;
    assert!(tag["manifest"].is_object());
    assert!(tag["layers"].is_array());
    assert!(
        tag["layers"]
            .as_array()
            .expect("layers")
            .iter()
            .any(|layer| layer["role"] == "layer")
    );

    let response = call(
        &app,
        Method::PATCH,
        "/api/repositories/alice/more/complicated/app",
        Some(actor(&alice)),
        Some(json!({ "description": "nested", "is_public": true })),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let patched = body_json(response).await;
    assert_eq!(patched["description"], "nested");
    assert_eq!(patched["is_public"], true);

    let response = call(
        &app,
        Method::DELETE,
        "/api/repositories/alice/more/complicated/app/tags/v1",
        Some(actor(&alice)),
        None,
    )
    .await;
    assert_eq!(response.status(), StatusCode::NO_CONTENT);

    let response = call(
        &app,
        Method::DELETE,
        "/api/repositories/alice/more/complicated/app",
        Some(actor(&alice)),
        None,
    )
    .await;
    assert_eq!(response.status(), StatusCode::NO_CONTENT);
    assert!(
        state
            .registry
            .find_repository("alice/more/complicated/app")
            .await
            .expect("find")
            .is_none()
    );
}

#[tokio::test]
async fn unique_and_shared_sizes_when_tags_share_layer() {
    let (_dir, state, app) = harness().await;
    let alice = create_user(&state, "alice").await;

    let config_a = b"config-aaaa";
    let config_b = b"config-bbbbbbbbbbbbbb";
    let layer = b"shared-layer-bytes-0123456789";
    seed_image(&state, "alice/app", "a", config_a, layer).await;
    seed_image(&state, "alice/app", "b", config_b, layer).await;

    let response = call(
        &app,
        Method::GET,
        "/api/repositories/alice/app",
        Some(actor(&alice)),
        None,
    )
    .await;
    let detail = body_json(response).await;
    let total = (config_a.len() + config_b.len() + layer.len()) as i64;
    let unique = (config_a.len() + config_b.len()) as i64;
    assert_eq!(detail["total_size"], total);
    assert_eq!(detail["unique_size"], unique);
    assert_eq!(detail["shared_size"], layer.len() as i64);
    assert_eq!(detail["tag_count"], 2);

    let response = call(
        &app,
        Method::GET,
        "/api/tags?namespace=alice&sort=unique_size&order=desc",
        Some(actor(&alice)),
        None,
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let ranked = body_json(response).await;
    assert_eq!(ranked["total"], 2);
    let mut uniques: Vec<i64> = ranked["items"]
        .as_array()
        .expect("items")
        .iter()
        .map(|item| item["unique_size"].as_i64().unwrap_or(0))
        .collect();
    uniques.sort_unstable();
    let mut expected = vec![config_a.len() as i64, config_b.len() as i64];
    expected.sort_unstable();
    assert_eq!(uniques, expected);
    for item in ranked["items"].as_array().expect("items") {
        assert_eq!(
            item["total_size"].as_i64().unwrap_or(0) - item["unique_size"].as_i64().unwrap_or(0),
            layer.len() as i64
        );
    }
}

#[tokio::test]
async fn global_tag_ranking_and_batch_delete() {
    let (_dir, state, app) = harness().await;
    let alice = create_user(&state, "alice").await;
    seed_image(&state, "alice/app", "a", b"config-a", b"layer-one").await;
    seed_image(&state, "alice/app", "b", b"config-b", b"layer-two").await;

    let response = call(
        &app,
        Method::GET,
        "/api/tags?sort=total_size&order=desc",
        Some(actor(&alice)),
        None,
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let ranked = body_json(response).await;
    assert_eq!(ranked["total"], 2);

    let response = call(
        &app,
        Method::POST,
        "/api/repositories/alice/app/tags/batch-delete",
        Some(actor(&alice)),
        Some(json!({ "tags": ["a", "b"] })),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let result = body_json(response).await;
    assert_eq!(result["deleted"], 2);

    let response = call(
        &app,
        Method::GET,
        "/api/repositories/alice/app/tags",
        Some(actor(&alice)),
        None,
    )
    .await;
    let tags = body_json(response).await;
    assert_eq!(tags["total"], 0);
}

// ---------------------------------------------------------------------------
// Visibility
// ---------------------------------------------------------------------------

#[tokio::test]
async fn private_repository_is_404_for_strangers() {
    let (_dir, state, app) = harness().await;
    let alice = create_user(&state, "alice").await;
    let bob = create_user(&state, "bob").await;
    seed_image(&state, "alice/secret", "latest", b"config", b"layer").await;

    let owner = call(
        &app,
        Method::GET,
        "/api/repositories/alice/secret",
        Some(actor(&alice)),
        None,
    )
    .await;
    assert_eq!(owner.status(), StatusCode::OK);

    let stranger = call(
        &app,
        Method::GET,
        "/api/repositories/alice/secret",
        Some(actor(&bob)),
        None,
    )
    .await;
    assert_eq!(stranger.status(), StatusCode::NOT_FOUND);

    let anonymous = call(&app, Method::GET, "/api/repositories/alice/secret", None, None).await;
    assert_eq!(anonymous.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn namespace_list_only_includes_visible_workspaces() {
    let (_dir, state, app) = harness().await;
    let alice = create_user(&state, "alice").await;
    let bob = create_user(&state, "bob").await;

    let response = call(
        &app,
        Method::POST,
        "/api/namespaces",
        Some(actor(&alice)),
        Some(json!({ "name": "team-alice" })),
    )
    .await;
    assert_eq!(response.status(), StatusCode::CREATED);

    let response = call(&app, Method::GET, "/api/namespaces", Some(actor(&alice)), None).await;
    let mine = body_json(response).await;
    assert!(
        mine["items"]
            .as_array()
            .expect("items")
            .iter()
            .any(|item| item["name"] == "team-alice")
    );

    let response = call(&app, Method::GET, "/api/namespaces", Some(actor(&bob)), None).await;
    let theirs = body_json(response).await;
    assert!(
        !theirs["items"]
            .as_array()
            .expect("items")
            .iter()
            .any(|item| item["name"] == "team-alice")
    );
}

// ---------------------------------------------------------------------------
// Layer browser
// ---------------------------------------------------------------------------

#[tokio::test]
async fn layer_tree_file_and_path_traversal_safety() {
    let (_dir, state, app) = harness().await;
    let alice = create_user(&state, "alice").await;
    let layer = layer_archive();
    let (_, layer_digest) =
        seed_image(&state, "alice/img", "latest", b"config", &layer).await;
    let digest = layer_digest.to_string();
    let tree_uri = format!("/api/repositories/alice/img/layers/{digest}/tree");
    let file_uri = format!("/api/repositories/alice/img/layers/{digest}/file");

    let response = call(&app, Method::GET, &tree_uri, Some(actor(&alice)), None).await;
    assert_eq!(response.status(), StatusCode::OK);
    let tree = body_json(response).await;
    let names: Vec<String> = tree
        .as_array()
        .expect("tree")
        .iter()
        .map(|entry| entry["name"].as_str().unwrap_or_default().to_string())
        .collect();
    assert!(names.contains(&"etc".to_string()));
    assert!(names.contains(&"usr".to_string()));
    assert!(!names.contains(&"top.txt".to_string()), "whiteout must hide top.txt");

    let response = call(
        &app,
        Method::GET,
        &format!("{tree_uri}?path=etc"),
        Some(actor(&alice)),
        None,
    )
    .await;
    let etc = body_json(response).await;
    let entries = etc.as_array().expect("etc");
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0]["name"], "config.txt");
    assert_eq!(entries[0]["kind"], "file");
    assert_eq!(entries[0]["size"], 12);

    let response = call(
        &app,
        Method::GET,
        &format!("{tree_uri}?path=var"),
        Some(actor(&alice)),
        None,
    )
    .await;
    let var = body_json(response).await;
    let var_names: Vec<String> = var
        .as_array()
        .expect("var")
        .iter()
        .map(|entry| entry["name"].as_str().unwrap_or_default().to_string())
        .collect();
    assert_eq!(var_names, vec!["b.txt".to_string()], "opaque dir hides a.txt");

    let response = call(
        &app,
        Method::GET,
        &format!("{file_uri}?path=etc/config.txt"),
        Some(actor(&alice)),
        None,
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let content_type = response
        .headers()
        .get("content-type")
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default()
        .to_string();
    assert!(content_type.starts_with("text/plain"));
    let content = body_bytes(response).await;
    assert_eq!(content, b"hello layer\n");

    let response = call(
        &app,
        Method::GET,
        &format!("{file_uri}?path=usr/bin/link"),
        Some(actor(&alice)),
        None,
    )
    .await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);

    let response = call(
        &app,
        Method::GET,
        &format!("{file_uri}?path=../etc/config.txt"),
        Some(actor(&alice)),
        None,
    )
    .await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);

    let response = call(
        &app,
        Method::GET,
        &format!("{tree_uri}?path=/etc"),
        Some(actor(&alice)),
        None,
    )
    .await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);

    let response = call(
        &app,
        Method::GET,
        &format!("{file_uri}?path=etc/missing"),
        Some(actor(&alice)),
        None,
    )
    .await;
    assert_eq!(response.status(), StatusCode::NOT_FOUND);

    let download_uri = format!("/api/repositories/alice/img/layers/{digest}/download");
    let response = call(&app, Method::GET, &download_uri, Some(actor(&alice)), None).await;
    assert_eq!(response.status(), StatusCode::OK);
    let downloaded = body_bytes(response).await;
    assert_eq!(downloaded, layer);
}

// ---------------------------------------------------------------------------
// Permissions and service accounts
// ---------------------------------------------------------------------------

#[tokio::test]
async fn permission_crud_and_user_autocomplete() {
    let (_dir, state, app) = harness().await;
    let alice = create_user(&state, "alice").await;
    create_user(&state, "bob").await;

    let response = call(
        &app,
        Method::POST,
        "/api/namespaces",
        Some(actor(&alice)),
        Some(json!({ "name": "team-alice" })),
    )
    .await;
    assert_eq!(response.status(), StatusCode::CREATED);

    let response = call(
        &app,
        Method::POST,
        "/api/namespaces/team-alice/permissions",
        Some(actor(&alice)),
        Some(json!({ "subject_type": "user", "subject": "bob", "can_push": true })),
    )
    .await;
    assert_eq!(response.status(), StatusCode::CREATED);
    let grant = body_json(response).await;
    assert_eq!(grant["can_pull"], true, "push implies pull");
    assert_eq!(grant["can_push"], true);
    assert_eq!(grant["subject"]["username"], "bob");
    let grant_id = grant["id"].as_i64().expect("grant id");

    let response = call(
        &app,
        Method::POST,
        "/api/namespaces/team-alice/permissions",
        Some(actor(&alice)),
        Some(json!({ "subject_type": "user", "subject": "bob", "can_push": true })),
    )
    .await;
    assert_eq!(response.status(), StatusCode::CONFLICT);

    let response = call(
        &app,
        Method::GET,
        "/api/namespaces/team-alice/permissions",
        Some(actor(&alice)),
        None,
    )
    .await;
    let grants = body_json(response).await;
    assert_eq!(grants.as_array().expect("grants").len(), 1);

    let response = call(
        &app,
        Method::GET,
        "/api/users/search?q=bo",
        Some(actor(&alice)),
        None,
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let results = body_json(response).await;
    let first = &results.as_array().expect("search")[0];
    assert_eq!(first["username"], "bob");
    assert!(first.get("email").is_none(), "search must never expose e-mail");

    let response = call(
        &app,
        Method::DELETE,
        &format!("/api/namespaces/team-alice/permissions/{grant_id}"),
        Some(actor(&alice)),
        None,
    )
    .await;
    assert_eq!(response.status(), StatusCode::NO_CONTENT);

    let response = call(
        &app,
        Method::GET,
        "/api/namespaces/team-alice/permissions",
        Some(actor(&alice)),
        None,
    )
    .await;
    let grants = body_json(response).await;
    assert!(grants.as_array().expect("grants").is_empty());
}

#[tokio::test]
async fn service_account_create_rotate_and_grants() {
    let (_dir, state, app) = harness().await;
    let alice = create_user(&state, "alice").await;

    let response = call(
        &app,
        Method::POST,
        "/api/service-accounts",
        Some(actor(&alice)),
        Some(json!({ "name": "ci", "username": "alice-ci" })),
    )
    .await;
    assert_eq!(response.status(), StatusCode::CREATED);
    let created = body_json(response).await;
    let token = created["token"].as_str().expect("token").to_string();
    assert!(token.starts_with("lhr_"));
    let account_id = created["account"]["id"].as_i64().expect("account id");
    assert_eq!(created["account"]["token_prefix"].as_str().unwrap().len(), 3);

    let response = call(
        &app,
        Method::GET,
        "/api/service-accounts",
        Some(actor(&alice)),
        None,
    )
    .await;
    let list = body_json(response).await;
    let rendered = list.to_string();
    assert!(!rendered.contains(&token), "listing must not expose the token");
    assert!(!rendered.contains("token_hash"));
    assert_eq!(list.as_array().expect("list").len(), 1);

    let response = call(
        &app,
        Method::POST,
        &format!("/api/service-accounts/{account_id}/token"),
        Some(actor(&alice)),
        None,
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let rotated = body_json(response).await;
    let rotated_token = rotated["token"].as_str().expect("rotated token");
    assert_ne!(rotated_token, token);

    let response = call(
        &app,
        Method::POST,
        &format!("/api/service-accounts/{account_id}/grants"),
        Some(actor(&alice)),
        Some(json!({ "namespace": "alice", "can_push": true })),
    )
    .await;
    assert_eq!(response.status(), StatusCode::CREATED);
    let grant = body_json(response).await;
    assert_eq!(grant["can_pull"], true);
    let grant_id = grant["id"].as_i64().expect("grant id");

    let response = call(
        &app,
        Method::GET,
        &format!("/api/service-accounts/{account_id}"),
        Some(actor(&alice)),
        None,
    )
    .await;
    let detail = body_json(response).await;
    assert_eq!(detail["grants"].as_array().expect("grants").len(), 1);

    let response = call(
        &app,
        Method::DELETE,
        &format!("/api/service-accounts/{account_id}/grants/{grant_id}"),
        Some(actor(&alice)),
        None,
    )
    .await;
    assert_eq!(response.status(), StatusCode::NO_CONTENT);

    let response = call(
        &app,
        Method::DELETE,
        &format!("/api/service-accounts/{account_id}"),
        Some(actor(&alice)),
        None,
    )
    .await;
    assert_eq!(response.status(), StatusCode::NO_CONTENT);

    let grants: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM service_account_grants")
        .fetch_one(&state.db)
        .await
        .expect("grants counted");
    assert_eq!(grants, 0, "deleting an account cascades its grants");
}

// ---------------------------------------------------------------------------
// Analytics, activity and dashboard
// ---------------------------------------------------------------------------

#[tokio::test]
async fn analytics_overview_aggregates_expected_totals() {
    let (_dir, state, app) = harness().await;
    let alice = create_user(&state, "alice").await;
    seed_image(&state, "alice/app", "a", b"config-a", b"layer-one").await;
    seed_image(&state, "alice/app", "b", b"config-b", b"layer-two").await;

    let repo_id = state
        .registry
        .find_repository("alice/app")
        .await
        .expect("find")
        .expect("repository")
        .id;
    sqlx::query("INSERT INTO pull_stats (repository_id, tag_name, day, pulls) VALUES (?, 'a', ?, 7)")
        .bind(repo_id)
        .bind(chrono::Utc::now().format("%Y-%m-%d").to_string())
        .execute(&state.db)
        .await
        .expect("pull stat");

    let response = call(
        &app,
        Method::GET,
        "/api/analytics/overview",
        Some(actor(&alice)),
        None,
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let overview = body_json(response).await;
    assert!(overview["total_size"].as_i64().unwrap_or(0) > 0);
    assert!(overview["unique_size"].as_i64().unwrap_or(0) > 0);
    assert_eq!(overview["repository_count"], 1);
    assert_eq!(overview["tag_count"], 2);
    assert!(overview["blob_count"].as_i64().unwrap_or(0) >= 4);
    assert_eq!(overview["pull_count"], 7);
    assert_eq!(overview["pull_count_30d"], 7);
    assert!(!overview["largest_tags"].as_array().expect("largest").is_empty());
    assert!(!overview["disk_usage_by_repository"]
        .as_array()
        .expect("disk")
        .is_empty());
    let shared = overview["shared_size"].as_i64().unwrap_or(0);
    let total = overview["total_size"].as_i64().unwrap_or(1);
    let percentage = overview["shared_percentage"].as_f64().unwrap_or(-1.0);
    assert!((percentage - (shared as f64 / total as f64 * 100.0)).abs() < 0.001);
}

#[tokio::test]
async fn activity_and_dashboard_for_seeded_user() {
    let (_dir, state, app) = harness().await;
    let alice = create_user(&state, "alice").await;
    seed_image(&state, "alice/app", "latest", b"config", b"layer").await;

    let response = call(
        &app,
        Method::PATCH,
        "/api/repositories/alice/app",
        Some(actor(&alice)),
        Some(json!({ "description": "updated", "is_public": true })),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);

    let response = call(
        &app,
        Method::GET,
        "/api/activity/me",
        Some(actor(&alice)),
        None,
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let activity = body_json(response).await;
    assert!(
        activity["items"]
            .as_array()
            .expect("items")
            .iter()
            .any(|item| item["kind"] == "repository.updated")
    );

    let response = call(&app, Method::GET, "/api/activity", None, None).await;
    assert_eq!(response.status(), StatusCode::OK);
    let public_feed = body_json(response).await;
    assert!(
        public_feed["items"]
            .as_array()
            .expect("items")
            .iter()
            .any(|item| item["repository"] == "alice/app")
    );

    let response = call(&app, Method::GET, "/api/dashboard", Some(actor(&alice)), None).await;
    assert_eq!(response.status(), StatusCode::OK);
    let dashboard = body_json(response).await;
    assert!(!dashboard["repositories"]
        .as_array()
        .expect("repositories")
        .is_empty());
    assert_eq!(dashboard["stats"]["repository_count"], 1);
    assert_eq!(dashboard["stats"]["tag_count"], 1);
    assert!(dashboard["stats"]["total_size"].as_i64().unwrap_or(0) > 0);
}

#[tokio::test]
async fn raw_blob_and_manifest_endpoints() {
    let (_dir, state, app) = harness().await;
    let alice = create_user(&state, "alice").await;
    let bob = create_user(&state, "bob").await;

    let config = br#"{"os":"linux","architecture":"amd64"}"#.to_vec();
    let (manifest_digest, layer_digest) =
        seed_image(&state, "alice/app", "latest", &config, b"layer-bytes").await;
    let config_digest = Digest::from_bytes(&config).to_string();
    let manifest = manifest_digest.to_string();

    let response = call(
        &app,
        Method::GET,
        &format!("/api/blobs/{config_digest}"),
        Some(actor(&alice)),
        None,
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response
            .headers()
            .get("content-type")
            .and_then(|value| value.to_str().ok()),
        Some("application/octet-stream")
    );
    assert_eq!(body_bytes(response).await, config);

    let response = call(
        &app,
        Method::GET,
        &format!("/api/blobs/{config_digest}/json"),
        Some(actor(&alice)),
        None,
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let json = body_json(response).await;
    assert_eq!(json["os"], "linux");
    assert_eq!(json["architecture"], "amd64");

    let response = call(
        &app,
        Method::GET,
        &format!("/api/blobs/{layer_digest}"),
        Some(actor(&bob)),
        None,
    )
    .await;
    assert_eq!(response.status(), StatusCode::NOT_FOUND);

    let response = call(
        &app,
        Method::GET,
        &format!("/api/repositories/alice/app/manifests/{manifest}"),
        Some(actor(&alice)),
        None,
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let raw = body_json(response).await;
    assert_eq!(raw["schemaVersion"], 2);

    let response = call(
        &app,
        Method::GET,
        &format!("/api/repositories/alice/app/manifests/{manifest}/references"),
        Some(actor(&alice)),
        None,
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let references = body_json(response).await;
    let roles: Vec<String> = references
        .as_array()
        .expect("references")
        .iter()
        .map(|reference| reference["role"].as_str().unwrap_or_default().to_string())
        .collect();
    assert!(roles.contains(&"config".to_string()));
    assert!(roles.contains(&"layer".to_string()));

    let response = call(
        &app,
        Method::GET,
        &format!("/api/repositories/alice/app/manifests/{manifest}"),
        Some(actor(&bob)),
        None,
    )
    .await;
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn user_profile_and_heatmap() {
    let (_dir, state, app) = harness().await;
    let alice = create_user(&state, "alice").await;

    let response = call(
        &app,
        Method::GET,
        "/api/users/alice",
        Some(actor(&alice)),
        None,
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let profile = body_json(response).await;
    assert_eq!(profile["username"], "alice");
    assert_eq!(profile["is_self"], true);
    assert!(profile.get("email").is_none(), "profiles never expose e-mail");
    assert!(profile.get("password_hash").is_none());

    let year = chrono::Utc::now().format("%Y").to_string();
    let response = call(
        &app,
        Method::GET,
        &format!("/api/users/alice/heatmap?year={year}"),
        Some(actor(&alice)),
        None,
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let heatmap = body_json(response).await;
    assert_eq!(heatmap["year"].as_i64().unwrap_or(0).to_string(), year);
    assert!(!heatmap["days"].as_array().expect("days").is_empty());
}
