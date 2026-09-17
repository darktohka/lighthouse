//! Mark-and-sweep garbage collection over the manifest reference graph.
//!
//! Roots are the manifests referenced by a tag or by a repository link; the
//! mark phase walks `manifest_blobs` (config/layer edges) and
//! `manifest_children` (index edges) transitively. The sweep phase then removes
//! unmarked blobs (row, disk file and repository links) and orphan manifests.
//! In-progress uploads are never touched.

#![allow(dead_code)]

use std::collections::{HashMap, HashSet, VecDeque};

use anyhow::Result;

use crate::oci::digest::Digest;
use crate::storage::registry::Registry;
use crate::storage::Storage;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct GcReport {
    pub marked: usize,
    pub blobs_deleted: usize,
    pub manifests_deleted: usize,
    pub bytes_reclaimed: u64,
}

#[derive(sqlx::FromRow)]
struct BlobRow {
    id: i64,
    digest: String,
    size: i64,
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
pub async fn collect(registry: &Registry, storage: &Storage, dry_run: bool) -> Result<GcReport> {
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

    let all_blobs = sqlx::query_as::<_, BlobRow>("SELECT id, digest, size FROM blobs")
        .fetch_all(db)
        .await?;
    let unmarked: Vec<BlobRow> = all_blobs
        .into_iter()
        .filter(|blob| !marks.contains(&blob.digest))
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
    for blob in &unmarked {
        bytes_reclaimed += blob.size.max(0) as u64;
        if !dry_run {
            if let Ok(digest) = Digest::parse(&blob.digest) {
                storage.delete_blob(&digest).await?;
            }
        }
    }

    Ok(GcReport {
        marked: marks.len(),
        blobs_deleted: unmarked.len(),
        manifests_deleted,
        bytes_reclaimed,
    })
}

/// Repeatedly deletes manifests that are unreachable: no repository link, no
/// tag, and no surviving parent index. Deleting a parent cascades its child
/// edges, which may make further manifests eligible on the next pass.
async fn sweep_orphan_manifests(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
) -> Result<usize> {
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

        let report = collect(&registry, &storage, false).await.expect("collect");
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

        let report = collect(&registry, &storage, false).await.expect("collect");
        assert_eq!(report.blobs_deleted, 4);
        assert_eq!(report.manifests_deleted, 3);
        assert!(report.bytes_reclaimed > 0);
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

        collect(&registry, &storage, false).await.expect("first");
        registry
            .delete_tag(repo_id, "latest")
            .await
            .expect("delete tag");
        collect(&registry, &storage, false).await.expect("sweep");

        // Re-push the graph and tag it again: nothing is collected.
        let (_repo_id, _digest) = build_graph(&registry).await;
        let report = collect(&registry, &storage, false).await.expect("final");
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

        let report = collect(&registry, &storage, true).await.expect("dry run");
        assert_eq!(report.blobs_deleted, 4);
        assert_eq!(report.manifests_deleted, 3);
        assert!(report.bytes_reclaimed > 0);

        let blob_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM blobs")
            .fetch_one(registry.db())
            .await
            .expect("count");
        assert_eq!(blob_count, 4);
    }
}
