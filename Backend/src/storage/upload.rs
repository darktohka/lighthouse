//! Upload session bookkeeping: `uploads` / `upload_chunks` rows layered on top
//! of the filesystem primitives in [`crate::storage`].

#![allow(dead_code)]

use std::sync::Arc;

use chrono::{Duration, Utc};
use uuid::Uuid;

use crate::db::{self, Db};
use crate::error::{RegistryError, RegistryResult};
use crate::models::Upload;
use crate::oci::digest::Digest;
use crate::storage::registry::Registry;
use crate::storage::Storage;

pub struct Uploads {
    db: Db,
    storage: Arc<Storage>,
    registry: Arc<Registry>,
}

impl Uploads {
    pub fn new(db: Db, storage: Arc<Storage>, registry: Arc<Registry>) -> Self {
        Self {
            db,
            storage,
            registry,
        }
    }

    /// Opens a new upload session, creating its directory and `startedat`
    /// marker before the database row is written.
    pub async fn create(&self, repo_id: i64, user_id: Option<i64>) -> RegistryResult<Upload> {
        let uuid = Uuid::new_v4().to_string();
        let now = Utc::now();
        self.storage
            .create_upload(&uuid, now)
            .await
            .map_err(storage_error)?;

        db::with_busy_retry(|| async {
            sqlx::query(
                "INSERT INTO uploads \
                 (uuid, repository_id, \"offset\", created_by, started_at, updated_at) \
                 VALUES (?, ?, 0, ?, ?, ?)",
            )
            .bind(&uuid)
            .bind(repo_id)
            .bind(user_id)
            .bind(now)
            .bind(now)
            .execute(&self.db)
            .await?;
            Ok::<(), sqlx::Error>(())
        })
        .await
        .map_err(RegistryError::from)?;

        self.status(&uuid)
            .await?
            .ok_or_else(|| RegistryError::upload_unknown(&uuid))
    }

    /// Looks up an upload session.
    pub async fn status(&self, uuid: &str) -> RegistryResult<Option<Upload>> {
        let upload = sqlx::query_as::<_, Upload>("SELECT * FROM uploads WHERE uuid = ?")
            .bind(uuid)
            .fetch_optional(&self.db)
            .await?;
        Ok(upload)
    }

    /// Appends a contiguous chunk of bytes and advances the recorded offset.
    pub async fn append(&self, uuid: &str, offset: u64, data: &[u8]) -> RegistryResult<u64> {
        if self.status(uuid).await?.is_none() {
            return Err(RegistryError::upload_unknown(uuid));
        }

        self.storage
            .write_upload_chunk(uuid, offset, data)
            .await
            .map_err(storage_error)?;

        let new_size = offset + data.len() as u64;
        let now = Utc::now();
        let uuid_owned = uuid.to_string();
        db::with_busy_retry(|| async {
            let mut tx = self.db.begin().await?;
            sqlx::query(
                "INSERT OR IGNORE INTO upload_chunks (upload_uuid, start_offset, end_offset) \
                 VALUES (?, ?, ?)",
            )
            .bind(&uuid_owned)
            .bind(offset as i64)
            .bind(new_size as i64)
            .execute(&mut *tx)
            .await?;
            sqlx::query("UPDATE uploads SET \"offset\" = ?, updated_at = ? WHERE uuid = ?")
                .bind(new_size as i64)
                .bind(now)
                .bind(&uuid_owned)
                .execute(&mut *tx)
                .await?;
            tx.commit().await?;
            Ok::<(), sqlx::Error>(())
        })
        .await
        .map_err(RegistryError::from)?;

        Ok(new_size)
    }

    /// Verifies the received bytes against `digest`, moves them into the blob
    /// store, registers and links the blob, then drops the upload session.
    pub async fn complete(&self, uuid: &str, digest: &Digest) -> RegistryResult<u64> {
        let Some(upload) = self.status(uuid).await? else {
            return Err(RegistryError::upload_unknown(uuid));
        };

        let size = self
            .storage
            .finalize_upload(uuid, digest)
            .await
            .map_err(storage_error)?;

        self.registry.register_blob(digest, size, None).await?;
        self.registry.link_blob(upload.repository_id, digest).await?;
        self.delete_session(uuid).await?;
        Ok(size)
    }

    /// Aborts an upload, deleting both its files and its database rows.
    pub async fn cancel(&self, uuid: &str) -> RegistryResult<()> {
        self.storage
            .discard_upload(uuid)
            .await
            .map_err(storage_error)?;
        self.delete_session(uuid).await
    }

    /// Cancels upload sessions whose `updated_at` is older than `ttl_secs`.
    pub async fn cleanup_stale(&self, ttl_secs: i64) -> RegistryResult<usize> {
        let cutoff = Utc::now() - Duration::seconds(ttl_secs);
        let stale: Vec<String> = sqlx::query_scalar("SELECT uuid FROM uploads WHERE updated_at < ?")
            .bind(cutoff)
            .fetch_all(&self.db)
            .await?;

        let mut removed = 0;
        for uuid in stale {
            self.cancel(&uuid).await?;
            removed += 1;
        }
        Ok(removed)
    }

    async fn delete_session(&self, uuid: &str) -> RegistryResult<()> {
        sqlx::query("DELETE FROM uploads WHERE uuid = ?")
            .bind(uuid)
            .execute(&self.db)
            .await?;
        Ok(())
    }
}

fn storage_error(err: anyhow::Error) -> RegistryError {
    tracing::error!(error = %err, "upload storage failure");
    RegistryError::internal("storage error")
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn harness() -> (tempfile::TempDir, Uploads, i64) {
        let dir = tempfile::tempdir().expect("tempdir");
        let url = format!("sqlite://{}?mode=rwc", dir.path().join("test.db").display());
        let pool = crate::db::connect(&url).await.expect("connect");
        crate::db::migrate(&pool).await.expect("migrate");

        let now = Utc::now();
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

        let storage = Arc::new(Storage::at(dir.path().join("data")));
        storage.ensure_layout().await.expect("layout");
        let registry = Arc::new(Registry::new(pool.clone(), Arc::clone(&storage)));
        let repository = registry
            .ensure_repository("darktohka/site")
            .await
            .expect("repository");

        let uploads = Uploads::new(pool, storage, registry);
        (dir, uploads, repository.id)
    }

    #[tokio::test]
    async fn create_append_complete_round_trip() {
        let (_dir, uploads, repo_id) = harness().await;
        let upload = uploads.create(repo_id, None).await.expect("create");
        assert_eq!(upload.offset, 0);
        assert!(uploads.status(&upload.uuid).await.expect("status").is_some());

        let content = b"distributed upload payload".to_vec();
        let digest = Digest::from_bytes(&content);

        let size = uploads
            .append(&upload.uuid, 0, &content[..5])
            .await
            .expect("first chunk");
        assert_eq!(size, 5);
        let size = uploads
            .append(&upload.uuid, 5, &content[5..])
            .await
            .expect("second chunk");
        assert_eq!(size, content.len() as u64);

        let completed = uploads
            .complete(&upload.uuid, &digest)
            .await
            .expect("complete");
        assert_eq!(completed, content.len() as u64);
        assert!(uploads.status(&upload.uuid).await.expect("status").is_none());
        assert_eq!(
            uploads
                .registry
                .blob_stat(&digest)
                .await
                .expect("blob")
                .map(|blob| blob.size),
            Some(content.len() as i64)
        );
        assert!(uploads
            .registry
            .blob_in_repository(repo_id, &digest)
            .await
            .expect("linked"));
    }

    #[tokio::test]
    async fn complete_rejects_wrong_digest() {
        let (_dir, uploads, repo_id) = harness().await;
        let upload = uploads.create(repo_id, None).await.expect("create");
        uploads
            .append(&upload.uuid, 0, b"actual bytes")
            .await
            .expect("append");

        let wrong = Digest::from_bytes(b"expected bytes");
        assert!(uploads.complete(&upload.uuid, &wrong).await.is_err());
        assert!(uploads
            .registry
            .blob_stat(&wrong)
            .await
            .expect("blob")
            .is_none());
        assert!(uploads.status(&upload.uuid).await.expect("status").is_some());
    }

    #[tokio::test]
    async fn append_requires_contiguous_offsets() {
        let (_dir, uploads, repo_id) = harness().await;
        let upload = uploads.create(repo_id, None).await.expect("create");
        uploads.append(&upload.uuid, 0, b"abc").await.expect("append");
        assert!(uploads.append(&upload.uuid, 9, b"def").await.is_err());
    }

    #[tokio::test]
    async fn cancel_removes_session_and_files() {
        let (_dir, uploads, repo_id) = harness().await;
        let upload = uploads.create(repo_id, None).await.expect("create");
        uploads.append(&upload.uuid, 0, b"abc").await.expect("append");
        let data_path = uploads
            .storage
            .upload_data_path(&upload.uuid)
            .expect("path");
        assert!(data_path.exists());

        uploads.cancel(&upload.uuid).await.expect("cancel");
        assert!(uploads.status(&upload.uuid).await.expect("status").is_none());
        assert!(!data_path.exists());
    }

    #[tokio::test]
    async fn cleanup_stale_removes_old_uploads() {
        let (_dir, uploads, repo_id) = harness().await;
        let fresh = uploads.create(repo_id, None).await.expect("fresh");
        let stale = uploads.create(repo_id, None).await.expect("stale");

        let long_ago = Utc::now() - Duration::hours(2);
        sqlx::query("UPDATE uploads SET updated_at = ? WHERE uuid = ?")
            .bind(long_ago)
            .bind(&stale.uuid)
            .execute(&uploads.db)
            .await
            .expect("age upload");

        let removed = uploads.cleanup_stale(3600).await.expect("cleanup");
        assert_eq!(removed, 1);
        assert!(uploads.status(&stale.uuid).await.expect("status").is_none());
        assert!(uploads.status(&fresh.uuid).await.expect("status").is_some());
    }

    #[tokio::test]
    async fn append_to_unknown_upload_fails() {
        let (_dir, uploads, _repo_id) = harness().await;
        let err = uploads.append("missing", 0, b"x").await.unwrap_err();
        assert_eq!(err.code, crate::error::ErrorCode::BlobUploadUnknown);
    }
}
