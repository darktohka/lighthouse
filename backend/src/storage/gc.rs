//! Mark-and-sweep garbage collection over the manifest reference graph.
//!
//! Roots are the manifests referenced by a tag or by a repository link; the
//! mark phase walks `manifest_blobs` (config/layer edges) and
//! `manifest_children` (index edges) transitively. The sweep phase then removes
//! unmarked blobs (row, disk file and repository links) and orphan manifests.
//! In-progress uploads are never touched.

#![allow(dead_code)]

use std::collections::{HashMap, HashSet, VecDeque};
use std::path::Path;

use anyhow::{Context, Result};
use chrono::{DateTime, Duration, Utc};

use crate::oci::digest::Digest;
use crate::storage::Storage;
use crate::storage::registry::Registry;

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct GcReport {
    pub marked: usize,
    pub blobs_deleted: usize,
    pub manifests_deleted: usize,
    pub bytes_reclaimed: u64,
    /// Digests whose blob row and file were swept, for cache invalidation.
    pub deleted_digests: Vec<Digest>,
}

#[derive(sqlx::FromRow)]
struct BlobRow {
    id: i64,
    digest: String,
    size: i64,
    created_at: DateTime<Utc>,
}

#[derive(sqlx::FromRow)]
struct EdgeRow {
    parent: String,
    child: String,
}

const ORPHAN_MANIFESTS_SQL: &str = "SELECT m.id FROM manifests m \
     WHERE NOT EXISTS (SELECT 1 FROM manifest_repositories mr WHERE mr.manifest_id = m.id) \
       AND NOT EXISTS (SELECT 1 FROM tags t WHERE t.manifest_id = m.id) \
       AND NOT EXISTS (SELECT 1 FROM manifest_children mc WHERE mc.child_manifest_id = m.id)";

/// Runs one garbage collection pass. With `dry_run` the database is left
/// untouched and only the projected report is returned.
///
/// `grace_secs` protects young blobs: any blob whose `created_at` is newer than
/// `now - grace_secs` is left untouched, so a push that has uploaded blobs but
/// not yet published its manifest cannot be collected. A value `<= 0` disables
/// the grace window.
pub async fn collect(
    registry: &Registry,
    storage: &Storage,
    dry_run: bool,
    grace_secs: i64,
) -> Result<GcReport> {
    let db = registry.db();

    let roots: Vec<String> = sqlx::query_scalar(
        "SELECT m.digest FROM tags t JOIN manifests m ON m.id = t.manifest_id \
         UNION \
         SELECT m.digest FROM manifest_repositories mr JOIN manifests m ON m.id = mr.manifest_id",
    )
    .fetch_all(db)
    .await?;

    let mut edges: HashMap<String, Vec<String>> = HashMap::new();
    for edge in sqlx::query_as::<_, EdgeRow>(
        "SELECT pm.digest AS parent, bm.digest AS child \
         FROM manifest_blobs mb \
         JOIN manifests pm ON pm.id = mb.manifest_id \
         JOIN blobs bm ON bm.id = mb.blob_id",
    )
    .fetch_all(db)
    .await?
    {
        edges.entry(edge.parent).or_default().push(edge.child);
    }
    for edge in sqlx::query_as::<_, EdgeRow>(
        "SELECT pm.digest AS parent, cm.digest AS child \
         FROM manifest_children mc \
         JOIN manifests pm ON pm.id = mc.parent_manifest_id \
         JOIN manifests cm ON cm.id = mc.child_manifest_id",
    )
    .fetch_all(db)
    .await?
    {
        edges.entry(edge.parent).or_default().push(edge.child);
    }

    let mut marks: HashSet<String> = HashSet::new();
    let mut queue: VecDeque<String> = VecDeque::new();
    for root in roots {
        if marks.insert(root.clone()) {
            queue.push_back(root);
        }
    }
    while let Some(node) = queue.pop_front() {
        if let Some(children) = edges.get(&node) {
            for child in children {
                if marks.insert(child.clone()) {
                    queue.push_back(child.clone());
                }
            }
        }
    }

    let all_blobs = sqlx::query_as::<_, BlobRow>("SELECT id, digest, size, created_at FROM blobs")
        .fetch_all(db)
        .await?;
    let grace_cutoff = (grace_secs > 0).then(|| Utc::now() - Duration::seconds(grace_secs));
    let unmarked: Vec<BlobRow> = all_blobs
        .into_iter()
        .filter(|blob| !marks.contains(&blob.digest))
        .filter(|blob| grace_cutoff.map_or(true, |cutoff| blob.created_at <= cutoff))
        .collect();

    let mut tx = db.begin().await?;
    for blob in &unmarked {
        sqlx::query("DELETE FROM blobs WHERE id = ?")
            .bind(blob.id)
            .execute(&mut *tx)
            .await?;
    }
    let manifests_deleted = sweep_orphan_manifests(&mut tx).await?;
    if dry_run {
        tx.rollback().await?;
    } else {
        tx.commit().await?;
    }

    let mut bytes_reclaimed = 0u64;
    let mut deleted_digests = Vec::new();
    for blob in &unmarked {
        bytes_reclaimed += blob.size.max(0) as u64;
        if let Ok(digest) = Digest::parse(&blob.digest) {
            if !dry_run {
                if let Err(err) = storage.delete_blob(&digest).await {
                    // The row is already committed; a failed unlink must not
                    // abort the rest of the sweep. The janitor's orphan-file
                    // sweep reclaims the file later.
                    tracing::warn!(
                        digest = %digest,
                        error = %err,
                        "blob file unlink failed"
                    );
                }
            }
            // Even when the unlink failed, the row is gone, so cached views
            // derived from it must be invalidated.
            deleted_digests.push(digest);
        }
    }

    Ok(GcReport {
        marked: marks.len(),
        blobs_deleted: unmarked.len(),
        manifests_deleted,
        bytes_reclaimed,
        deleted_digests,
    })
}

/// Sweeps blob directories that have no `blobs` row and whose `data` file is
/// older than the grace window. This reclaims files orphaned when a previous
/// GC pass committed the row deletion but failed to unlink the file (or the
/// process crashed in between). Per-entry failures are logged and skipped; they
/// never abort the sweep.
pub async fn sweep_orphan_files(
    registry: &Registry,
    storage: &Storage,
    grace_secs: i64,
) -> Result<usize> {
    let cutoff = Utc::now() - Duration::seconds(grace_secs.max(0));
    let mut removed = 0usize;

    let algorithms = match read_dir_paths(&storage.blobs_dir()).await {
        Ok(paths) => paths,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(0),
        Err(err) => return Err(err).context("reading blob root"),
    };

    for algorithm_path in algorithms {
        let Some(algorithm) = algorithm_path
            .file_name()
            .and_then(|name| name.to_str())
            .map(str::to_string)
        else {
            continue;
        };

        let shards = match read_dir_paths(&algorithm_path).await {
            Ok(paths) => paths,
            Err(err) => {
                tracing::warn!(
                    path = %algorithm_path.display(),
                    error = %err,
                    "orphan sweep: cannot read algorithm directory"
                );
                continue;
            }
        };

        for shard_path in shards {
            let entries = match read_dir_paths(&shard_path).await {
                Ok(paths) => paths,
                Err(err) => {
                    tracing::warn!(
                        path = %shard_path.display(),
                        error = %err,
                        "orphan sweep: cannot read shard directory"
                    );
                    continue;
                }
            };

            for blob_path in entries {
                let Some(hex) = blob_path.file_name().and_then(|name| name.to_str()) else {
                    continue;
                };
                if !is_lowercase_hex_64(hex) {
                    continue;
                }

                match tokio::fs::symlink_metadata(&blob_path).await {
                    Ok(meta) if meta.is_dir() => {}
                    Ok(_) => continue,
                    Err(err) => {
                        tracing::warn!(
                            path = %blob_path.display(),
                            error = %err,
                            "orphan sweep: cannot stat blob directory"
                        );
                        continue;
                    }
                }

                let digest = format!("{algorithm}:{hex}");
                match sqlx::query_scalar::<_, i64>("SELECT 1 FROM blobs WHERE digest = ?")
                    .bind(&digest)
                    .fetch_optional(registry.db())
                    .await
                {
                    Ok(Some(_)) => continue,
                    Ok(None) => {}
                    Err(err) => {
                        tracing::warn!(
                            digest = %digest,
                            error = %err,
                            "orphan sweep: blob lookup failed"
                        );
                        continue;
                    }
                }

                // Age is judged by the data file; fall back to the directory
                // itself when `data` is missing or unreadable.
                let modified = match tokio::fs::metadata(blob_path.join("data")).await {
                    Ok(meta) => meta.modified().ok(),
                    Err(_) => match tokio::fs::metadata(&blob_path).await {
                        Ok(meta) => meta.modified().ok(),
                        Err(_) => None,
                    },
                };
                let Some(modified) = modified else {
                    continue;
                };
                if DateTime::<Utc>::from(modified) > cutoff {
                    continue;
                }

                match tokio::fs::remove_dir_all(&blob_path).await {
                    Ok(()) => removed += 1,
                    Err(err) if err.kind() == std::io::ErrorKind::NotFound => continue,
                    Err(err) => {
                        tracing::warn!(
                            path = %blob_path.display(),
                            error = %err,
                            "orphan sweep: cannot remove orphan blob"
                        );
                        continue;
                    }
                }

                if let Some(shard) = blob_path.parent() {
                    // Best-effort: only succeeds once the shard is empty.
                    let _ = tokio::fs::remove_dir(shard).await;
                }
            }
        }
    }

    Ok(removed)
}

async fn read_dir_paths(dir: &Path) -> std::io::Result<Vec<std::path::PathBuf>> {
    let mut entries = tokio::fs::read_dir(dir).await?;
    let mut paths = Vec::new();
    while let Some(entry) = entries.next_entry().await? {
        paths.push(entry.path());
    }
    Ok(paths)
}

/// True for a 64-character lowercase hexadecimal string (a sha256 payload).
fn is_lowercase_hex_64(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

/// Repeatedly deletes manifests that are unreachable: no repository link, no
/// tag, and no surviving parent index. Deleting a parent cascades its child
/// edges, which may make further manifests eligible on the next pass.
async fn sweep_orphan_manifests(tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>) -> Result<usize> {
    let mut deleted = 0usize;
    loop {
        let ids: Vec<i64> = sqlx::query_scalar(ORPHAN_MANIFESTS_SQL)
            .fetch_all(&mut **tx)
            .await?;
        if ids.is_empty() {
            break;
        }
        for id in ids {
            sqlx::query("DELETE FROM manifests WHERE id = ?")
                .bind(id)
                .execute(&mut **tx)
                .await?;
            deleted += 1;
        }
    }
    Ok(deleted)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::oci::media_types;
    use crate::storage::registry::Registry;
    use std::sync::Arc;

    async fn setup() -> (tempfile::TempDir, Registry, Storage) {
        let dir = tempfile::tempdir().expect("tempdir");
        let url = format!("sqlite://{}?mode=rwc", dir.path().join("test.db").display());
        let pool = crate::db::connect(&url).await.expect("connect");
        crate::db::migrate(&pool).await.expect("migrate");

        let now = chrono::Utc::now();
        sqlx::query(
            "INSERT INTO namespaces \
             (name, kind, owner_user_id, description, is_public, created_at, updated_at) \
             VALUES ('darktohka', 'user', NULL, NULL, 0, ?, ?)",
        )
        .bind(now)
        .bind(now)
        .execute(&pool)
        .await
        .expect("namespace");

        let storage = Storage::at(dir.path().join("data"));
        storage.ensure_layout().await.expect("layout");
        let registry = Registry::new(pool, Arc::new(storage.clone()));
        (dir, registry, storage)
    }

    fn image_manifest(config: &str, layers: &[&str]) -> Vec<u8> {
        let layers: Vec<serde_json::Value> = layers
            .iter()
            .map(|digest| {
                serde_json::json!({
                    "mediaType": media_types::OCI_IMAGE_LAYER_GZIP,
                    "digest": digest,
                    "size": 42
                })
            })
            .collect();
        serde_json::json!({
            "schemaVersion": 2,
            "mediaType": media_types::OCI_IMAGE_MANIFEST,
            "config": {
                "mediaType": media_types::OCI_IMAGE_CONFIG,
                "digest": config,
                "size": 7
            },
            "layers": layers
        })
        .to_string()
        .into_bytes()
    }

    fn index_manifest(children: &[&str]) -> Vec<u8> {
        let manifests: Vec<serde_json::Value> = children
            .iter()
            .map(|digest| {
                serde_json::json!({
                    "mediaType": media_types::OCI_IMAGE_MANIFEST,
                    "digest": digest,
                    "size": 100,
                    "platform": { "os": "linux", "architecture": "amd64" }
                })
            })
            .collect();
        serde_json::json!({
            "schemaVersion": 2,
            "mediaType": media_types::OCI_IMAGE_INDEX,
            "manifests": manifests
        })
        .to_string()
        .into_bytes()
    }

    /// Builds the synthetic graph from the architecture document:
    /// `A(config_a, layer1)`, `B(config_b, layer1, layer2)`, `I -> [A, B]`.
    /// The manifests are deliberately left without repository links so that the
    /// tag is the only root.
    async fn build_graph(registry: &Registry) -> (i64, Digest) {
        let repo = registry
            .ensure_repository("darktohka/site")
            .await
            .expect("repository");

        let config_a = Digest::from_bytes(b"config-a").to_string();
        let config_b = Digest::from_bytes(b"config-b").to_string();
        let layer1 = Digest::from_bytes(b"layer-one").to_string();
        let layer2 = Digest::from_bytes(b"layer-two").to_string();

        let manifest_a = image_manifest(&config_a, &[&layer1]);
        let manifest_b = image_manifest(&config_b, &[&layer1, &layer2]);
        let digest_a = Digest::from_bytes(&manifest_a);
        let digest_b = Digest::from_bytes(&manifest_b);
        let index = index_manifest(&[&digest_a.to_string(), &digest_b.to_string()]);
        let digest_i = Digest::from_bytes(&index);

        registry
            .put_manifest(repo.id, media_types::OCI_IMAGE_MANIFEST, &manifest_a)
            .await
            .expect("manifest a");
        registry
            .put_manifest(repo.id, media_types::OCI_IMAGE_MANIFEST, &manifest_b)
            .await
            .expect("manifest b");
        registry
            .put_manifest(repo.id, media_types::OCI_IMAGE_INDEX, &index)
            .await
            .expect("index");

        // The tag is the sole root for this synthetic graph.
        sqlx::query("DELETE FROM manifest_repositories")
            .execute(registry.db())
            .await
            .expect("unlink repositories");

        registry
            .set_tag(repo.id, "latest", &digest_i)
            .await
            .expect("tag index");

        (repo.id, digest_i)
    }

    #[tokio::test]
    async fn marks_reachable_then_sweeps_unreachable() {
        let (_dir, registry, storage) = setup().await;
        let (repo_id, index_digest) = build_graph(&registry).await;

        let report = collect(&registry, &storage, false, 0)
            .await
            .expect("collect");
        assert_eq!(report.blobs_deleted, 0);
        assert_eq!(report.manifests_deleted, 0);
        assert_eq!(report.marked, 7); // 3 manifests + 4 blobs

        let blob_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM blobs")
            .fetch_one(registry.db())
            .await
            .expect("count");
        assert_eq!(blob_count, 4);

        registry
            .delete_tag(repo_id, "latest")
            .await
            .expect("delete tag");

        let report = collect(&registry, &storage, false, 0)
            .await
            .expect("collect");
        assert_eq!(report.blobs_deleted, 4);
        assert_eq!(report.manifests_deleted, 3);
        assert!(report.bytes_reclaimed > 0);
        assert_eq!(report.deleted_digests.len(), 4);
        assert!(index_digest.to_string().starts_with("sha256:"));

        let blob_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM blobs")
            .fetch_one(registry.db())
            .await
            .expect("count");
        assert_eq!(blob_count, 0);
        let manifest_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM manifests")
            .fetch_one(registry.db())
            .await
            .expect("count");
        assert_eq!(manifest_count, 0);
    }

    #[tokio::test]
    async fn retag_keeps_everything() {
        let (_dir, registry, storage) = setup().await;
        let (repo_id, index_digest) = build_graph(&registry).await;

        collect(&registry, &storage, false, 0).await.expect("first");
        registry
            .delete_tag(repo_id, "latest")
            .await
            .expect("delete tag");
        collect(&registry, &storage, false, 0).await.expect("sweep");

        // Re-push the graph and tag it again: nothing is collected.
        let (_repo_id, _digest) = build_graph(&registry).await;
        let report = collect(&registry, &storage, false, 0).await.expect("final");
        assert_eq!(report.blobs_deleted, 0);
        assert_eq!(report.manifests_deleted, 0);
        assert_eq!(
            registry
                .resolve_tag(repo_id, "latest")
                .await
                .expect("resolve"),
            Some(index_digest)
        );
    }

    #[tokio::test]
    async fn dry_run_reports_without_deleting() {
        let (_dir, registry, storage) = setup().await;
        let (repo_id, _index_digest) = build_graph(&registry).await;
        registry
            .delete_tag(repo_id, "latest")
            .await
            .expect("delete tag");

        let report = collect(&registry, &storage, true, 0)
            .await
            .expect("dry run");
        assert_eq!(report.blobs_deleted, 4);
        assert_eq!(report.manifests_deleted, 3);
        assert!(report.bytes_reclaimed > 0);
        assert_eq!(report.deleted_digests.len(), 4);

        let blob_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM blobs")
            .fetch_one(registry.db())
            .await
            .expect("count");
        assert_eq!(blob_count, 4);
    }

    #[tokio::test]
    async fn grace_window_keeps_fresh_blobs() {
        let (_dir, registry, storage) = setup().await;
        let repo = registry
            .ensure_repository("darktohka/site")
            .await
            .expect("repository");

        let content = b"grace-window-layer".to_vec();
        let digest = Digest::from_bytes(&content);
        let source = storage.tmp_dir().join("grace-source");
        tokio::fs::write(&source, &content).await.expect("write");
        storage
            .put_blob_from_path(&source, &digest)
            .await
            .expect("put blob");

        let manifest = image_manifest(&digest.to_string(), &[]);
        let manifest_digest = Digest::from_bytes(&manifest);
        registry
            .put_manifest(repo.id, media_types::OCI_IMAGE_MANIFEST, &manifest)
            .await
            .expect("manifest");
        registry
            .set_tag(repo.id, "latest", &manifest_digest)
            .await
            .expect("tag");
        // Make the tag the only root so the blob becomes unmarked.
        sqlx::query("DELETE FROM manifest_repositories")
            .execute(registry.db())
            .await
            .expect("unlink repositories");
        registry.delete_tag(repo.id, "latest").await.expect("untag");

        let report = collect(&registry, &storage, false, 3600)
            .await
            .expect("grace collect");
        assert_eq!(report.blobs_deleted, 0, "fresh blob must be protected");
        assert!(storage.blob_path(&digest).exists());
        assert!(registry.blob_stat(&digest).await.expect("stat").is_some());

        let report = collect(&registry, &storage, false, 0)
            .await
            .expect("full collect");
        assert_eq!(report.blobs_deleted, 1);
        assert!(registry.blob_stat(&digest).await.expect("stat").is_none());
        assert!(!storage.blob_path(&digest).exists());
    }

    #[tokio::test]
    async fn sweep_orphan_files_removes_untracked_old_dirs() {
        let (_dir, registry, storage) = setup().await;

        let hex_old = format!("ab{}", "0".repeat(62));
        let hex_fresh = format!("cd{}", "1".repeat(62));
        let old_dir = storage.blobs_dir().join("sha256").join("ab").join(&hex_old);
        let fresh_dir = storage
            .blobs_dir()
            .join("sha256")
            .join("cd")
            .join(&hex_fresh);
        tokio::fs::create_dir_all(&old_dir).await.expect("old dir");
        tokio::fs::create_dir_all(&fresh_dir)
            .await
            .expect("fresh dir");
        tokio::fs::write(old_dir.join("data"), b"orphan")
            .await
            .expect("old data");
        tokio::fs::write(fresh_dir.join("data"), b"fresh")
            .await
            .expect("fresh data");

        let long_ago = std::time::SystemTime::now() - std::time::Duration::from_secs(2 * 60 * 60);
        std::fs::File::open(old_dir.join("data"))
            .expect("open old data")
            .set_modified(long_ago)
            .expect("age old file");

        let removed = sweep_orphan_files(&registry, &storage, 60)
            .await
            .expect("sweep");
        assert_eq!(removed, 1);
        assert!(!old_dir.exists());
        assert!(!old_dir.parent().expect("shard").exists());
        assert!(fresh_dir.exists(), "fresh orphan must survive the grace");
    }

    #[tokio::test]
    async fn delete_blob_prunes_empty_shard_directory() {
        let (_dir, _registry, storage) = setup().await;

        let content = b"prunable-shard-blob".to_vec();
        let digest = Digest::from_bytes(&content);
        let source = storage.tmp_dir().join("prune-source");
        tokio::fs::write(&source, &content).await.expect("write");
        storage
            .put_blob_from_path(&source, &digest)
            .await
            .expect("put blob");

        let blob_dir = storage.blob_dir(&digest);
        let shard = blob_dir.parent().expect("shard").to_path_buf();
        assert!(blob_dir.exists());
        assert!(shard.exists());

        storage.delete_blob(&digest).await.expect("delete");
        assert!(!blob_dir.exists());
        assert!(!shard.exists(), "empty shard directory should be pruned");
        assert!(storage.blobs_dir().exists(), "blob root must survive");
    }
}

#[cfg(test)]
mod attestation_tests {
    use super::*;
    use crate::oci::media_types;
    use crate::storage::registry::Registry;
    use std::sync::Arc;

    async fn setup() -> (tempfile::TempDir, Registry, Storage) {
        let dir = tempfile::tempdir().expect("tempdir");
        let url = format!("sqlite://{}?mode=rwc", dir.path().join("test.db").display());
        let pool = crate::db::connect(&url).await.expect("connect");
        crate::db::migrate(&pool).await.expect("migrate");
        let now = chrono::Utc::now();
        sqlx::query(
            "INSERT INTO namespaces \
             (name, kind, owner_user_id, description, is_public, created_at, updated_at) \
             VALUES ('darktohka', 'user', NULL, NULL, 0, ?, ?)",
        )
        .bind(now)
        .bind(now)
        .execute(&pool)
        .await
        .expect("namespace");
        let storage = Storage::at(dir.path().join("data"));
        storage.ensure_layout().await.expect("layout");
        let registry = Registry::new(pool, Arc::new(storage.clone()));
        (dir, registry, storage)
    }

    fn image_manifest(config: &str) -> Vec<u8> {
        serde_json::json!({
            "schemaVersion": 2,
            "mediaType": media_types::OCI_IMAGE_MANIFEST,
            "config": {
                "mediaType": media_types::OCI_IMAGE_CONFIG,
                "digest": config,
                "size": 2
            },
            "layers": []
        })
        .to_string()
        .into_bytes()
    }

    fn attestation_manifest(subject: &str, statement_digest: &str) -> Vec<u8> {
        serde_json::json!({
            "schemaVersion": 2,
            "mediaType": media_types::OCI_IMAGE_MANIFEST,
            "artifactType": media_types::DOCKER_ATTESTATION_MANIFEST,
            "config": {
                "mediaType": media_types::OCI_EMPTY,
                "digest": "sha256:44136fa355b3678a1146ad16f7e8649e94fb4fc21fe77e8310c060f61caaff8a",
                "size": 2
            },
            "layers": [{
                "mediaType": media_types::IN_TOTO_JSON,
                "digest": statement_digest,
                "size": 10,
                "annotations": { media_types::PREDICATE_TYPE_ANNOTATION: "https://slsa.dev/provenance/v1" }
            }],
            "subject": {
                "mediaType": media_types::OCI_IMAGE_MANIFEST,
                "digest": subject,
                "size": 100
            }
        })
        .to_string()
        .into_bytes()
    }

    /// An index whose attestation entry is the only thing keeping the
    /// attestation manifest and its statement blob alive.
    async fn build_attested_index(registry: &Registry) -> (i64, Digest, Digest, Digest) {
        let repo = registry
            .ensure_repository("darktohka/attested")
            .await
            .expect("repository");

        let config = Digest::from_bytes(b"config").to_string();
        let image = image_manifest(&config);
        let image_digest = Digest::from_bytes(&image);
        registry
            .put_manifest(repo.id, media_types::OCI_IMAGE_MANIFEST, &image)
            .await
            .expect("image");

        let statement = Digest::from_bytes(b"statement").to_string();
        let attestation = attestation_manifest(&image_digest.to_string(), &statement);
        let attestation_digest = Digest::from_bytes(&attestation);
        registry
            .put_manifest(repo.id, media_types::OCI_IMAGE_MANIFEST, &attestation)
            .await
            .expect("attestation");

        let index = serde_json::json!({
            "schemaVersion": 2,
            "mediaType": media_types::OCI_IMAGE_INDEX,
            "manifests": [
                {
                    "mediaType": media_types::OCI_IMAGE_MANIFEST,
                    "digest": image_digest.to_string(),
                    "size": 100,
                    "platform": { "os": "linux", "architecture": "amd64" }
                },
                {
                    "mediaType": media_types::OCI_IMAGE_MANIFEST,
                    "digest": attestation_digest.to_string(),
                    "size": 100,
                    "annotations": {
                        media_types::REFERENCE_TYPE_ANNOTATION: media_types::ATTESTATION_MANIFEST_REFERENCE,
                        media_types::REFERENCE_DIGEST_ANNOTATION: image_digest.to_string()
                    },
                    "platform": { "os": "unknown", "architecture": "unknown" }
                }
            ]
        })
        .to_string()
        .into_bytes();
        let index_digest = Digest::from_bytes(&index);
        registry
            .put_manifest(repo.id, media_types::OCI_IMAGE_INDEX, &index)
            .await
            .expect("index");
        registry
            .set_tag(repo.id, "latest", &index_digest)
            .await
            .expect("tag");

        (repo.id, index_digest, image_digest, attestation_digest)
    }

    #[tokio::test]
    async fn gc_keeps_attestation_manifests_reachable_from_the_index() {
        let (_dir, registry, storage) = setup().await;
        let (_repo_id, _index, _image, attestation_digest) = build_attested_index(&registry).await;

        let report = collect(&registry, &storage, false, 0)
            .await
            .expect("collect");
        assert_eq!(report.manifests_deleted, 0, "nothing is unreachable");
        assert_eq!(report.blobs_deleted, 0);

        // The attestation manifest survives, so attestations keep resolving.
        assert!(
            registry
                .manifest(&attestation_digest)
                .await
                .expect("query")
                .is_some(),
            "the attestation manifest must survive GC"
        );
    }

    #[tokio::test]
    async fn gc_reclaims_attestations_once_the_tag_is_removed() {
        let (_dir, registry, storage) = setup().await;
        let (repo_id, _index, image, attestation_digest) = build_attested_index(&registry).await;

        registry
            .delete_tag(repo_id, "latest")
            .await
            .expect("delete tag");
        // Drop the repository links so the tag is the only root.
        sqlx::query("DELETE FROM manifest_repositories")
            .execute(registry.db())
            .await
            .expect("unlink");

        let report = collect(&registry, &storage, false, 0)
            .await
            .expect("collect");
        assert!(report.manifests_deleted >= 3, "index, image and attestation");
        assert!(
            registry
                .manifest(&attestation_digest)
                .await
                .expect("query")
                .is_none(),
            "the attestation manifest is reclaimed with its tag"
        );
        assert!(
            registry
                .manifest(&image)
                .await
                .expect("query")
                .is_none()
        );
    }
}
