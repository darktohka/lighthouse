//! DB-backed content service: repositories, blobs, manifests, tags and the
//! reference graph used by garbage collection.

#![allow(dead_code)]

use std::sync::Arc;

use chrono::Utc;

use crate::db::{self, Db};
use crate::error::{RegistryError, RegistryResult};
use crate::models::{Blob, Manifest, Repository};
use crate::oci::digest::Digest;
use crate::oci::media_types;
use crate::oci::reference;
use crate::storage::Storage;

pub struct Registry {
    db: Db,
    storage: Arc<Storage>,
}

impl Registry {
    pub fn new(db: Db, storage: Arc<Storage>) -> Self {
        Self { db, storage }
    }

    /// The underlying connection pool.
    pub fn db(&self) -> &Db {
        &self.db
    }

    /// The filesystem content store.
    pub fn storage(&self) -> &Storage {
        &self.storage
    }

    // ---- repositories -------------------------------------------------------

    /// Resolves an existing repository, or creates it when its namespace exists.
    ///
    /// Namespace (workspace/user) creation is owned by the control plane; a
    /// repository whose first path segment has no namespace row is rejected with
    /// `NAME_UNKNOWN`.
    pub async fn ensure_repository(&self, name: &str) -> RegistryResult<Repository> {
        if !reference::validate_repository_name(name) {
            return Err(RegistryError::name_invalid(name));
        }
        let (namespace, path) = match name.split_once('/') {
            Some((namespace, path)) => (namespace, path),
            None => (name, ""),
        };

        let namespace_row: Option<(i64, bool)> =
            sqlx::query_as("SELECT id, is_public FROM namespaces WHERE name = ? COLLATE NOCASE LIMIT 1")
                .bind(namespace)
                .fetch_optional(&self.db)
                .await?;
        let (namespace_id, namespace_is_public) =
            namespace_row.ok_or_else(|| RegistryError::name_unknown(name))?;

        let now = Utc::now();
        let repository = db::with_busy_retry(|| async {
            sqlx::query(
                "INSERT INTO repositories \
                 (namespace_id, name, path, description, is_public, created_by, created_at, updated_at) \
                 VALUES (?, ?, ?, NULL, ?, NULL, ?, ?) \
                 ON CONFLICT DO NOTHING",
            )
            .bind(namespace_id)
            .bind(name)
            .bind(path)
            .bind(namespace_is_public)
            .bind(now)
            .bind(now)
            .execute(&self.db)
            .await?;

            sqlx::query_as::<_, Repository>(
                "SELECT * FROM repositories WHERE name = ? COLLATE NOCASE LIMIT 1",
            )
            .bind(name)
            .fetch_one(&self.db)
            .await
        })
        .await
        .map_err(RegistryError::from)?;

        Ok(repository)
    }

    /// Looks up a repository by its full name.
    pub async fn find_repository(&self, name: &str) -> RegistryResult<Option<Repository>> {
        let repository = sqlx::query_as::<_, Repository>(
            "SELECT * FROM repositories WHERE name = ? COLLATE NOCASE LIMIT 1",
        )
        .bind(name)
        .fetch_optional(&self.db)
        .await?;
        Ok(repository)
    }

    /// Lexicographically paginated repository names.
    pub async fn list_repository_names(
        &self,
        n: usize,
        last: Option<&str>,
    ) -> RegistryResult<Vec<String>> {
        let names = sqlx::query_scalar::<_, String>(
            "SELECT name FROM repositories \
             WHERE (? IS NULL OR name > ? COLLATE NOCASE) \
             ORDER BY name COLLATE NOCASE LIMIT ?",
        )
        .bind(last)
        .bind(last)
        .bind(n as i64)
        .fetch_all(&self.db)
        .await?;
        Ok(names)
    }

    // ---- blobs --------------------------------------------------------------

    /// Metadata for a stored blob, if any.
    pub async fn blob_stat(&self, digest: &Digest) -> RegistryResult<Option<Blob>> {
        let blob = sqlx::query_as::<_, Blob>("SELECT * FROM blobs WHERE digest = ?")
            .bind(digest.to_string())
            .fetch_optional(&self.db)
            .await?;
        Ok(blob)
    }

    /// Inserts or refreshes a blob metadata row.
    pub async fn register_blob(
        &self,
        digest: &Digest,
        size: u64,
        media_type: Option<&str>,
    ) -> RegistryResult<Blob> {
        let now = Utc::now();
        let digest_string = digest.to_string();
        let blob = db::with_busy_retry(|| async {
            sqlx::query(
                "INSERT INTO blobs (digest, size, media_type, created_at) VALUES (?, ?, ?, ?) \
                 ON CONFLICT(digest) DO UPDATE SET \
                     size = excluded.size, \
                     media_type = COALESCE(excluded.media_type, blobs.media_type)",
            )
            .bind(&digest_string)
            .bind(size as i64)
            .bind(media_type)
            .bind(now)
            .execute(&self.db)
            .await?;

            sqlx::query_as::<_, Blob>("SELECT * FROM blobs WHERE digest = ?")
                .bind(&digest_string)
                .fetch_one(&self.db)
                .await
        })
        .await
        .map_err(RegistryError::from)?;
        Ok(blob)
    }

    /// Links a blob to a repository. Returns `false` when the link already
    /// existed.
    pub async fn link_blob(&self, repo_id: i64, digest: &Digest) -> RegistryResult<bool> {
        let Some(blob) = self.blob_stat(digest).await? else {
            return Err(RegistryError::blob_unknown(&digest.to_string()));
        };
        let result = sqlx::query(
            "INSERT OR IGNORE INTO blob_repositories (blob_id, repository_id, created_at) \
             VALUES (?, ?, ?)",
        )
        .bind(blob.id)
        .bind(repo_id)
        .bind(Utc::now())
        .execute(&self.db)
        .await?;
        Ok(result.rows_affected() > 0)
    }

    /// True when the blob is linked to the repository.
    pub async fn blob_in_repository(&self, repo_id: i64, digest: &Digest) -> RegistryResult<bool> {
        let found: Option<i64> = sqlx::query_scalar(
            "SELECT b.id FROM blobs b \
             JOIN blob_repositories br ON br.blob_id = b.id \
             WHERE b.digest = ? AND br.repository_id = ? LIMIT 1",
        )
        .bind(digest.to_string())
        .bind(repo_id)
        .fetch_optional(&self.db)
        .await?;
        Ok(found.is_some())
    }

    /// Links a blob that already exists in `from_repo_id` into `to_repo_id`.
    /// Returns `false` when the source repository does not hold the blob.
    pub async fn mount_blob(
        &self,
        from_repo_id: i64,
        to_repo_id: i64,
        digest: &Digest,
    ) -> RegistryResult<bool> {
        if !self.blob_in_repository(from_repo_id, digest).await? {
            return Ok(false);
        }
        self.link_blob(to_repo_id, digest).await?;
        Ok(true)
    }

    // ---- manifests ----------------------------------------------------------

    /// The stored manifest body for a digest, if any.
    pub async fn manifest(&self, digest: &Digest) -> RegistryResult<Option<Manifest>> {
        let manifest = sqlx::query_as::<_, Manifest>("SELECT * FROM manifests WHERE digest = ?")
            .bind(digest.to_string())
            .fetch_optional(&self.db)
            .await?;
        Ok(manifest)
    }

    /// Stores a manifest body, links it into a repository and records its
    /// reference edges, all in one transaction.
    pub async fn put_manifest(
        &self,
        repo_id: i64,
        media_type: &str,
        content: &[u8],
    ) -> RegistryResult<Manifest> {
        let references = ManifestReferences::parse(content, media_type)?;
        let digest = Digest::from_bytes(content).to_string();
        let media_type = media_type.to_string();
        let artifact_type = extract_artifact_type(content);
        let size = content.len() as i64;
        let content = content.to_vec();
        let now = Utc::now();

        let manifest = db::with_busy_retry(|| async {
            let mut tx = self.db.begin().await?;
            sqlx::query(
                "INSERT INTO manifests \
                 (digest, media_type, size, artifact_type, content, created_at) \
                 VALUES (?, ?, ?, ?, ?, ?) \
                 ON CONFLICT(digest) DO UPDATE SET \
                     media_type = excluded.media_type, \
                     size = excluded.size, \
                     artifact_type = COALESCE(excluded.artifact_type, manifests.artifact_type), \
                     content = excluded.content",
            )
            .bind(&digest)
            .bind(&media_type)
            .bind(size)
            .bind(&artifact_type)
            .bind(&content)
            .bind(now)
            .execute(&mut *tx)
            .await?;

            let manifest_id: i64 =
                sqlx::query_scalar("SELECT id FROM manifests WHERE digest = ?")
                    .bind(&digest)
                    .fetch_one(&mut *tx)
                    .await?;

            sqlx::query(
                "INSERT OR IGNORE INTO manifest_repositories \
                 (manifest_id, repository_id, created_at) VALUES (?, ?, ?)",
            )
            .bind(manifest_id)
            .bind(repo_id)
            .bind(now)
            .execute(&mut *tx)
            .await?;

            apply_references(&mut tx, manifest_id, &references).await?;

            let manifest = sqlx::query_as::<_, Manifest>("SELECT * FROM manifests WHERE id = ?")
                .bind(manifest_id)
                .fetch_one(&mut *tx)
                .await?;

            tx.commit().await?;
            Ok(manifest)
        })
        .await
        .map_err(RegistryError::from)?;

        Ok(manifest)
    }

    /// True when the manifest is linked to the repository.
    pub async fn manifest_in_repository(
        &self,
        repo_id: i64,
        digest: &Digest,
    ) -> RegistryResult<bool> {
        let found: Option<i64> = sqlx::query_scalar(
            "SELECT m.id FROM manifests m \
             JOIN manifest_repositories mr ON mr.manifest_id = m.id \
             WHERE m.digest = ? AND mr.repository_id = ? LIMIT 1",
        )
        .bind(digest.to_string())
        .bind(repo_id)
        .fetch_optional(&self.db)
        .await?;
        Ok(found.is_some())
    }

    /// Resolves a tag to its manifest digest.
    pub async fn resolve_tag(&self, repo_id: i64, tag: &str) -> RegistryResult<Option<Digest>> {
        let digest: Option<String> = sqlx::query_scalar(
            "SELECT m.digest FROM tags t \
             JOIN manifests m ON m.id = t.manifest_id \
             WHERE t.repository_id = ? AND t.name = ? LIMIT 1",
        )
        .bind(repo_id)
        .bind(tag)
        .fetch_optional(&self.db)
        .await?;

        match digest {
            Some(raw) => Ok(Some(Digest::parse(&raw)?)),
            None => Ok(None),
        }
    }

    /// Creates or moves a tag to point at a manifest.
    pub async fn set_tag(&self, repo_id: i64, tag: &str, digest: &Digest) -> RegistryResult<()> {
        if !reference::validate_tag(tag) {
            return Err(RegistryError::tag_invalid(format!("invalid tag `{tag}`")));
        }
        let Some(manifest) = self.manifest(digest).await? else {
            return Err(RegistryError::manifest_unknown(&digest.to_string()));
        };
        let now = Utc::now();
        db::with_busy_retry(|| async {
            sqlx::query(
                "INSERT INTO tags (repository_id, name, manifest_id, created_at, updated_at) \
                 VALUES (?, ?, ?, ?, ?) \
                 ON CONFLICT(repository_id, name) DO UPDATE SET \
                     manifest_id = excluded.manifest_id, updated_at = excluded.updated_at",
            )
            .bind(repo_id)
            .bind(tag)
            .bind(manifest.id)
            .bind(now)
            .bind(now)
            .execute(&self.db)
            .await?;
            Ok::<(), sqlx::Error>(())
        })
        .await
        .map_err(RegistryError::from)?;
        Ok(())
    }

    /// Deletes a tag. Returns `false` when it did not exist.
    pub async fn delete_tag(&self, repo_id: i64, tag: &str) -> RegistryResult<bool> {
        let result = sqlx::query("DELETE FROM tags WHERE repository_id = ? AND name = ?")
            .bind(repo_id)
            .bind(tag)
            .execute(&self.db)
            .await?;
        Ok(result.rows_affected() > 0)
    }

    /// Removes a manifest from a repository, deleting the manifest row itself
    /// once no repository link, tag or parent index references it.
    pub async fn delete_manifest(&self, repo_id: i64, digest: &Digest) -> RegistryResult<bool> {
        let digest = digest.to_string();
        let removed = db::with_busy_retry(|| async {
            let mut tx = self.db.begin().await?;

            let manifest_id: Option<i64> =
                sqlx::query_scalar("SELECT id FROM manifests WHERE digest = ?")
                    .bind(&digest)
                    .fetch_optional(&mut *tx)
                    .await?;
            let Some(manifest_id) = manifest_id else {
                tx.commit().await?;
                return Ok(false);
            };

            let unlink = sqlx::query(
                "DELETE FROM manifest_repositories WHERE manifest_id = ? AND repository_id = ?",
            )
            .bind(manifest_id)
            .bind(repo_id)
            .execute(&mut *tx)
            .await?;

            let links: i64 =
                sqlx::query_scalar("SELECT COUNT(*) FROM manifest_repositories WHERE manifest_id = ?")
                    .bind(manifest_id)
                    .fetch_one(&mut *tx)
                    .await?;
            let tags: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM tags WHERE manifest_id = ?")
                .bind(manifest_id)
                .fetch_one(&mut *tx)
                .await?;
            let parents: i64 = sqlx::query_scalar(
                "SELECT COUNT(*) FROM manifest_children WHERE child_manifest_id = ?",
            )
            .bind(manifest_id)
            .fetch_one(&mut *tx)
            .await?;

            let mut removed = unlink.rows_affected() > 0;
            if links == 0 && tags == 0 && parents == 0 {
                sqlx::query("DELETE FROM manifests WHERE id = ?")
                    .bind(manifest_id)
                    .execute(&mut *tx)
                    .await?;
                removed = true;
            }

            tx.commit().await?;
            Ok(removed)
        })
        .await
        .map_err(RegistryError::from)?;

        Ok(removed)
    }

    /// Lexicographically paginated tag names for a repository.
    pub async fn list_tags(
        &self,
        repo_id: i64,
        n: usize,
        last: Option<&str>,
    ) -> RegistryResult<Vec<String>> {
        let tags = sqlx::query_scalar::<_, String>(
            "SELECT name FROM tags \
             WHERE repository_id = ? AND (? IS NULL OR name > ?) \
             ORDER BY name LIMIT ?",
        )
        .bind(repo_id)
        .bind(last)
        .bind(last)
        .bind(n as i64)
        .fetch_all(&self.db)
        .await?;
        Ok(tags)
    }

    /// Records the reference edges of an already-stored manifest body.
    pub async fn record_manifest_references(
        &self,
        manifest_id: i64,
        content: &[u8],
        media_type: &str,
    ) -> RegistryResult<()> {
        let references = ManifestReferences::parse(content, media_type)?;
        db::with_busy_retry(|| async {
            let mut tx = self.db.begin().await?;
            apply_references(&mut tx, manifest_id, &references).await?;
            tx.commit().await?;
            Ok::<(), sqlx::Error>(())
        })
        .await
        .map_err(RegistryError::from)?;
        Ok(())
    }
}

/// Reference edges extracted from a manifest body.
enum ManifestReferences {
    Blobs(Vec<BlobReference>),
    Children(Vec<ChildReference>),
}

struct BlobReference {
    role: &'static str,
    digest: Digest,
    size: u64,
    media_type: Option<String>,
}

struct ChildReference {
    digest: Digest,
    size: u64,
    media_type: String,
    platform_os: Option<String>,
    platform_architecture: Option<String>,
    platform_variant: Option<String>,
}

impl ManifestReferences {
    /// Parses a manifest body into its graph edges. Malformed JSON or an
    /// invalid descriptor digest map to `MANIFEST_INVALID`.
    fn parse(content: &[u8], media_type: &str) -> RegistryResult<Self> {
        let value: serde_json::Value = serde_json::from_slice(content)
            .map_err(|err| RegistryError::manifest_invalid(format!("invalid manifest JSON: {err}")))?;
        let object = value.as_object().ok_or_else(|| {
            RegistryError::manifest_invalid("manifest body must be a JSON object")
        })?;

        if media_types::is_index_type(media_type) {
            let mut children = Vec::new();
            if let Some(entries) = object.get("manifests") {
                let array = entries.as_array().ok_or_else(|| {
                    RegistryError::manifest_invalid("`manifests` must be an array")
                })?;
                for entry in array {
                    let (os, architecture, variant) = match entry.get("platform") {
                        Some(platform) => (
                            string_field(platform, "os"),
                            string_field(platform, "architecture"),
                            string_field(platform, "variant"),
                        ),
                        None => (None, None, None),
                    };
                    children.push(ChildReference {
                        digest: descriptor_digest(entry)?,
                        size: descriptor_size(entry),
                        media_type: string_field(entry, "mediaType")
                            .unwrap_or_else(|| media_types::OCI_IMAGE_MANIFEST.to_string()),
                        platform_os: os,
                        platform_architecture: architecture,
                        platform_variant: variant,
                    });
                }
            }
            return Ok(ManifestReferences::Children(children));
        }

        if media_types::is_manifest_type(media_type) {
            let mut blobs = Vec::new();
            if let Some(config) = object.get("config") {
                blobs.push(BlobReference {
                    role: "config",
                    digest: descriptor_digest(config)?,
                    size: descriptor_size(config),
                    media_type: string_field(config, "mediaType"),
                });
            }
            if let Some(layers) = object.get("layers") {
                let array = layers
                    .as_array()
                    .ok_or_else(|| RegistryError::manifest_invalid("`layers` must be an array"))?;
                for layer in array {
                    blobs.push(BlobReference {
                        role: "layer",
                        digest: descriptor_digest(layer)?,
                        size: descriptor_size(layer),
                        media_type: string_field(layer, "mediaType"),
                    });
                }
            }
            return Ok(ManifestReferences::Blobs(blobs));
        }

        Ok(ManifestReferences::Blobs(Vec::new()))
    }
}

async fn apply_references(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    manifest_id: i64,
    references: &ManifestReferences,
) -> Result<(), sqlx::Error> {
    let now = Utc::now();
    match references {
        ManifestReferences::Blobs(blobs) => {
            for blob in blobs {
                let digest = blob.digest.to_string();
                sqlx::query(
                    "INSERT INTO blobs (digest, size, media_type, created_at) VALUES (?, ?, ?, ?) \
                     ON CONFLICT(digest) DO UPDATE SET \
                         size = excluded.size, \
                         media_type = COALESCE(excluded.media_type, blobs.media_type)",
                )
                .bind(&digest)
                .bind(blob.size as i64)
                .bind(blob.media_type.as_deref())
                .bind(now)
                .execute(&mut **tx)
                .await?;

                let blob_id: i64 = sqlx::query_scalar("SELECT id FROM blobs WHERE digest = ?")
                    .bind(&digest)
                    .fetch_one(&mut **tx)
                    .await?;

                sqlx::query(
                    "INSERT INTO manifest_blobs (manifest_id, blob_id, role) VALUES (?, ?, ?) \
                     ON CONFLICT DO NOTHING",
                )
                .bind(manifest_id)
                .bind(blob_id)
                .bind(blob.role)
                .execute(&mut **tx)
                .await?;
            }
        }
        ManifestReferences::Children(children) => {
            for child in children {
                let digest = child.digest.to_string();
                sqlx::query(
                    "INSERT INTO manifests \
                     (digest, media_type, size, artifact_type, content, created_at) \
                     VALUES (?, ?, ?, NULL, ?, ?) \
                     ON CONFLICT(digest) DO NOTHING",
                )
                .bind(&digest)
                .bind(&child.media_type)
                .bind(child.size as i64)
                .bind(Vec::<u8>::new())
                .bind(now)
                .execute(&mut **tx)
                .await?;

                let child_id: i64 = sqlx::query_scalar("SELECT id FROM manifests WHERE digest = ?")
                    .bind(&digest)
                    .fetch_one(&mut **tx)
                    .await?;

                sqlx::query(
                    "INSERT INTO manifest_children \
                     (parent_manifest_id, child_manifest_id, platform_os, platform_architecture, platform_variant) \
                     VALUES (?, ?, ?, ?, ?) ON CONFLICT DO NOTHING",
                )
                .bind(manifest_id)
                .bind(child_id)
                .bind(child.platform_os.as_deref())
                .bind(child.platform_architecture.as_deref())
                .bind(child.platform_variant.as_deref())
                .execute(&mut **tx)
                .await?;
            }
        }
    }
    Ok(())
}

fn descriptor_digest(value: &serde_json::Value) -> RegistryResult<Digest> {
    let raw = value
        .get("digest")
        .and_then(|digest| digest.as_str())
        .ok_or_else(|| RegistryError::manifest_invalid("descriptor is missing `digest`"))?;
    Digest::parse(raw)
        .map_err(|_| RegistryError::manifest_invalid(format!("descriptor digest `{raw}` is invalid")))
}

fn descriptor_size(value: &serde_json::Value) -> u64 {
    value.get("size").and_then(|size| size.as_u64()).unwrap_or(0)
}

fn string_field(value: &serde_json::Value, key: &str) -> Option<String> {
    value
        .get(key)
        .and_then(|field| field.as_str())
        .map(str::to_string)
}

fn extract_artifact_type(content: &[u8]) -> Option<String> {
    serde_json::from_slice::<serde_json::Value>(content)
        .ok()
        .and_then(|value| string_field(&value, "artifactType"))
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn test_registry() -> (tempfile::TempDir, Registry) {
        let dir = tempfile::tempdir().expect("tempdir");
        let url = format!("sqlite://{}?mode=rwc", dir.path().join("test.db").display());
        let pool = crate::db::connect(&url).await.expect("connect");
        crate::db::migrate(&pool).await.expect("migrate");
        let storage = Arc::new(Storage::at(dir.path().join("data")));
        storage.ensure_layout().await.expect("layout");
        (dir, Registry::new(pool, storage))
    }

    async fn create_namespace(registry: &Registry, name: &str) {
        let now = Utc::now();
        sqlx::query(
            "INSERT INTO namespaces \
             (name, kind, owner_user_id, description, is_public, created_at, updated_at) \
             VALUES (?, 'user', NULL, NULL, 0, ?, ?)",
        )
        .bind(name)
        .bind(now)
        .bind(now)
        .execute(registry.db())
        .await
        .expect("insert namespace");
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

    #[tokio::test]
    async fn ensure_repository_requires_existing_namespace() {
        let (_dir, registry) = test_registry().await;
        let err = registry.ensure_repository("ghost/project").await.unwrap_err();
        assert_eq!(err.code, crate::error::ErrorCode::NameUnknown);

        create_namespace(&registry, "darktohka").await;
        let repository = registry
            .ensure_repository("darktohka/more/complicated/project2")
            .await
            .expect("create repository");
        assert_eq!(repository.name, "darktohka/more/complicated/project2");
        assert_eq!(repository.path, "more/complicated/project2");

        let again = registry
            .ensure_repository("darktohka/more/complicated/project2")
            .await
            .expect("idempotent");
        assert_eq!(again.id, repository.id);

        let found = registry
            .find_repository("darktohka/more/complicated/project2")
            .await
            .expect("find");
        assert_eq!(found.map(|r| r.id), Some(repository.id));

        let names = registry.list_repository_names(10, None).await.expect("list");
        assert_eq!(names, vec!["darktohka/more/complicated/project2"]);
        let page = registry
            .list_repository_names(10, Some("darktohka/more/complicated/project2"))
            .await
            .expect("page");
        assert!(page.is_empty());
    }

    #[tokio::test]
    async fn ensure_repository_rejects_invalid_names() {
        let (_dir, registry) = test_registry().await;
        let err = registry.ensure_repository("Bad/Name").await.unwrap_err();
        assert_eq!(err.code, crate::error::ErrorCode::NameInvalid);
    }

    #[tokio::test]
    async fn ensure_repository_inherits_namespace_visibility() {
        let (_dir, registry) = test_registry().await;
        let now = Utc::now();
        for (name, is_public) in [("public-team", true), ("private-team", false)] {
            sqlx::query(
                "INSERT INTO namespaces \
                 (name, kind, owner_user_id, description, is_public, created_at, updated_at) \
                 VALUES (?, 'workspace', NULL, NULL, ?, ?, ?)",
            )
            .bind(name)
            .bind(is_public)
            .bind(now)
            .bind(now)
            .execute(registry.db())
            .await
            .expect("insert namespace");

            let repository = registry
                .ensure_repository(&format!("{name}/app"))
                .await
                .expect("create repository");
            assert_eq!(repository.is_public, is_public);

            let again = registry
                .ensure_repository(&format!("{name}/app"))
                .await
                .expect("idempotent");
            assert_eq!(again.is_public, is_public);
        }
    }

    #[tokio::test]
    async fn blob_crud_round_trips() {
        let (_dir, registry) = test_registry().await;
        create_namespace(&registry, "darktohka").await;
        let repository = registry
            .ensure_repository("darktohka/site")
            .await
            .expect("repository");

        let digest = Digest::from_bytes(b"layer bytes");
        assert!(registry.blob_stat(&digest).await.expect("stat").is_none());
        let blob = registry
            .register_blob(&digest, 11, Some(media_types::OCI_IMAGE_LAYER_GZIP))
            .await
            .expect("register");
        assert_eq!(blob.size, 11);
        assert_eq!(
            blob.media_type.as_deref(),
            Some(media_types::OCI_IMAGE_LAYER_GZIP)
        );

        assert!(!registry
            .blob_in_repository(repository.id, &digest)
            .await
            .expect("linked"));
        assert!(registry
            .link_blob(repository.id, &digest)
            .await
            .expect("link"));
        assert!(!registry
            .link_blob(repository.id, &digest)
            .await
            .expect("relink"));
        assert!(registry
            .blob_in_repository(repository.id, &digest)
            .await
            .expect("linked"));
    }

    #[tokio::test]
    async fn mounting_requires_source_link() {
        let (_dir, registry) = test_registry().await;
        create_namespace(&registry, "darktohka").await;
        let source = registry
            .ensure_repository("darktohka/source")
            .await
            .expect("source");
        let target = registry
            .ensure_repository("darktohka/target")
            .await
            .expect("target");

        let digest = Digest::from_bytes(b"mounted layer");
        registry
            .register_blob(&digest, 13, None)
            .await
            .expect("register");

        assert!(!registry
            .mount_blob(source.id, target.id, &digest)
            .await
            .expect("unlinked mount"));
        registry.link_blob(source.id, &digest).await.expect("link");
        assert!(registry
            .mount_blob(source.id, target.id, &digest)
            .await
            .expect("mount"));
        assert!(registry
            .blob_in_repository(target.id, &digest)
            .await
            .expect("target linked"));
    }

    #[tokio::test]
    async fn manifest_and_tag_round_trip() {
        let (_dir, registry) = test_registry().await;
        create_namespace(&registry, "darktohka").await;
        let repository = registry
            .ensure_repository("darktohka/site")
            .await
            .expect("repository");

        let config = Digest::from_bytes(b"config");
        let layer = Digest::from_bytes(b"layer");
        let content = image_manifest(&config.to_string(), &[&layer.to_string()]);
        let digest = Digest::from_bytes(&content);

        let manifest = registry
            .put_manifest(repository.id, media_types::OCI_IMAGE_MANIFEST, &content)
            .await
            .expect("put manifest");
        assert_eq!(manifest.digest, digest.to_string());
        assert_eq!(manifest.size, content.len() as i64);
        assert!(registry
            .manifest_in_repository(repository.id, &digest)
            .await
            .expect("linked"));

        let edge_count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM manifest_blobs WHERE manifest_id = ?",
        )
        .bind(manifest.id)
        .fetch_one(registry.db())
        .await
        .expect("edges");
        assert_eq!(edge_count, 2);
        let config_blob = registry.blob_stat(&config).await.expect("config blob");
        assert!(config_blob.is_some());

        assert!(registry
            .resolve_tag(repository.id, "latest")
            .await
            .expect("resolve")
            .is_none());
        registry
            .set_tag(repository.id, "latest", &digest)
            .await
            .expect("set tag");
        registry
            .set_tag(repository.id, "v1", &digest)
            .await
            .expect("set tag");
        assert_eq!(
            registry
                .resolve_tag(repository.id, "latest")
                .await
                .expect("resolve"),
            Some(digest.clone())
        );
        let tags = registry
            .list_tags(repository.id, 10, None)
            .await
            .expect("tags");
        assert_eq!(tags, vec!["latest".to_string(), "v1".to_string()]);
        assert!(registry
            .delete_tag(repository.id, "v1")
            .await
            .expect("delete tag"));
        assert!(!registry
            .delete_tag(repository.id, "v1")
            .await
            .expect("re-delete"));
    }

    #[tokio::test]
    async fn index_manifest_records_children() {
        let (_dir, registry) = test_registry().await;
        create_namespace(&registry, "darktohka").await;
        let repository = registry
            .ensure_repository("darktohka/site")
            .await
            .expect("repository");

        let child = Digest::from_bytes(b"child manifest");
        let content = index_manifest(&[&child.to_string()]);
        let manifest = registry
            .put_manifest(repository.id, media_types::OCI_IMAGE_INDEX, &content)
            .await
            .expect("put index");

        let children: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM manifest_children WHERE parent_manifest_id = ?")
                .bind(manifest.id)
                .fetch_one(registry.db())
                .await
                .expect("children");
        assert_eq!(children, 1);

        let child_row: Option<String> =
            sqlx::query_scalar("SELECT digest FROM manifests WHERE digest = ?")
                .bind(child.to_string())
                .fetch_optional(registry.db())
                .await
                .expect("child row");
        assert!(child_row.is_some());
    }

    #[tokio::test]
    async fn malformed_manifest_is_rejected() {
        let (_dir, registry) = test_registry().await;
        create_namespace(&registry, "darktohka").await;
        let repository = registry
            .ensure_repository("darktohka/site")
            .await
            .expect("repository");
        let err = registry
            .put_manifest(repository.id, media_types::OCI_IMAGE_MANIFEST, b"{not json")
            .await
            .unwrap_err();
        assert_eq!(err.code, crate::error::ErrorCode::ManifestInvalid);

        let bad_descriptor = serde_json::json!({
            "schemaVersion": 2,
            "config": { "digest": "not-a-digest", "size": 1 },
            "layers": []
        })
        .to_string()
        .into_bytes();
        let err = registry
            .put_manifest(
                repository.id,
                media_types::OCI_IMAGE_MANIFEST,
                &bad_descriptor,
            )
            .await
            .unwrap_err();
        assert_eq!(err.code, crate::error::ErrorCode::ManifestInvalid);
    }

    #[tokio::test]
    async fn delete_manifest_unlinks_then_removes() {
        let (_dir, registry) = test_registry().await;
        create_namespace(&registry, "darktohka").await;
        let first = registry
            .ensure_repository("darktohka/one")
            .await
            .expect("first");
        let second = registry
            .ensure_repository("darktohka/two")
            .await
            .expect("second");

        let content = image_manifest(&Digest::from_bytes(b"cfg").to_string(), &[]);
        let digest = Digest::from_bytes(&content);
        registry
            .put_manifest(first.id, media_types::OCI_IMAGE_MANIFEST, &content)
            .await
            .expect("put first");
        registry
            .put_manifest(second.id, media_types::OCI_IMAGE_MANIFEST, &content)
            .await
            .expect("put second");

        assert!(registry
            .delete_manifest(first.id, &digest)
            .await
            .expect("unlink"));
        assert!(registry
            .manifest(&digest)
            .await
            .expect("still present")
            .is_some());
        assert!(registry
            .delete_manifest(second.id, &digest)
            .await
            .expect("final unlink"));
        assert!(registry.manifest(&digest).await.expect("removed").is_none());
    }
}
