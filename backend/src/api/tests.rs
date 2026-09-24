//! Integration tests for the control-plane API over a real temp SQLite
//! database and a temp `DATA_DIR`, seeded through the `Registry` service.

use std::io::Write;

use axum::Router;
use axum::body::Body;
use axum::http::{Method, Request, StatusCode};
use serde_json::{Value, json};
use tower::ServiceExt;

use crate::auth::test_support::{body_json, create_user, test_state, test_state_with};
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
        ..AuthContext::default()
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

fn image_manifest(
    config: &str,
    config_size: i64,
    layer: &str,
    layer_size: i64,
    layer_media_type: &str,
) -> Vec<u8> {
    serde_json::json!({
        "schemaVersion": 2,
        "mediaType": media_types::OCI_IMAGE_MANIFEST,
        "config": {
            "mediaType": media_types::OCI_IMAGE_CONFIG,
            "digest": config,
            "size": config_size,
        },
        "layers": [{
            "mediaType": layer_media_type,
            "digest": layer,
            "size": layer_size,
        }],
    })
    .to_string()
    .into_bytes()
}

/// An OCI image manifest with an explicit, ordered layer list.
fn image_manifest_layers(
    config: &str,
    config_size: i64,
    layers: &[(String, i64, String)],
) -> Vec<u8> {
    let descriptors: Vec<Value> = layers
        .iter()
        .map(|(digest, size, media_type)| {
            json!({ "mediaType": media_type, "digest": digest, "size": size })
        })
        .collect();
    json!({
        "schemaVersion": 2,
        "mediaType": media_types::OCI_IMAGE_MANIFEST,
        "config": {
            "mediaType": media_types::OCI_IMAGE_CONFIG,
            "digest": config,
            "size": config_size,
        },
        "layers": descriptors,
    })
    .to_string()
    .into_bytes()
}

/// Seeds an image with several gzip layers in order and returns its manifest
/// digest plus the ordered layer digests.
async fn seed_multilayer_image(
    state: &AppState,
    repo_name: &str,
    tag: &str,
    layers: &[Vec<u8>],
) -> (Digest, Vec<Digest>) {
    let repo = state
        .registry
        .ensure_repository(repo_name)
        .await
        .expect("repository");
    let config = b"{\"cfg\":1}";
    let config_digest = push_blob(state, repo.id, config, media_types::OCI_IMAGE_CONFIG).await;
    let mut digests = Vec::new();
    let mut descriptors = Vec::new();
    for layer in layers {
        let digest = push_blob(state, repo.id, layer, media_types::OCI_IMAGE_LAYER_GZIP).await;
        descriptors.push((
            digest.to_string(),
            layer.len() as i64,
            media_types::OCI_IMAGE_LAYER_GZIP.to_string(),
        ));
        digests.push(digest);
    }
    let manifest = image_manifest_layers(
        &config_digest.to_string(),
        config.len() as i64,
        &descriptors,
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
    (manifest_digest, digests)
}

/// Seeds a multi-platform index over already-stored child image manifests.
async fn seed_index(state: &AppState, repo_name: &str, tag: &str, children: &[Digest]) -> Digest {
    let repo = state
        .registry
        .ensure_repository(repo_name)
        .await
        .expect("repository");
    let descriptors: Vec<Value> = children
        .iter()
        .map(|digest| {
            json!({
                "mediaType": media_types::OCI_IMAGE_MANIFEST,
                "digest": digest.to_string(),
                "size": 0,
                "platform": { "os": "linux", "architecture": "amd64" },
            })
        })
        .collect();
    let manifest = json!({
        "schemaVersion": 2,
        "mediaType": media_types::OCI_IMAGE_INDEX,
        "manifests": descriptors,
    })
    .to_string()
    .into_bytes();
    let digest = Digest::from_bytes(&manifest);
    state
        .registry
        .put_manifest(repo.id, media_types::OCI_IMAGE_INDEX, &manifest)
        .await
        .expect("put index");
    state
        .registry
        .set_tag(repo.id, tag, &digest)
        .await
        .expect("set index tag");
    digest
}

async fn seed_image(
    state: &AppState,
    repo_name: &str,
    tag: &str,
    config: &[u8],
    layer: &[u8],
) -> (Digest, Digest) {
    seed_image_typed(
        state,
        repo_name,
        tag,
        config,
        layer,
        media_types::OCI_IMAGE_LAYER_GZIP,
    )
    .await
}

/// Seeds an image whose single layer is declared with `layer_media_type`. The
/// bytes are stored as given, so the declared type can intentionally disagree
/// with the archive's actual compression.
async fn seed_image_typed(
    state: &AppState,
    repo_name: &str,
    tag: &str,
    config: &[u8],
    layer: &[u8],
    layer_media_type: &str,
) -> (Digest, Digest) {
    let repo = state
        .registry
        .ensure_repository(repo_name)
        .await
        .expect("repository");
    let config_digest = push_blob(state, repo.id, config, media_types::OCI_IMAGE_CONFIG).await;
    let layer_digest = push_blob(state, repo.id, layer, layer_media_type).await;
    let manifest = image_manifest(
        &config_digest.to_string(),
        config.len() as i64,
        &layer_digest.to_string(),
        layer.len() as i64,
        layer_media_type,
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
        add_symlink(&mut builder, "root-link", "usr");
        add_file(&mut builder, ".wh.top.txt", b"", 0o644);
        builder.finish().expect("finish tar");
    }
    encoder.finish().expect("finish gzip")
}

/// A real uncompressed tar, mirroring the entries of [`layer_archive`] but with
/// no compression layer.
fn plain_layer_archive() -> Vec<u8> {
    let mut builder = tar::Builder::new(Vec::new());
    add_file(&mut builder, "etc/config.txt", b"hello layer\n", 0o644);
    add_file(&mut builder, "usr/bin/run", b"#!/bin/sh\n", 0o755);
    builder.finish().expect("finish tar");
    builder.into_inner().expect("tar bytes")
}

/// A real zstd-compressed tar with the same layout as [`plain_layer_archive`].
fn zstd_layer_archive() -> Vec<u8> {
    let mut encoder = zstd::stream::write::Encoder::new(Vec::new(), 3).expect("zstd encoder");
    {
        let mut builder = tar::Builder::new(&mut encoder);
        add_file(&mut builder, "etc/config.txt", b"hello layer\n", 0o644);
        add_file(&mut builder, "usr/bin/run", b"#!/bin/sh\n", 0o755);
        builder.finish().expect("finish tar");
    }
    encoder.finish().expect("finish zstd")
}

/// One entry of a synthetic gzip layer archive.
enum LayerEntry<'a> {
    File(&'a str, &'a [u8], u32),
    Symlink(&'a str, &'a str),
}

/// Builds a gzip-compressed tar from a concise, ordered entry list. `.wh.*`
/// paths are ordinary files whose names encode the whiteout.
fn layer_archive_with(entries: &[LayerEntry<'_>]) -> Vec<u8> {
    let mut builder = tar::Builder::new(Vec::new());
    for entry in entries {
        match entry {
            LayerEntry::File(path, data, mode) => add_file(&mut builder, path, data, *mode),
            LayerEntry::Symlink(path, target) => add_symlink(&mut builder, path, target),
        }
    }
    builder.finish().expect("finish tar");
    let tar = builder.into_inner().expect("tar bytes");
    let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    encoder.write_all(&tar).expect("gzip write");
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
    assert_eq!(detail["can_pull"], true);
    assert_eq!(detail["can_push"], true);

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
    assert_eq!(tag["can_pull"], true);
    assert_eq!(tag["can_push"], true);
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
async fn tag_list_supports_server_side_sorting() {
    let (_dir, state, app) = harness().await;
    let alice = create_user(&state, "alice").await;
    seed_image(&state, "alice/app", "alpha", b"{\"cfg\":1}", b"aa").await;
    seed_image(&state, "alice/app", "beta", b"{\"cfg\":1}", b"bbbbbbbbbbbb").await;
    seed_image(&state, "alice/app", "gamma", b"{\"cfg\":1}", b"ggggggg").await;

    fn tag_names(page: &Value) -> Vec<String> {
        page["items"]
            .as_array()
            .expect("items")
            .iter()
            .map(|item| item["name"].as_str().expect("name").to_string())
            .collect()
    }

    let default = body_json(
        call(
            &app,
            Method::GET,
            "/api/repositories/alice/app/tags",
            Some(actor(&alice)),
            None,
        )
        .await,
    )
    .await;
    assert_eq!(default["total"], 3);
    assert_eq!(tag_names(&default), ["alpha", "beta", "gamma"]);

    let name_desc = body_json(
        call(
            &app,
            Method::GET,
            "/api/repositories/alice/app/tags?sort=name&order=desc",
            Some(actor(&alice)),
            None,
        )
        .await,
    )
    .await;
    assert_eq!(tag_names(&name_desc), ["gamma", "beta", "alpha"]);

    let size_desc = body_json(
        call(
            &app,
            Method::GET,
            "/api/repositories/alice/app/tags?sort=compressed_size&order=desc",
            Some(actor(&alice)),
            None,
        )
        .await,
    )
    .await;
    assert_eq!(tag_names(&size_desc), ["beta", "gamma", "alpha"]);
    let sizes: Vec<i64> = size_desc["items"]
        .as_array()
        .expect("items")
        .iter()
        .map(|item| item["compressed_size"].as_i64().expect("compressed_size"))
        .collect();
    assert!(sizes.windows(2).all(|window| window[0] > window[1]));

    let bogus = call(
        &app,
        Method::GET,
        "/api/repositories/alice/app/tags?sort=bogus",
        Some(actor(&alice)),
        None,
    )
    .await;
    assert_eq!(bogus.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn detail_reports_effective_actor_access() {
    let (_dir, state, app) = harness().await;
    let alice = create_user(&state, "alice").await;
    let bob = create_user(&state, "bob").await;
    seed_image(&state, "alice/app", "v1", b"{\"cfg\":1}", b"layer-one").await;

    let detail = body_json(
        call(
            &app,
            Method::GET,
            "/api/repositories/alice/app",
            Some(actor(&alice)),
            None,
        )
        .await,
    )
    .await;
    assert_eq!(detail["can_pull"], true);
    assert_eq!(detail["can_push"], true);

    let response = call(
        &app,
        Method::POST,
        "/api/repositories/alice/app/permissions",
        Some(actor(&alice)),
        Some(json!({ "subject_type": "user", "subject": "bob", "can_push": false })),
    )
    .await;
    assert_eq!(response.status(), StatusCode::CREATED);

    let detail = body_json(
        call(
            &app,
            Method::GET,
            "/api/repositories/alice/app",
            Some(actor(&bob)),
            None,
        )
        .await,
    )
    .await;
    assert_eq!(detail["can_pull"], true);
    assert_eq!(detail["can_push"], false);

    let tag = body_json(
        call(
            &app,
            Method::GET,
            "/api/repositories/alice/app/tags/v1",
            Some(actor(&bob)),
            None,
        )
        .await,
    )
    .await;
    assert_eq!(tag["can_pull"], true);
    assert_eq!(tag["can_push"], false);

    let response = call(
        &app,
        Method::PATCH,
        "/api/repositories/alice/app",
        Some(actor(&alice)),
        Some(json!({ "is_public": true })),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);

    let detail =
        body_json(call(&app, Method::GET, "/api/repositories/alice/app", None, None).await).await;
    assert_eq!(detail["can_pull"], true);
    assert_eq!(detail["can_push"], false);
}

#[tokio::test]
async fn create_tagless_repository_endpoint() {
    let (_dir, state, app) = harness().await;
    let alice = create_user(&state, "alice").await;
    let bob = create_user(&state, "bob").await;

    let response = call(
        &app,
        Method::POST,
        "/api/namespaces/alice/repositories",
        Some(actor(&alice)),
        Some(json!({ "name": "brand-new", "description": "  fresh  ", "is_public": false })),
    )
    .await;
    assert_eq!(response.status(), StatusCode::CREATED);
    let detail = body_json(response).await;
    assert_eq!(detail["name"], "alice/brand-new");
    assert_eq!(detail["path"], "brand-new");
    assert_eq!(detail["description"], "fresh");
    assert_eq!(detail["is_public"], false);
    assert_eq!(detail["tag_count"], 0);
    assert_eq!(detail["can_push"], true);

    let inherited = call(
        &app,
        Method::POST,
        "/api/namespaces/alice/repositories",
        Some(actor(&alice)),
        Some(json!({ "name": "inherits-namespace" })),
    )
    .await;
    assert_eq!(inherited.status(), StatusCode::CREATED);
    let inherited = body_json(inherited).await;
    assert_eq!(inherited["is_public"], true);

    let duplicate = call(
        &app,
        Method::POST,
        "/api/namespaces/alice/repositories",
        Some(actor(&alice)),
        Some(json!({ "name": "brand-new" })),
    )
    .await;
    assert_eq!(duplicate.status(), StatusCode::CONFLICT);

    let invalid = call(
        &app,
        Method::POST,
        "/api/namespaces/alice/repositories",
        Some(actor(&alice)),
        Some(json!({ "name": "Bad/Name" })),
    )
    .await;
    assert_eq!(invalid.status(), StatusCode::BAD_REQUEST);

    let empty = call(
        &app,
        Method::POST,
        "/api/namespaces/alice/repositories",
        Some(actor(&alice)),
        Some(json!({ "name": "  " })),
    )
    .await;
    assert_eq!(empty.status(), StatusCode::BAD_REQUEST);

    let denied = call(
        &app,
        Method::POST,
        "/api/namespaces/alice/repositories",
        Some(actor(&bob)),
        Some(json!({ "name": "intruder" })),
    )
    .await;
    assert_eq!(denied.status(), StatusCode::NOT_FOUND);
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
    // The repository reaches every blob once and owns all of them: tag `a`
    // introduced the shared layer, so nothing is shared across images.
    let total = (config_a.len() + config_b.len() + layer.len()) as i64;
    assert_eq!(detail["total_size"], total);
    assert_eq!(detail["unique_size"], total);
    assert_eq!(detail["shared_size"], 0);
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
    for item in ranked["items"].as_array().expect("items") {
        let total = item["total_size"].as_i64().unwrap_or(0);
        let unique = item["unique_size"].as_i64().unwrap_or(0);
        let shared = item["shared_size"].as_i64().unwrap_or(0);
        assert_eq!(shared, total - unique);
        match item["tag"].as_str().unwrap_or_default() {
            "a" => {
                // The first tag owns the layer outright.
                assert_eq!(unique, (config_a.len() + layer.len()) as i64);
                assert_eq!(shared, 0);
            }
            "b" => {
                // The later tag counts the already-owned layer as shared.
                assert_eq!(unique, config_b.len() as i64);
                assert_eq!(shared, layer.len() as i64);
            }
            other => panic!("unexpected tag {other}"),
        }
    }
    assert_eq!(ranked["items"][0]["tag"], "a", "owner ranks highest first");
}

#[tokio::test]
async fn cross_repository_sharing_is_owned_by_the_first_pusher() {
    let (_dir, state, app) = harness().await;
    let alice = create_user(&state, "alice").await;

    let config_one = b"config-one";
    let config_two = b"config-two-cccccccc";
    let layer = b"shared-layer-bytes-0123456789";
    seed_image(&state, "alice/one", "v1", config_one, layer).await;
    seed_image(&state, "alice/two", "v1", config_two, layer).await;

    let one = body_json(
        call(
            &app,
            Method::GET,
            "/api/repositories/alice/one",
            Some(actor(&alice)),
            None,
        )
        .await,
    )
    .await;
    assert_eq!(one["unique_size"], (config_one.len() + layer.len()) as i64);
    assert_eq!(one["shared_size"], 0);

    let two = body_json(
        call(
            &app,
            Method::GET,
            "/api/repositories/alice/two",
            Some(actor(&alice)),
            None,
        )
        .await,
    )
    .await;
    assert_eq!(two["unique_size"], config_two.len() as i64);
    assert_eq!(two["shared_size"], layer.len() as i64);

    let overview = body_json(
        call(
            &app,
            Method::GET,
            "/api/analytics/overview",
            Some(actor(&alice)),
            None,
        )
        .await,
    )
    .await;
    let logical = (config_one.len() + config_two.len() + 2 * layer.len()) as i64;
    assert_eq!(overview["total_size"], logical);
    assert_eq!(
        overview["unique_size"],
        (config_one.len() + config_two.len() + layer.len()) as i64
    );
    assert_eq!(overview["shared_size"], layer.len() as i64);
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
    sqlx::query("UPDATE namespaces SET is_public = 0 WHERE name = 'alice' COLLATE NOCASE")
        .execute(&state.db)
        .await
        .expect("private namespace");
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

    let anonymous = call(
        &app,
        Method::GET,
        "/api/repositories/alice/secret",
        None,
        None,
    )
    .await;
    assert_eq!(anonymous.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn hidden_repository_is_unlisted_for_anonymous() {
    let (_dir, state, app) = harness().await;
    let alice = create_user(&state, "alice").await;
    seed_image(
        &state,
        "alice/visible",
        "v1",
        b"{\"visible\":1}",
        b"layer-visible",
    )
    .await;
    seed_image(
        &state,
        "alice/hidden",
        "v1",
        b"{\"hidden\":1}",
        b"layer-hidden",
    )
    .await;
    sqlx::query("UPDATE repositories SET is_hidden = 1 WHERE name = 'alice/hidden' COLLATE NOCASE")
        .execute(&state.db)
        .await
        .expect("hide repository");

    let hidden = call(
        &app,
        Method::GET,
        "/api/repositories/alice/hidden",
        None,
        None,
    )
    .await;
    assert_eq!(hidden.status(), StatusCode::NOT_FOUND);

    let visible = call(
        &app,
        Method::GET,
        "/api/repositories/alice/visible",
        None,
        None,
    )
    .await;
    assert_eq!(visible.status(), StatusCode::OK);

    let anonymous_list = body_json(
        call(
            &app,
            Method::GET,
            "/api/namespaces/alice/repositories",
            None,
            None,
        )
        .await,
    )
    .await;
    assert_eq!(anonymous_list["total"], 1);
    let items = anonymous_list["items"].as_array().expect("items");
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["name"], "alice/visible");

    let owner_list = body_json(
        call(
            &app,
            Method::GET,
            "/api/namespaces/alice/repositories",
            Some(actor(&alice)),
            None,
        )
        .await,
    )
    .await;
    assert_eq!(owner_list["total"], 2);

    let hidden_tags = call(
        &app,
        Method::GET,
        "/api/repositories/alice/hidden/tags",
        None,
        None,
    )
    .await;
    assert_eq!(hidden_tags.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn hiding_a_repository_persists_via_patch() {
    let (_dir, state, app) = harness().await;
    let alice = create_user(&state, "alice").await;
    seed_image(
        &state,
        "alice/visible",
        "v1",
        b"{\"cfg\":1}",
        b"layer-patch",
    )
    .await;

    let response = call(
        &app,
        Method::PATCH,
        "/api/repositories/alice/visible",
        Some(actor(&alice)),
        Some(json!({ "is_hidden": true })),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let patched = body_json(response).await;
    assert_eq!(patched["is_hidden"], true);
    assert_eq!(
        patched["is_public"], true,
        "hiding does not change visibility"
    );

    let hidden = call(
        &app,
        Method::GET,
        "/api/repositories/alice/visible",
        None,
        None,
    )
    .await;
    assert_eq!(hidden.status(), StatusCode::NOT_FOUND);

    let response = call(
        &app,
        Method::PATCH,
        "/api/repositories/alice/visible",
        Some(actor(&alice)),
        Some(json!({ "is_hidden": false })),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let unhidden = body_json(response).await;
    assert_eq!(unhidden["is_hidden"], false);

    let visible = call(
        &app,
        Method::GET,
        "/api/repositories/alice/visible",
        None,
        None,
    )
    .await;
    assert_eq!(visible.status(), StatusCode::OK);
}

async fn hide_repository(state: &AppState, name: &str) {
    sqlx::query("UPDATE repositories SET is_hidden = 1 WHERE name = ? COLLATE NOCASE")
        .bind(name)
        .execute(&state.db)
        .await
        .expect("hide repository");
}

async fn repository_id(state: &AppState, name: &str) -> i64 {
    state
        .registry
        .find_repository(name)
        .await
        .expect("find")
        .expect("repository")
        .id
}

async fn insert_activity(state: &AppState, actor_user_id: Option<i64>, repo_name: &str) {
    let repo_id = repository_id(state, repo_name).await;
    sqlx::query(
        "INSERT INTO activity \
         (actor_user_id, namespace_id, repository_id, kind, summary, metadata, is_public, created_at) \
         VALUES (?, (SELECT namespace_id FROM repositories WHERE id = ?), ?, 'tag.pushed', \
                 'Pushed tag latest', '{\"tag\":\"latest\"}', 1, ?)",
    )
    .bind(actor_user_id)
    .bind(repo_id)
    .bind(repo_id)
    .bind(chrono::Utc::now())
    .execute(&state.db)
    .await
    .expect("activity");
}

fn repository_names(value: &Value) -> Vec<String> {
    value["items"]
        .as_array()
        .expect("items")
        .iter()
        .filter_map(|item| item["repository"].as_str().map(str::to_string))
        .collect()
}

fn feed_mentions(value: &Value, needle: &str) -> bool {
    value["items"]
        .as_array()
        .expect("items")
        .iter()
        .any(|item| {
            item["summary"]
                .as_str()
                .is_some_and(|summary| summary.contains(needle))
                || item["metadata"]["repository"].as_str() == Some(needle)
        })
}

#[tokio::test]
async fn hidden_repository_visible_only_to_explicit_access() {
    let (_dir, state, app) = harness().await;
    let alice = create_user(&state, "alice").await;
    let bob = create_user(&state, "bob").await;
    seed_image(&state, "alice/visible", "v1", b"{\"v\":1}", b"layer-v").await;
    seed_image(&state, "alice/hidden", "v1", b"{\"h\":1}", b"layer-h").await;
    hide_repository(&state, "alice/hidden").await;

    let anon = body_json(
        call(
            &app,
            Method::GET,
            "/api/namespaces/alice/repositories",
            None,
            None,
        )
        .await,
    )
    .await;
    assert_eq!(anon["total"], 1);

    let stranger = body_json(
        call(
            &app,
            Method::GET,
            "/api/namespaces/alice/repositories",
            Some(actor(&bob)),
            None,
        )
        .await,
    )
    .await;
    assert_eq!(stranger["total"], 1);
    assert_eq!(stranger["items"][0]["name"], "alice/visible");

    let stranger_detail = call(
        &app,
        Method::GET,
        "/api/repositories/alice/hidden",
        Some(actor(&bob)),
        None,
    )
    .await;
    assert_eq!(stranger_detail.status(), StatusCode::NOT_FOUND);

    let owner = body_json(
        call(
            &app,
            Method::GET,
            "/api/namespaces/alice/repositories",
            Some(actor(&alice)),
            None,
        )
        .await,
    )
    .await;
    assert_eq!(owner["total"], 2);

    let hidden_id = repository_id(&state, "alice/hidden").await;
    sqlx::query(
        "INSERT INTO repository_permissions \
         (repository_id, subject_type, subject_user_id, can_pull, can_push, created_by, created_at) \
         VALUES (?, 'user', ?, 1, 0, NULL, ?)",
    )
    .bind(hidden_id)
    .bind(bob.id)
    .bind(chrono::Utc::now())
    .execute(&state.db)
    .await
    .expect("grant");

    let granted_detail = call(
        &app,
        Method::GET,
        "/api/repositories/alice/hidden",
        Some(actor(&bob)),
        None,
    )
    .await;
    assert_eq!(granted_detail.status(), StatusCode::OK);

    let granted = body_json(
        call(
            &app,
            Method::GET,
            "/api/namespaces/alice/repositories",
            Some(actor(&bob)),
            None,
        )
        .await,
    )
    .await;
    assert_eq!(granted["total"], 2);
}

#[tokio::test]
async fn namespace_with_only_hidden_repositories_is_not_enumerated() {
    let (_dir, state, app) = harness().await;
    let alice = create_user(&state, "alice").await;
    let bob = create_user(&state, "bob").await;
    seed_image(&state, "alice/hidden", "v1", b"{\"h\":1}", b"layer-h").await;
    hide_repository(&state, "alice/hidden").await;

    for viewer in [None, Some(actor(&bob))] {
        let listing =
            body_json(call(&app, Method::GET, "/api/namespaces", viewer, None).await).await;
        let names: Vec<String> = listing["items"]
            .as_array()
            .expect("items")
            .iter()
            .map(|item| item["name"].as_str().unwrap_or_default().to_string())
            .collect();
        assert!(!names.contains(&"alice".to_string()));

        let detail = call(&app, Method::GET, "/api/namespaces/alice", None, None).await;
        assert_eq!(detail.status(), StatusCode::NOT_FOUND);
    }

    let owner = body_json(
        call(
            &app,
            Method::GET,
            "/api/namespaces",
            Some(actor(&alice)),
            None,
        )
        .await,
    )
    .await;
    assert!(
        owner["items"]
            .as_array()
            .expect("items")
            .iter()
            .any(|item| item["name"] == "alice")
    );
}

#[tokio::test]
async fn analytics_hides_hidden_repositories_from_public_callers() {
    let (_dir, state, app) = harness().await;
    let alice = create_user(&state, "alice").await;
    seed_image(&state, "alice/visible", "v1", b"{\"v\":1}", b"layer-v").await;
    seed_image(&state, "alice/hidden", "v1", b"{\"h\":1}", b"layer-h").await;
    hide_repository(&state, "alice/hidden").await;

    let anon =
        body_json(call(&app, Method::GET, "/api/analytics/overview", None, None).await).await;
    let anon_repos: Vec<String> = anon["disk_usage_by_repository"]
        .as_array()
        .expect("disk")
        .iter()
        .map(|entry| entry["repository"].as_str().unwrap_or_default().to_string())
        .collect();
    assert!(anon_repos.contains(&"alice/visible".to_string()));
    assert!(!anon_repos.contains(&"alice/hidden".to_string()));

    let owner = body_json(
        call(
            &app,
            Method::GET,
            "/api/analytics/overview",
            Some(actor(&alice)),
            None,
        )
        .await,
    )
    .await;
    let owner_repos: Vec<String> = owner["disk_usage_by_repository"]
        .as_array()
        .expect("disk")
        .iter()
        .map(|entry| entry["repository"].as_str().unwrap_or_default().to_string())
        .collect();
    assert!(owner_repos.contains(&"alice/hidden".to_string()));
}

#[tokio::test]
async fn activity_feed_hides_repositories_the_caller_cannot_see() {
    let (_dir, state, app) = harness().await;
    let alice = create_user(&state, "alice").await;
    seed_image(&state, "alice/public", "v1", b"{\"p\":1}", b"layer-p").await;
    seed_image(&state, "alice/secret", "v1", b"{\"s\":1}", b"layer-s").await;
    seed_image(&state, "alice/hidden", "v1", b"{\"h\":1}", b"layer-h").await;
    sqlx::query("UPDATE repositories SET is_public = 0 WHERE name = 'alice/secret' COLLATE NOCASE")
        .execute(&state.db)
        .await
        .expect("private repository");
    hide_repository(&state, "alice/hidden").await;

    insert_activity(&state, Some(alice.id), "alice/public").await;
    insert_activity(&state, Some(alice.id), "alice/secret").await;
    insert_activity(&state, Some(alice.id), "alice/hidden").await;
    insert_activity(&state, None, "alice/hidden").await;

    let anon =
        body_json(call(&app, Method::GET, "/api/activity?per_page=100", None, None).await).await;
    let anon_repos = repository_names(&anon);
    assert!(anon_repos.contains(&"alice/public".to_string()));
    assert!(!anon_repos.contains(&"alice/secret".to_string()));
    assert!(!anon_repos.contains(&"alice/hidden".to_string()));

    let owner = body_json(
        call(
            &app,
            Method::GET,
            "/api/activity?per_page=100",
            Some(actor(&alice)),
            None,
        )
        .await,
    )
    .await;
    let owner_repos = repository_names(&owner);
    assert!(owner_repos.contains(&"alice/public".to_string()));
    assert!(owner_repos.contains(&"alice/secret".to_string()));
    assert!(owner_repos.contains(&"alice/hidden".to_string()));
}

#[tokio::test]
async fn activity_feed_hides_deleted_hidden_repository_names() {
    let (_dir, state, app) = harness().await;
    let alice = create_user(&state, "alice").await;
    seed_image(&state, "alice/public", "v1", b"{\"p\":1}", b"layer-p").await;

    sqlx::query(
        "INSERT INTO activity \
         (actor_user_id, namespace_id, repository_id, kind, summary, metadata, is_public, created_at) \
         VALUES (NULL, (SELECT id FROM namespaces WHERE name = 'alice' COLLATE NOCASE), NULL, \
                 'manifest.deleted', 'deleted repository alice/ghost', \
                 '{\"repository\":\"alice/ghost\"}', 0, ?)",
    )
    .bind(chrono::Utc::now())
    .execute(&state.db)
    .await
    .expect("activity");

    let anon =
        body_json(call(&app, Method::GET, "/api/activity?per_page=100", None, None).await).await;
    assert!(!feed_mentions(&anon, "alice/ghost"));

    let owner = body_json(
        call(&app, Method::GET, "/api/activity?per_page=100", Some(actor(&alice)), None).await,
    )
    .await;
    assert!(feed_mentions(&owner, "alice/ghost"));
}

#[tokio::test]
async fn access_token_is_rejected_after_session_revocation() {
    let (_dir, state, app) = harness().await;
    let alice = create_user(&state, "alice").await;
    let (session, _refresh) = crate::auth::sessions::create(
        &state,
        &alice,
        crate::auth::sessions::SessionAudit::default(),
    )
    .await
    .expect("session");
    let token = crate::auth::tokens::issue_access_token(
        &state.config,
        alice.id,
        &alice.username,
        &session.id,
    )
    .expect("token");
    let cookie = format!("{}={token}", state.config.cookie_name);

    let request = Request::builder()
        .method(Method::GET)
        .uri("/api/auth/me")
        .header("cookie", &cookie)
        .body(Body::empty())
        .expect("request");
    let response = app.clone().oneshot(request).await.expect("response");
    assert_eq!(response.status(), StatusCode::OK);

    crate::auth::sessions::revoke(&state, &session.id)
        .await
        .expect("revoke");

    let request = Request::builder()
        .method(Method::GET)
        .uri("/api/auth/me")
        .header("cookie", &cookie)
        .body(Body::empty())
        .expect("request");
    let response = app.clone().oneshot(request).await.expect("response");
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
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

    let response = call(
        &app,
        Method::GET,
        "/api/namespaces",
        Some(actor(&alice)),
        None,
    )
    .await;
    let mine = body_json(response).await;
    assert!(
        mine["items"]
            .as_array()
            .expect("items")
            .iter()
            .any(|item| item["name"] == "team-alice")
    );

    let response = call(
        &app,
        Method::GET,
        "/api/namespaces",
        Some(actor(&bob)),
        None,
    )
    .await;
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
    let (_, layer_digest) = seed_image(&state, "alice/img", "latest", b"config", &layer).await;
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
    assert!(
        !names.contains(&"top.txt".to_string()),
        "whiteout must hide top.txt"
    );

    let size_of = |entries: &serde_json::Value, name: &str| -> i64 {
        entries
            .as_array()
            .expect("tree")
            .iter()
            .find(|entry| entry["name"] == name)
            .and_then(|entry| entry["size"].as_i64())
            .unwrap_or(-1)
    };
    let entry_of = |entries: &serde_json::Value, name: &str| -> serde_json::Value {
        entries
            .as_array()
            .expect("tree")
            .iter()
            .find(|entry| entry["name"] == name)
            .cloned()
            .expect("entry present")
    };
    assert_eq!(size_of(&tree, "etc"), 12, "directory rolls up its file");
    assert_eq!(size_of(&tree, "usr"), 10, "nested directory total");
    assert_eq!(
        size_of(&tree, "var"),
        4,
        "opaque dir keeps only surviving file"
    );

    let root_link = entry_of(&tree, "root-link");
    assert_eq!(root_link["kind"], "symlink");
    assert_eq!(root_link["link_kind"], "dir");
    assert_eq!(root_link["link_resolved"], "usr");

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
    assert_eq!(
        var_names,
        vec!["b.txt".to_string()],
        "opaque dir hides a.txt"
    );

    let response = call(
        &app,
        Method::GET,
        &format!("{tree_uri}?path=usr"),
        Some(actor(&alice)),
        None,
    )
    .await;
    let usr = body_json(response).await;
    assert_eq!(size_of(&usr, "bin"), 10, "nested directory total");

    let response = call(
        &app,
        Method::GET,
        &format!("{tree_uri}?path=usr/bin"),
        Some(actor(&alice)),
        None,
    )
    .await;
    let usr_bin = body_json(response).await;
    let link = entry_of(&usr_bin, "link");
    assert_eq!(link["kind"], "symlink");
    assert_eq!(link["link_kind"], "file");
    assert_eq!(link["link_resolved"], "usr/bin/run");

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
    let disposition = response
        .headers()
        .get("content-disposition")
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default()
        .to_string();
    assert_eq!(
        disposition,
        format!(
            "attachment; filename=\"sha256-{}.tar.gz\"",
            layer_digest.encoded()
        )
    );
    let downloaded = body_bytes(response).await;
    assert_eq!(downloaded, layer);
}

#[tokio::test]
async fn layer_tree_rejects_a_layer_over_the_scan_byte_cap() {
    let (_dir, state) = test_state_with(|config| config.layer_max_scan_bytes = 1).await;
    let app = crate::routes::build(state.clone());
    let alice = create_user(&state, "alice").await;
    let layer = layer_archive();
    let (_, layer_digest) = seed_image(&state, "alice/img", "latest", b"config", &layer).await;
    let tree_uri = format!("/api/repositories/alice/img/layers/{layer_digest}/tree");

    let response = call(&app, Method::GET, &tree_uri, Some(actor(&alice)), None).await;
    assert_eq!(response.status(), StatusCode::PAYLOAD_TOO_LARGE);
}

#[tokio::test]
async fn layer_tree_rejects_a_layer_over_the_entry_cap() {
    let (_dir, state) = test_state_with(|config| config.layer_max_entries = 1).await;
    let app = crate::routes::build(state.clone());
    let alice = create_user(&state, "alice").await;
    let layer = layer_archive();
    let (_, layer_digest) = seed_image(&state, "alice/img", "latest", b"config", &layer).await;
    let tree_uri = format!("/api/repositories/alice/img/layers/{layer_digest}/tree");

    let response = call(&app, Method::GET, &tree_uri, Some(actor(&alice)), None).await;
    assert_eq!(response.status(), StatusCode::PAYLOAD_TOO_LARGE);
}

#[tokio::test]
async fn layer_tree_serves_from_cache_and_invalidation_forces_a_rescan() {
    let (_dir, state, app) = harness().await;
    let alice = create_user(&state, "alice").await;
    let layer = layer_archive();
    let (_, layer_digest) = seed_image(&state, "alice/img", "latest", b"config", &layer).await;
    let digest = layer_digest.to_string();
    let tree_uri = format!("/api/repositories/alice/img/layers/{digest}/tree");

    let response = call(&app, Method::GET, &tree_uri, Some(actor(&alice)), None).await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(state.layer_cache.len(), 1, "first request caches the index");
    assert!(state.layer_cache.weight() > 0);

    // A cache hit must not reopen the archive: corrupt it and the listing is
    // still served, because only a rescan would read the bytes.
    let blob_path = state.storage.blob_path(&layer_digest);
    tokio::fs::write(&blob_path, b"not a tar")
        .await
        .expect("corrupt blob");
    let response = call(&app, Method::GET, &tree_uri, Some(actor(&alice)), None).await;
    assert_eq!(
        response.status(),
        StatusCode::OK,
        "second request is served from the cached index"
    );
    let tree = body_json(response).await;
    assert!(
        tree.as_array()
            .expect("tree")
            .iter()
            .any(|entry| entry["name"] == "etc")
    );

    // Dropping the index forces a rebuild, which now fails on the corrupt blob.
    state.layer_cache.invalidate(&layer_digest);
    assert_eq!(state.layer_cache.len(), 0);
    let response = call(&app, Method::GET, &tree_uri, Some(actor(&alice)), None).await;
    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
}

#[tokio::test]
async fn layer_browsing_sniffs_plain_tar_declared_as_zstd() {
    let (_dir, state, app) = harness().await;
    let alice = create_user(&state, "alice").await;
    let layer = plain_layer_archive();
    let (_, layer_digest) = seed_image_typed(
        &state,
        "alice/img",
        "latest",
        b"config",
        &layer,
        media_types::OCI_IMAGE_LAYER_ZSTD,
    )
    .await;
    let digest = layer_digest.to_string();
    let tree_uri = format!("/api/repositories/alice/img/layers/{digest}/tree");
    let file_uri = format!("/api/repositories/alice/img/layers/{digest}/file");

    // The declared media type claims zstd, but the content is a plain tar;
    // sniffing the magic bytes must still browse it.
    let response = call(&app, Method::GET, &tree_uri, Some(actor(&alice)), None).await;
    assert_eq!(
        response.status(),
        StatusCode::OK,
        "a plain tar declared as zstd must not fail"
    );
    let tree = body_json(response).await;
    let names: Vec<String> = tree
        .as_array()
        .expect("tree")
        .iter()
        .map(|entry| entry["name"].as_str().unwrap_or_default().to_string())
        .collect();
    assert!(names.contains(&"etc".to_string()));
    assert!(names.contains(&"usr".to_string()));

    let response = call(
        &app,
        Method::GET,
        &format!("{file_uri}?path=etc/config.txt"),
        Some(actor(&alice)),
        None,
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let content = body_bytes(response).await;
    assert_eq!(content, b"hello layer\n");
}

#[tokio::test]
async fn layer_browsing_sniffs_zstd_declared_as_plain_tar() {
    let (_dir, state, app) = harness().await;
    let alice = create_user(&state, "alice").await;
    let layer = zstd_layer_archive();
    let (_, layer_digest) = seed_image_typed(
        &state,
        "alice/img",
        "latest",
        b"config",
        &layer,
        media_types::OCI_IMAGE_LAYER,
    )
    .await;
    let digest = layer_digest.to_string();
    let tree_uri = format!("/api/repositories/alice/img/layers/{digest}/tree");
    let file_uri = format!("/api/repositories/alice/img/layers/{digest}/file");

    // The declared media type claims an uncompressed tar, but the content is
    // zstd-compressed; detection is content-based, not media-type-based.
    let response = call(&app, Method::GET, &tree_uri, Some(actor(&alice)), None).await;
    assert_eq!(
        response.status(),
        StatusCode::OK,
        "a zstd tar declared as plain must not fail"
    );
    let tree = body_json(response).await;
    let names: Vec<String> = tree
        .as_array()
        .expect("tree")
        .iter()
        .map(|entry| entry["name"].as_str().unwrap_or_default().to_string())
        .collect();
    assert!(names.contains(&"etc".to_string()));
    assert!(names.contains(&"usr".to_string()));

    let response = call(
        &app,
        Method::GET,
        &format!("{file_uri}?path=etc/config.txt"),
        Some(actor(&alice)),
        None,
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let content = body_bytes(response).await;
    assert_eq!(content, b"hello layer\n");
}

fn tree_path(repo: &str, digest: &str) -> String {
    format!("/api/repositories/{repo}/layers/{digest}/tree")
}

fn tree_names(entries: &Value) -> Vec<String> {
    entries
        .as_array()
        .expect("tree array")
        .iter()
        .map(|entry| entry["name"].as_str().unwrap_or_default().to_string())
        .collect()
}

fn tree_entry<'a>(entries: &'a Value, name: &str) -> &'a Value {
    entries
        .as_array()
        .expect("tree array")
        .iter()
        .find(|entry| entry["name"] == name)
        .unwrap_or_else(|| panic!("entry {name} missing from {entries}"))
}

#[tokio::test]
async fn layer_aggregate_whiteout_and_diff_ghost() {
    let (_dir, state, app) = harness().await;
    let alice = create_user(&state, "alice").await;
    let base = layer_archive_with(&[
        LayerEntry::File("etc/app.conf", b"conf", 0o644),
        LayerEntry::File("etc/keep.txt", b"keep", 0o644),
    ]);
    let upper = layer_archive_with(&[LayerEntry::File("etc/.wh.app.conf", b"", 0o644)]);
    let (manifest, layers) =
        seed_multilayer_image(&state, "alice/img", "latest", &[base, upper]).await;
    let manifest = manifest.to_string();
    let target = layers[1].to_string();
    let base_digest = layers[0].to_string();
    let tree_uri = tree_path("alice/img", &target);

    let response = call(
        &app,
        Method::GET,
        &format!("{tree_uri}?path=etc&mode=aggregate&manifest={manifest}"),
        Some(actor(&alice)),
        None,
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let tree = body_json(response).await;
    assert_eq!(
        tree_names(&tree),
        vec!["keep.txt".to_string()],
        "aggregate omits the whiteouted lower file"
    );
    let keep = tree_entry(&tree, "keep.txt");
    assert!(keep["change"].is_null(), "plain aggregate has no colour");
    assert_eq!(keep["source_digest"], base_digest);

    let response = call(
        &app,
        Method::GET,
        &format!("{tree_uri}?path=etc&mode=diff&manifest={manifest}"),
        Some(actor(&alice)),
        None,
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let tree = body_json(response).await;
    assert_eq!(
        tree_names(&tree),
        vec!["app.conf".to_string()],
        "diff keeps only the removed leaf"
    );
    let ghost = tree_entry(&tree, "app.conf");
    assert_eq!(ghost["change"], "removed");
    assert_eq!(ghost["kind"], "file");
    assert_eq!(ghost["source_digest"], base_digest);

    let response = call(
        &app,
        Method::GET,
        &format!("{tree_uri}?path=etc&mode=aggregate-diff&manifest={manifest}"),
        Some(actor(&alice)),
        None,
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let tree = body_json(response).await;
    assert_eq!(tree_entry(&tree, "app.conf")["change"], "removed");
    assert!(
        tree_entry(&tree, "keep.txt")["change"].is_null(),
        "unchanged survivor is uncoloured"
    );
}

#[tokio::test]
async fn layer_aggregate_opaque_removes_deep_unheadered_path() {
    let (_dir, state, app) = harness().await;
    let alice = create_user(&state, "alice").await;
    let base = layer_archive_with(&[LayerEntry::File("var/lib/deep/data.txt", b"deep", 0o644)]);
    let upper = layer_archive_with(&[LayerEntry::File("var/.wh..wh..opq", b"", 0o644)]);
    let (manifest, layers) =
        seed_multilayer_image(&state, "alice/img", "latest", &[base, upper]).await;
    let manifest = manifest.to_string();
    let target = layers[1].to_string();
    let tree_uri = tree_path("alice/img", &target);

    let response = call(
        &app,
        Method::GET,
        &format!("{tree_uri}?path=var&mode=aggregate&manifest={manifest}"),
        Some(actor(&alice)),
        None,
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let tree = body_json(response).await;
    assert!(
        tree.as_array().expect("tree").is_empty(),
        "opaque var removes the whole deep subtree"
    );

    let response = call(
        &app,
        Method::GET,
        &format!("{tree_uri}?path=var&mode=diff&manifest={manifest}"),
        Some(actor(&alice)),
        None,
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let tree = body_json(response).await;
    let lib = tree_entry(&tree, "lib");
    assert_eq!(lib["change"], "removed");
    assert_eq!(lib["kind"], "dir");
}

#[tokio::test]
async fn layer_whiteout_dir_removes_lower_subtree() {
    let (_dir, state, app) = harness().await;
    let alice = create_user(&state, "alice").await;
    let base = layer_archive_with(&[
        LayerEntry::File("opt/dir/a.txt", b"a", 0o644),
        LayerEntry::File("opt/dir/sub/b.txt", b"b", 0o644),
    ]);
    let upper = layer_archive_with(&[LayerEntry::File("opt/.wh.dir", b"", 0o644)]);
    let (manifest, layers) =
        seed_multilayer_image(&state, "alice/img", "latest", &[base, upper]).await;
    let manifest = manifest.to_string();
    let target = layers[1].to_string();
    let tree_uri = tree_path("alice/img", &target);

    let response = call(
        &app,
        Method::GET,
        &format!("{tree_uri}?path=opt&mode=aggregate&manifest={manifest}"),
        Some(actor(&alice)),
        None,
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let tree = body_json(response).await;
    assert!(
        tree.as_array().expect("tree").is_empty(),
        "a directory whiteout removes the entire subtree"
    );

    let response = call(
        &app,
        Method::GET,
        &format!("{tree_uri}?path=opt&mode=diff&manifest={manifest}"),
        Some(actor(&alice)),
        None,
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let tree = body_json(response).await;
    let dir = tree_entry(&tree, "dir");
    assert_eq!(dir["change"], "removed");
    assert_eq!(dir["kind"], "dir");
}

#[tokio::test]
async fn layer_whiteout_readd_is_unchanged_or_modified() {
    let (_dir, state, app) = harness().await;
    let alice = create_user(&state, "alice").await;
    let base = layer_archive_with(&[
        LayerEntry::File("data/same.txt", b"hello", 0o644),
        LayerEntry::File("data/changed.txt", b"aaaa", 0o644),
        LayerEntry::File("data/mode.txt", b"zzzz", 0o644),
        LayerEntry::Symlink("data/link", "target-a"),
    ]);
    let upper = layer_archive_with(&[
        LayerEntry::File("data/.wh.same.txt", b"", 0o644),
        LayerEntry::File("data/same.txt", b"hello", 0o644),
        LayerEntry::File("data/.wh.changed.txt", b"", 0o644),
        LayerEntry::File("data/changed.txt", b"aaaaaa", 0o644),
        LayerEntry::File("data/.wh.mode.txt", b"", 0o644),
        LayerEntry::File("data/mode.txt", b"zzzz", 0o600),
        LayerEntry::File("data/.wh.link", b"", 0o644),
        LayerEntry::Symlink("data/link", "target-b"),
    ]);
    let (manifest, layers) =
        seed_multilayer_image(&state, "alice/img", "latest", &[base, upper]).await;
    let manifest = manifest.to_string();
    let target = layers[1].to_string();
    let tree_uri = tree_path("alice/img", &target);

    let response = call(
        &app,
        Method::GET,
        &format!("{tree_uri}?path=data&mode=aggregate-diff&manifest={manifest}"),
        Some(actor(&alice)),
        None,
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let tree = body_json(response).await;
    let names = tree_names(&tree);
    assert_eq!(names.len(), 4, "re-added paths stay present: {names:?}");
    assert!(
        tree_entry(&tree, "same.txt")["change"].is_null(),
        "an identical re-add is unchanged"
    );
    assert_eq!(tree_entry(&tree, "changed.txt")["change"], "modified");
    assert_eq!(
        tree_entry(&tree, "mode.txt")["change"],
        "modified",
        "a mode-only change is a modification"
    );
    assert_eq!(tree_entry(&tree, "link")["change"], "modified");
}

#[tokio::test]
async fn layer_aggregate_diff_directory_colors() {
    let (_dir, state, app) = harness().await;
    let alice = create_user(&state, "alice").await;
    let base = layer_archive_with(&[
        LayerEntry::File("mix/a.txt", b"a", 0o644),
        LayerEntry::File("mix/old/b.txt", b"b", 0o644),
        LayerEntry::File("stable/keep.txt", b"k", 0o644),
    ]);
    let upper = layer_archive_with(&[
        LayerEntry::File("mix/.wh.old", b"", 0o644),
        LayerEntry::File("mix/new.txt", b"n", 0o644),
        LayerEntry::File("new/c.txt", b"c", 0o644),
    ]);
    let (manifest, layers) =
        seed_multilayer_image(&state, "alice/img", "latest", &[base, upper]).await;
    let manifest = manifest.to_string();
    let target = layers[1].to_string();
    let tree_uri = tree_path("alice/img", &target);

    let response = call(
        &app,
        Method::GET,
        &format!("{tree_uri}?mode=aggregate-diff&manifest={manifest}"),
        Some(actor(&alice)),
        None,
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let tree = body_json(response).await;
    assert_eq!(tree_entry(&tree, "new")["change"], "new", "all-new folder");
    assert_eq!(
        tree_entry(&tree, "mix")["change"],
        "modified",
        "mixed folder"
    );
    assert!(
        tree_entry(&tree, "stable")["change"].is_null(),
        "all-unchanged folder"
    );

    let response = call(
        &app,
        Method::GET,
        &format!("{tree_uri}?mode=diff&manifest={manifest}"),
        Some(actor(&alice)),
        None,
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let tree = body_json(response).await;
    let names = tree_names(&tree);
    assert!(
        names.contains(&"mix".to_string()),
        "changed ancestor dir kept"
    );
    assert!(names.contains(&"new".to_string()));
    assert!(
        !names.contains(&"stable".to_string()),
        "unchanged dir omitted from diff"
    );

    let response = call(
        &app,
        Method::GET,
        &format!("{tree_uri}?path=mix&mode=aggregate-diff&manifest={manifest}"),
        Some(actor(&alice)),
        None,
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let tree = body_json(response).await;
    let old = tree_entry(&tree, "old");
    assert_eq!(old["change"], "removed", "all-removed folder");
    assert_eq!(old["kind"], "dir");
    assert_eq!(tree_entry(&tree, "new.txt")["change"], "new");
    assert!(tree_entry(&tree, "a.txt")["change"].is_null());
}

#[tokio::test]
async fn layer_diff_first_layer_is_all_new() {
    let (_dir, state, app) = harness().await;
    let alice = create_user(&state, "alice").await;
    let base = layer_archive_with(&[LayerEntry::File("etc/a.txt", b"a", 0o644)]);
    let (manifest, layers) = seed_multilayer_image(&state, "alice/img", "latest", &[base]).await;
    let manifest = manifest.to_string();
    let target = layers[0].to_string();
    let tree_uri = tree_path("alice/img", &target);

    let response = call(
        &app,
        Method::GET,
        &format!("{tree_uri}?mode=diff&manifest={manifest}"),
        Some(actor(&alice)),
        None,
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let tree = body_json(response).await;
    assert_eq!(tree_entry(&tree, "etc")["change"], "new");
    assert!(
        tree_entry(&tree, "etc")["source_digest"].is_null(),
        "a synthesized directory has no source layer"
    );

    let response = call(
        &app,
        Method::GET,
        &format!("{tree_uri}?path=etc&mode=diff&manifest={manifest}"),
        Some(actor(&alice)),
        None,
    )
    .await;
    let tree = body_json(response).await;
    let file = tree_entry(&tree, "a.txt");
    assert_eq!(file["change"], "new");
    assert!(file["source_digest"].is_string());
}

#[tokio::test]
async fn layer_tree_mode_validation_and_manifest_errors() {
    let (_dir, state, app) = harness().await;
    let alice = create_user(&state, "alice").await;
    let base = layer_archive_with(&[LayerEntry::File("etc/a.txt", b"a", 0o644)]);
    let (manifest, layers) = seed_multilayer_image(&state, "alice/img", "latest", &[base]).await;
    let manifest = manifest.to_string();
    let target = layers[0].to_string();
    let tree_uri = tree_path("alice/img", &target);

    let response = call(
        &app,
        Method::GET,
        &format!("{tree_uri}?mode=bogus"),
        Some(actor(&alice)),
        None,
    )
    .await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);

    let response = call(
        &app,
        Method::GET,
        &format!("{tree_uri}?mode=aggregate"),
        Some(actor(&alice)),
        None,
    )
    .await;
    assert_eq!(
        response.status(),
        StatusCode::BAD_REQUEST,
        "a non-single mode requires a manifest"
    );

    let repo_id = state
        .registry
        .find_repository("alice/img")
        .await
        .expect("find")
        .expect("repository")
        .id;
    let orphan = push_blob(
        &state,
        repo_id,
        b"orphan-layer-bytes",
        media_types::OCI_IMAGE_LAYER_GZIP,
    )
    .await;
    let response = call(
        &app,
        Method::GET,
        &format!(
            "{}?mode=aggregate&manifest={manifest}",
            tree_path("alice/img", &orphan.to_string())
        ),
        Some(actor(&alice)),
        None,
    )
    .await;
    assert_eq!(
        response.status(),
        StatusCode::NOT_FOUND,
        "a layer outside the manifest is not browsable"
    );
}

#[tokio::test]
async fn layer_index_disambiguates_shared_layer() {
    let (_dir, state, app) = harness().await;
    let alice = create_user(&state, "alice").await;
    let base_a = layer_archive_with(&[LayerEntry::File("first.txt", b"a", 0o644)]);
    let base_b = layer_archive_with(&[LayerEntry::File("second.txt", b"b", 0o644)]);
    let shared = layer_archive_with(&[LayerEntry::File("shared.txt", b"s", 0o644)]);
    let (m1, l1) = seed_multilayer_image(
        &state,
        "alice/multi",
        "img1",
        &[base_a.clone(), shared.clone()],
    )
    .await;
    let (m2, l2) = seed_multilayer_image(&state, "alice/multi", "img2", &[base_b, shared]).await;
    let shared_digest = l1[1].to_string();
    let tree_uri = tree_path("alice/multi", &shared_digest);

    let response = call(
        &app,
        Method::GET,
        &format!("{tree_uri}?mode=aggregate&manifest={m1}"),
        Some(actor(&alice)),
        None,
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let tree = body_json(response).await;
    assert_eq!(
        tree_names(&tree),
        vec!["first.txt".to_string(), "shared.txt".to_string()]
    );

    let response = call(
        &app,
        Method::GET,
        &format!("{tree_uri}?mode=aggregate&manifest={m2}"),
        Some(actor(&alice)),
        None,
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let tree = body_json(response).await;
    assert_eq!(
        tree_names(&tree),
        vec!["second.txt".to_string(), "shared.txt".to_string()]
    );

    let index_both = seed_index(&state, "alice/multi", "both", &[m1.clone(), m2.clone()]).await;
    let response = call(
        &app,
        Method::GET,
        &format!("{tree_uri}?mode=aggregate&manifest={index_both}"),
        Some(actor(&alice)),
        None,
    )
    .await;
    assert_eq!(
        response.status(),
        StatusCode::BAD_REQUEST,
        "a layer shared by several index children is ambiguous"
    );

    let index_one = seed_index(&state, "alice/multi", "one", std::slice::from_ref(&m1)).await;
    let response = call(
        &app,
        Method::GET,
        &format!("{tree_uri}?mode=aggregate&manifest={index_one}"),
        Some(actor(&alice)),
        None,
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let tree = body_json(response).await;
    assert_eq!(
        tree_names(&tree),
        vec!["first.txt".to_string(), "shared.txt".to_string()]
    );

    let missing = l2[0].to_string();
    let response = call(
        &app,
        Method::GET,
        &format!(
            "{}?mode=aggregate&manifest={index_one}",
            tree_path("alice/multi", &missing)
        ),
        Some(actor(&alice)),
        None,
    )
    .await;
    assert_eq!(
        response.status(),
        StatusCode::NOT_FOUND,
        "a layer in no index child is not found"
    );
}

#[tokio::test]
async fn layer_composed_cache_reuses_and_respects_invalidation() {
    let (_dir, state, app) = harness().await;
    let alice = create_user(&state, "alice").await;
    let base = layer_archive_with(&[LayerEntry::File("a.txt", b"a", 0o644)]);
    let upper = layer_archive_with(&[LayerEntry::File("b.txt", b"b", 0o644)]);
    let (manifest, layers) =
        seed_multilayer_image(&state, "alice/img", "latest", &[base, upper]).await;
    let manifest = manifest.to_string();
    let target = layers[1].to_string();
    let uri = format!(
        "{}?mode=aggregate&manifest={manifest}",
        tree_path("alice/img", &target)
    );

    let response = call(&app, Method::GET, &uri, Some(actor(&alice)), None).await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(state.layer_cache.len(), 2, "both layer indices cached");
    assert_eq!(
        state.composed_cache.len(),
        2,
        "both cumulative positions cached"
    );

    for layer in &layers {
        let path = state.storage.blob_path(layer);
        tokio::fs::write(&path, b"not a tar")
            .await
            .expect("corrupt blob");
    }
    let response = call(&app, Method::GET, &uri, Some(actor(&alice)), None).await;
    assert_eq!(
        response.status(),
        StatusCode::OK,
        "a repeat aggregate is served from the composed cache without rescanning"
    );

    state.layer_cache.invalidate(&layers[0]);
    let response = call(&app, Method::GET, &uri, Some(actor(&alice)), None).await;
    assert_eq!(
        response.status(),
        StatusCode::INTERNAL_SERVER_ERROR,
        "invalidating a layer index must not leave a stale composed overlay"
    );
}

#[tokio::test]
async fn layer_listing_cache_reuses_and_respects_invalidation() {
    let (_dir, state, app) = harness().await;
    let alice = create_user(&state, "alice").await;
    let base = layer_archive_with(&[LayerEntry::File("a.txt", b"a", 0o644)]);
    let upper = layer_archive_with(&[LayerEntry::File("b.txt", b"b", 0o644)]);
    let (manifest, layers) =
        seed_multilayer_image(&state, "alice/img", "latest", &[base, upper]).await;
    let manifest = manifest.to_string();
    let target = layers[1].to_string();
    let uri = format!(
        "{}?mode=aggregate&manifest={manifest}",
        tree_path("alice/img", &target)
    );

    let response = call(&app, Method::GET, &uri, Some(actor(&alice)), None).await;
    assert_eq!(response.status(), StatusCode::OK);
    let first = body_bytes(response).await;
    assert!(
        state.listing_cache.len() >= 1,
        "the rendered listing is cached"
    );

    for layer in &layers {
        let path = state.storage.blob_path(layer);
        tokio::fs::write(&path, b"not a tar")
            .await
            .expect("corrupt blob");
    }
    let response = call(&app, Method::GET, &uri, Some(actor(&alice)), None).await;
    assert_eq!(
        response.status(),
        StatusCode::OK,
        "a repeat listing is served from the cache without rescanning"
    );
    let second = body_bytes(response).await;
    assert_eq!(first, second, "the cached listing is byte-identical");

    state.layer_cache.invalidate(&layers[0]);
    let response = call(&app, Method::GET, &uri, Some(actor(&alice)), None).await;
    assert_eq!(
        response.status(),
        StatusCode::INTERNAL_SERVER_ERROR,
        "invalidating a layer index must not leave a stale listing"
    );
}

#[tokio::test]
async fn layer_single_listing_cache_reuses_and_respects_invalidation() {
    let (_dir, state, app) = harness().await;
    let alice = create_user(&state, "alice").await;
    let layer = layer_archive();
    let (_, layer_digest) = seed_image(&state, "alice/img", "latest", b"config", &layer).await;
    let digest = layer_digest.to_string();
    let tree_uri = format!("/api/repositories/alice/img/layers/{digest}/tree");

    let response = call(&app, Method::GET, &tree_uri, Some(actor(&alice)), None).await;
    assert_eq!(response.status(), StatusCode::OK);
    let first = body_bytes(response).await;
    assert!(
        state.listing_cache.len() >= 1,
        "the single-mode listing is cached"
    );

    let blob_path = state.storage.blob_path(&layer_digest);
    tokio::fs::write(&blob_path, b"not a tar")
        .await
        .expect("corrupt blob");
    let response = call(&app, Method::GET, &tree_uri, Some(actor(&alice)), None).await;
    assert_eq!(
        response.status(),
        StatusCode::OK,
        "a repeat single listing is served from the cache"
    );
    let second = body_bytes(response).await;
    assert_eq!(first, second, "the cached listing is byte-identical");

    state.layer_cache.invalidate(&layer_digest);
    let response = call(&app, Method::GET, &tree_uri, Some(actor(&alice)), None).await;
    assert_eq!(
        response.status(),
        StatusCode::INTERNAL_SERVER_ERROR,
        "invalidating a layer index must not leave a stale listing"
    );
}

#[tokio::test]
async fn layer_changes_cache_reuses_and_respects_invalidation() {
    let (_dir, state, app) = harness().await;
    let alice = create_user(&state, "alice").await;
    let base = layer_archive_with(&[LayerEntry::File("a.txt", b"a", 0o644)]);
    let upper = layer_archive_with(&[LayerEntry::File("b.txt", b"b", 0o644)]);
    let (manifest, layers) =
        seed_multilayer_image(&state, "alice/img", "latest", &[base, upper]).await;
    let manifest = manifest.to_string();
    let target = layers[1].to_string();
    let uri = format!(
        "{}?mode=aggregate-diff&manifest={manifest}",
        tree_path("alice/img", &target)
    );

    let response = call(&app, Method::GET, &uri, Some(actor(&alice)), None).await;
    assert_eq!(response.status(), StatusCode::OK);
    assert!(
        state.changes_cache.len() >= 1,
        "the diff classification is cached"
    );

    for layer in &layers {
        let path = state.storage.blob_path(layer);
        tokio::fs::write(&path, b"not a tar")
            .await
            .expect("corrupt blob");
    }
    let response = call(&app, Method::GET, &uri, Some(actor(&alice)), None).await;
    assert_eq!(
        response.status(),
        StatusCode::OK,
        "a repeat diff is served from the changes/composed cache without rescanning"
    );

    state.layer_cache.invalidate(&layers[0]);
    let response = call(&app, Method::GET, &uri, Some(actor(&alice)), None).await;
    assert_eq!(
        response.status(),
        StatusCode::INTERNAL_SERVER_ERROR,
        "invalidating a layer index must not leave a stale classification"
    );
}

#[tokio::test]
async fn layer_single_mode_reports_null_change_fields() {
    let (_dir, state, app) = harness().await;
    let alice = create_user(&state, "alice").await;
    let layer = layer_archive();
    let (_manifest, layer_digest) =
        seed_image(&state, "alice/img", "latest", b"config", &layer).await;
    let digest = layer_digest.to_string();
    let tree_uri = tree_path("alice/img", &digest);

    for uri in [tree_uri.clone(), format!("{tree_uri}?mode=single")] {
        let response = call(&app, Method::GET, &uri, Some(actor(&alice)), None).await;
        assert_eq!(response.status(), StatusCode::OK);
        let tree = body_json(response).await;
        let entries = tree.as_array().expect("tree");
        assert!(
            entries.iter().any(|entry| entry["name"] == "etc"),
            "single mode still lists the layer"
        );
        for entry in entries {
            assert!(entry["change"].is_null(), "single mode has no change");
            assert!(
                entry["source_digest"].is_null(),
                "single mode has no source digest"
            );
        }
    }
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
    assert!(
        first.get("email").is_none(),
        "search must never expose e-mail"
    );

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
    assert_eq!(
        created["account"]["token_prefix"].as_str().unwrap().len(),
        3
    );

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
    assert!(
        !rendered.contains(&token),
        "listing must not expose the token"
    );
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

#[tokio::test]
async fn service_account_ip_ranges_are_normalized_and_managed() {
    let (_dir, state, app) = harness().await;
    let alice = create_user(&state, "alice").await;

    let response = call(
        &app,
        Method::POST,
        "/api/service-accounts",
        Some(actor(&alice)),
        Some(json!({
            "name": "ci",
            "username": "alice-ci",
            "ip_ranges": ["203.0.113.4", "10.1.2.3/24"],
        })),
    )
    .await;
    assert_eq!(response.status(), StatusCode::CREATED);
    let created = body_json(response).await;
    let account_id = created["account"]["id"].as_i64().expect("account id");
    let ranges = created["account"]["ip_ranges"]
        .as_array()
        .expect("ip_ranges");
    assert_eq!(ranges.len(), 2);
    let rendered = serde_json::Value::Array(ranges.clone()).to_string();
    assert!(rendered.contains("203.0.113.4/32"));
    assert!(rendered.contains("10.1.2.0/24"));

    let response = call(
        &app,
        Method::POST,
        &format!("/api/service-accounts/{account_id}/ip-ranges"),
        Some(actor(&alice)),
        Some(json!({ "cidr": "192.168.0.0/16" })),
    )
    .await;
    assert_eq!(response.status(), StatusCode::CREATED);
    let added = body_json(response).await;
    assert_eq!(added["cidr"], "192.168.0.0/16");
    let range_id = added["id"].as_i64().expect("range id");

    let response = call(
        &app,
        Method::POST,
        &format!("/api/service-accounts/{account_id}/ip-ranges"),
        Some(actor(&alice)),
        Some(json!({ "cidr": "192.168.0.0/16" })),
    )
    .await;
    assert_eq!(response.status(), StatusCode::CONFLICT);

    let response = call(
        &app,
        Method::POST,
        &format!("/api/service-accounts/{account_id}/ip-ranges"),
        Some(actor(&alice)),
        Some(json!({ "cidr": "not-a-cidr" })),
    )
    .await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);

    let response = call(
        &app,
        Method::GET,
        &format!("/api/service-accounts/{account_id}"),
        Some(actor(&alice)),
        None,
    )
    .await;
    let detail = body_json(response).await;
    assert_eq!(detail["ip_ranges"].as_array().expect("ip_ranges").len(), 3);

    let response = call(
        &app,
        Method::GET,
        "/api/service-accounts",
        Some(actor(&alice)),
        None,
    )
    .await;
    let listed = body_json(response).await;
    assert_eq!(
        listed[0]["ip_ranges"].as_array().expect("ip_ranges").len(),
        3
    );

    let response = call(
        &app,
        Method::DELETE,
        &format!("/api/service-accounts/{account_id}/ip-ranges/{range_id}"),
        Some(actor(&alice)),
        None,
    )
    .await;
    assert_eq!(response.status(), StatusCode::NO_CONTENT);

    let response = call(
        &app,
        Method::DELETE,
        &format!("/api/service-accounts/{account_id}/ip-ranges/{range_id}"),
        Some(actor(&alice)),
        None,
    )
    .await;
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn service_account_create_with_invalid_range_is_rejected() {
    let (_dir, state, app) = harness().await;
    let alice = create_user(&state, "alice").await;

    let response = call(
        &app,
        Method::POST,
        "/api/service-accounts",
        Some(actor(&alice)),
        Some(json!({ "name": "ci", "ip_ranges": ["::ffff:1.2.3.0/120"] })),
    )
    .await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);

    let accounts: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM service_accounts")
        .fetch_one(&state.db)
        .await
        .expect("accounts counted");
    assert_eq!(accounts, 0, "a rejected creation leaves no account behind");
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
    sqlx::query(
        "INSERT INTO pull_stats (repository_id, tag_name, day, pulls) VALUES (?, 'a', ?, 7)",
    )
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
    assert!(
        !overview["largest_tags"]
            .as_array()
            .expect("largest")
            .is_empty()
    );
    assert!(
        !overview["disk_usage_by_repository"]
            .as_array()
            .expect("disk")
            .is_empty()
    );
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

    let response = call(
        &app,
        Method::GET,
        "/api/dashboard",
        Some(actor(&alice)),
        None,
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let dashboard = body_json(response).await;
    assert!(
        !dashboard["repositories"]
            .as_array()
            .expect("repositories")
            .is_empty()
    );
    assert_eq!(dashboard["stats"]["repository_count"], 1);
    assert_eq!(dashboard["stats"]["tag_count"], 1);
    assert!(dashboard["stats"]["total_size"].as_i64().unwrap_or(0) > 0);
}

#[tokio::test]
async fn raw_blob_and_manifest_endpoints() {
    let (_dir, state, app) = harness().await;
    let alice = create_user(&state, "alice").await;
    let bob = create_user(&state, "bob").await;
    sqlx::query("UPDATE namespaces SET is_public = 0 WHERE name = 'alice' COLLATE NOCASE")
        .execute(&state.db)
        .await
        .expect("private namespace");

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
    assert_eq!(
        profile["avatar_hash"],
        "497f085b5955fac70a3418401432cdf4223dc77b41067d1c09eddc3eee6bf6da"
    );
    assert!(
        profile.get("email").is_none(),
        "profiles never expose e-mail"
    );
    assert!(profile.get("password_hash").is_none());

    let today = chrono::Utc::now().date_naive();
    let end_param = today.format("%Y-%m-%d").to_string();
    let response = call(
        &app,
        Method::GET,
        &format!("/api/users/alice/heatmap?end={end_param}"),
        Some(actor(&alice)),
        None,
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let heatmap = body_json(response).await;

    let start =
        chrono::NaiveDate::parse_from_str(heatmap["start"].as_str().expect("start"), "%Y-%m-%d")
            .expect("start parses");
    let end = chrono::NaiveDate::parse_from_str(heatmap["end"].as_str().expect("end"), "%Y-%m-%d")
        .expect("end parses");
    use chrono::Datelike;
    assert_eq!(start.weekday(), chrono::Weekday::Sun);
    assert_eq!(end.weekday(), chrono::Weekday::Sat);

    let days = heatmap["days"].as_array().expect("days");
    assert_eq!(days.len(), 364);
    assert_eq!(days.first().expect("first")["date"], heatmap["start"]);
    assert_eq!(days.last().expect("last")["date"], heatmap["end"]);
    assert!(heatmap["total"].is_number());
    assert!(heatmap.get("year").is_none());

    let response = call(
        &app,
        Method::GET,
        "/api/users/alice/heatmap",
        Some(actor(&alice)),
        None,
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let heatmap = body_json(response).await;
    assert_eq!(heatmap["days"].as_array().expect("days").len(), 364);
}

#[tokio::test]
async fn tag_detail_layers_follow_manifest_order_not_digest_order() {
    let (_dir, state, app) = harness().await;
    let alice = create_user(&state, "alice").await;

    // Seed the manifest with layers in descending digest order, so the old
    // digest-ordered query could never produce the same sequence.
    let mut layers: Vec<(Digest, Vec<u8>)> = [
        b"layer-a".to_vec(),
        b"layer-b".to_vec(),
        b"layer-c".to_vec(),
    ]
    .into_iter()
    .map(|bytes| (Digest::from_bytes(&bytes), bytes))
    .collect();
    layers.sort_by_key(|(digest, _)| std::cmp::Reverse(digest.to_string()));
    let ordered: Vec<Vec<u8>> = layers.iter().map(|(_, bytes)| bytes.clone()).collect();
    let expected: Vec<String> = layers
        .iter()
        .map(|(digest, _)| digest.to_string())
        .collect();

    let (_, layer_digests) = seed_multilayer_image(&state, "alice/img", "release", &ordered).await;
    let seeded: Vec<String> = layer_digests.iter().map(|d| d.to_string()).collect();
    assert_eq!(seeded, expected, "manifest declares layers in this order");

    let response = call(
        &app,
        Method::GET,
        "/api/repositories/alice/img/tags/release",
        Some(actor(&alice)),
        None,
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let tag = body_json(response).await;

    let layer_order = |value: &Value| -> Vec<String> {
        value
            .as_array()
            .expect("layers")
            .iter()
            .filter(|layer| layer["role"] == "layer")
            .map(|layer| layer["digest"].as_str().unwrap_or_default().to_string())
            .collect()
    };

    assert_eq!(
        layer_order(&tag["layers"]),
        expected,
        "combined layers follow the manifest, not digest order"
    );
    assert_eq!(
        layer_order(&tag["platform_details"][0]["layers"]),
        expected,
        "platform layers follow the manifest, not digest order"
    );
}

/// Asserts the two non-empty history entries landed on the two layers in order.
fn assert_layer_history(layers: &[Value]) {
    assert_eq!(
        layers.len(),
        2,
        "two non-empty history entries map to two layers"
    );
    assert_eq!(layers[0]["created"], "2024-01-02T00:00:00Z");
    assert_eq!(layers[0]["created_by"], "RUN one");
    assert_eq!(layers[0]["comment"], "layer-1");
    assert_eq!(layers[1]["created"], "2024-01-03T00:00:00Z");
    assert_eq!(layers[1]["created_by"], "RUN two");
    assert_eq!(layers[1]["comment"], "layer-2");
}

#[tokio::test]
async fn layer_history_metadata_maps_non_empty_entries_in_order() {
    let (_dir, state, app) = harness().await;
    let alice = create_user(&state, "alice").await;
    let repo = state
        .registry
        .ensure_repository("alice/hist")
        .await
        .expect("repository");

    let config = serde_json::to_vec(&json!({
        "architecture": "amd64",
        "os": "linux",
        "history": [
            {
                "created": "2024-01-01T00:00:00Z",
                "created_by": "ARG BASE",
                "comment": "buildkit.dockerfile.v0",
                "empty_layer": true
            },
            {
                "created": "2024-01-02T00:00:00Z",
                "created_by": "RUN one",
                "comment": "layer-1"
            },
            {
                "created": "2024-01-03T00:00:00Z",
                "created_by": "RUN two",
                "comment": "layer-2"
            }
        ]
    }))
    .expect("config json");

    let config_digest = push_blob(&state, repo.id, &config, media_types::OCI_IMAGE_CONFIG).await;
    let first = push_blob(
        &state,
        repo.id,
        b"layer-one",
        media_types::OCI_IMAGE_LAYER_GZIP,
    )
    .await;
    let second = push_blob(
        &state,
        repo.id,
        b"layer-two",
        media_types::OCI_IMAGE_LAYER_GZIP,
    )
    .await;
    let descriptors = vec![
        (
            first.to_string(),
            b"layer-one".len() as i64,
            media_types::OCI_IMAGE_LAYER_GZIP.to_string(),
        ),
        (
            second.to_string(),
            b"layer-two".len() as i64,
            media_types::OCI_IMAGE_LAYER_GZIP.to_string(),
        ),
    ];
    let manifest = image_manifest_layers(
        &config_digest.to_string(),
        config.len() as i64,
        &descriptors,
    );
    let manifest_digest = Digest::from_bytes(&manifest);
    state
        .registry
        .put_manifest(repo.id, media_types::OCI_IMAGE_MANIFEST, &manifest)
        .await
        .expect("put manifest");
    state
        .registry
        .set_tag(repo.id, "latest", &manifest_digest)
        .await
        .expect("set tag");

    let role_rows = |value: &Value, role: &str| -> Vec<Value> {
        value
            .as_array()
            .expect("rows")
            .iter()
            .filter(|row| row["role"] == role)
            .cloned()
            .collect()
    };

    let response = call(
        &app,
        Method::GET,
        "/api/repositories/alice/hist/tags/latest",
        Some(actor(&alice)),
        None,
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let tag = body_json(response).await;

    let platform_layers = role_rows(&tag["platform_details"][0]["layers"], "layer");
    assert_layer_history(&platform_layers);
    assert_layer_history(&role_rows(&tag["layers"], "layer"));

    let config_rows = role_rows(&tag["platform_details"][0]["layers"], "config");
    assert_eq!(config_rows.len(), 1, "the config row is present");
    assert!(config_rows[0]["created"].is_null());
    assert!(config_rows[0]["created_by"].is_null());
    assert!(config_rows[0]["comment"].is_null());

    let response = call(
        &app,
        Method::GET,
        &format!("/api/repositories/alice/hist/manifests/{manifest_digest}/references"),
        Some(actor(&alice)),
        None,
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let references = body_json(response).await;
    assert_layer_history(&role_rows(&references, "layer"));
    let reference_config = role_rows(&references, "config");
    assert_eq!(reference_config.len(), 1, "the config reference is present");
    assert!(reference_config[0]["created"].is_null());
    assert!(reference_config[0]["created_by"].is_null());
    assert!(reference_config[0]["comment"].is_null());
}
