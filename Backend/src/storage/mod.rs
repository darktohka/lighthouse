//! Filesystem primitives for the content store.
//!
//! Everything lives under `DATA_DIR` using a flat, digest-sharded layout:
//!
//! ```text
//! data/
//! ├── blobs/<algorithm>/<hex[0..2]>/<hex>/data
//! ├── uploads/<uuid>/data
//! ├── uploads/<uuid>/startedat
//! └── tmp/
//! ```
//!
//! The functions in this module only touch the filesystem; database bookkeeping
//! lives in [`crate::storage::registry`] and [`crate::storage::upload`].

#![allow(dead_code)]

pub mod gc;
pub mod registry;
pub mod upload;

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use chrono::{DateTime, Utc};
use tokio::fs::{self, File, OpenOptions};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use crate::config::Config;
use crate::error::RegistryError;
use crate::oci::digest::{Digest, Verifier};

/// Maximum accepted length of an upload identifier.
const MAX_UPLOAD_ID_LEN: usize = 128;

/// Read buffer used when streaming a finalized upload through its verifier.
const HASH_BUFFER_SIZE: usize = 64 * 1024;

#[derive(Clone)]
pub struct Storage {
    root: PathBuf,
}

impl Storage {
    /// Builds a store rooted at `config.data_dir`.
    pub fn new(config: &Config) -> Result<Self> {
        Ok(Self::at(config.data_dir.clone()))
    }

    /// Builds a store rooted at an explicit directory (used by tests).
    pub fn at(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    /// The configured `DATA_DIR`.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// `DATA_DIR/blobs`
    pub fn blobs_dir(&self) -> PathBuf {
        self.root.join("blobs")
    }

    /// `DATA_DIR/uploads`
    pub fn uploads_dir(&self) -> PathBuf {
        self.root.join("uploads")
    }

    /// `DATA_DIR/tmp`
    pub fn tmp_dir(&self) -> PathBuf {
        self.root.join("tmp")
    }

    /// Creates the top-level directories. Safe to call repeatedly.
    pub async fn ensure_layout(&self) -> Result<()> {
        fs::create_dir_all(self.blobs_dir())
            .await
            .context("creating blob directory")?;
        fs::create_dir_all(self.uploads_dir())
            .await
            .context("creating upload directory")?;
        fs::create_dir_all(self.tmp_dir())
            .await
            .context("creating temporary directory")?;
        Ok(())
    }

    /// `DATA_DIR/blobs/<algorithm>/<hex[0..2]>/<hex>`
    pub fn blob_dir(&self, digest: &Digest) -> PathBuf {
        let encoded = digest.encoded();
        let shard = &encoded[..encoded.len().min(2)];
        self.blobs_dir()
            .join(digest.algorithm())
            .join(shard)
            .join(encoded)
    }

    /// `DATA_DIR/blobs/<algorithm>/<hex[0..2]>/<hex>/data`
    pub fn blob_path(&self, digest: &Digest) -> PathBuf {
        self.blob_dir(digest).join("data")
    }

    /// `DATA_DIR/uploads/<uuid>`, rejecting unsafe identifiers.
    pub fn upload_dir(&self, uuid: &str) -> Result<PathBuf> {
        if !is_valid_upload_id(uuid) {
            bail!("invalid upload identifier");
        }
        Ok(self.uploads_dir().join(uuid))
    }

    /// `DATA_DIR/uploads/<uuid>/data`
    pub fn upload_data_path(&self, uuid: &str) -> Result<PathBuf> {
        Ok(self.upload_dir(uuid)?.join("data"))
    }

    /// `DATA_DIR/uploads/<uuid>/startedat`
    pub fn upload_started_at_path(&self, uuid: &str) -> Result<PathBuf> {
        Ok(self.upload_dir(uuid)?.join("startedat"))
    }

    /// Creates the upload directory, an empty data file and the `startedat`
    /// marker.
    pub async fn create_upload(&self, uuid: &str, started_at: DateTime<Utc>) -> Result<()> {
        let dir = self.upload_dir(uuid)?;
        fs::create_dir_all(&dir)
            .await
            .context("creating upload directory")?;
        File::create(dir.join("data"))
            .await
            .context("creating upload data file")?;
        fs::write(dir.join("startedat"), started_at.to_rfc3339())
            .await
            .context("writing upload start marker")?;
        Ok(())
    }

    /// Opens a stored blob, or `None` when it is absent.
    pub async fn open_blob(&self, digest: &Digest) -> Result<Option<File>> {
        match File::open(self.blob_path(digest)).await {
            Ok(file) => Ok(Some(file)),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(err) => Err(err).context("opening blob"),
        }
    }

    /// Returns the on-disk size of a stored blob, or `None` when it is absent.
    pub async fn blob_size(&self, digest: &Digest) -> Result<Option<u64>> {
        match fs::metadata(self.blob_path(digest)).await {
            Ok(metadata) => Ok(Some(metadata.len())),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(err) => Err(err).context("reading blob metadata"),
        }
    }

    /// Atomically moves `src` into the blob store. When a blob with the same
    /// digest already exists, `src` is dropped and the existing size returned.
    pub async fn put_blob_from_path(&self, src: &Path, digest: &Digest) -> Result<u64> {
        if let Some(size) = self.blob_size(digest).await? {
            let _ = fs::remove_file(src).await;
            return Ok(size);
        }

        let destination = self.blob_path(digest);
        if let Some(parent) = destination.parent() {
            fs::create_dir_all(parent)
                .await
                .context("creating blob shard directory")?;
        }
        fs::rename(src, &destination)
            .await
            .context("moving blob into the store")?;

        self.blob_size(digest)
            .await?
            .context("blob missing immediately after move")
    }

    /// Removes a blob and its shard directory. Missing blobs are not an error.
    pub async fn delete_blob(&self, digest: &Digest) -> Result<()> {
        match fs::remove_dir_all(self.blob_dir(digest)).await {
            Ok(()) => Ok(()),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(err) => Err(err).context("deleting blob"),
        }
    }

    /// Appends `data` to an upload, requiring `offset` to equal the current
    /// durable size so gaps and rewrites are rejected.
    pub async fn write_upload_chunk(&self, uuid: &str, offset: u64, data: &[u8]) -> Result<()> {
        let path = self.upload_data_path(uuid)?;
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .await
            .context("opening upload data file")?;

        let current = file.metadata().await.context("stat upload data")?.len();
        if current != offset {
            return Err(RegistryError::upload_invalid(format!(
                "upload offset mismatch: expected {current}, got {offset}"
            ))
            .into());
        }

        file.write_all(data).await.context("writing upload chunk")?;
        file.sync_data().await.context("syncing upload chunk")?;
        Ok(())
    }

    /// Verifies a finished upload against `digest` and moves it into the blob
    /// store. The upload directory is always removed.
    pub async fn finalize_upload(&self, uuid: &str, digest: &Digest) -> Result<u64> {
        let dir = self.upload_dir(uuid)?;
        let data_path = dir.join("data");
        let mut file = File::open(&data_path)
            .await
            .context("opening upload for finalization")?;

        let mut verifier = digest.verifier();
        let mut buffer = vec![0u8; HASH_BUFFER_SIZE];
        let mut size = 0u64;
        loop {
            let read = file.read(&mut buffer).await.context("hashing upload")?;
            if read == 0 {
                break;
            }
            verifier.update(&buffer[..read]);
            size += read as u64;
        }
        drop(file);

        if !verifier.matches() {
            return Err(RegistryError::digest_invalid(&digest.to_string()).into());
        }

        let destination = self.blob_path(digest);
        if let Some(parent) = destination.parent() {
            fs::create_dir_all(parent)
                .await
                .context("creating blob shard directory")?;
        }

        if fs::try_exists(&destination)
            .await
            .context("checking blob destination")?
        {
            fs::remove_dir_all(&dir)
                .await
                .context("discarding duplicate upload")?;
        } else {
            fs::rename(&data_path, &destination)
                .await
                .context("moving finalized upload into the store")?;
            fs::remove_dir_all(&dir)
                .await
                .context("removing upload directory")?;
        }

        Ok(size)
    }

    /// Removes an upload directory and all of its files.
    pub async fn discard_upload(&self, uuid: &str) -> Result<()> {
        let dir = self.upload_dir(uuid)?;
        match fs::remove_dir_all(&dir).await {
            Ok(()) => Ok(()),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(err) => Err(err).context("discarding upload"),
        }
    }
}

/// True when `uuid` is safe to use as a single path component.
pub fn is_valid_upload_id(uuid: &str) -> bool {
    !uuid.is_empty()
        && uuid.len() <= MAX_UPLOAD_ID_LEN
        && uuid != "."
        && uuid != ".."
        && uuid
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'=' | b'-'))
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn temp_storage() -> (tempfile::TempDir, Storage) {
        let dir = tempfile::tempdir().expect("tempdir");
        let storage = Storage::at(dir.path());
        storage.ensure_layout().await.expect("layout");
        (dir, storage)
    }

    #[tokio::test]
    async fn ensure_layout_creates_directories() {
        let (_dir, storage) = temp_storage().await;
        assert!(storage.blobs_dir().is_dir());
        assert!(storage.uploads_dir().is_dir());
        assert!(storage.tmp_dir().is_dir());
    }

    #[tokio::test]
    async fn blob_round_trip() {
        let (_dir, storage) = temp_storage().await;
        let content = b"a blob payload".repeat(32);
        let digest = Digest::from_bytes(&content);

        let source = storage.tmp_dir().join("incoming");
        fs::write(&source, &content).await.expect("write source");

        let size = storage
            .put_blob_from_path(&source, &digest)
            .await
            .expect("put blob");
        assert_eq!(size, content.len() as u64);
        assert_eq!(
            storage.blob_size(&digest).await.expect("size"),
            Some(content.len() as u64)
        );

        let mut file = storage
            .open_blob(&digest)
            .await
            .expect("open")
            .expect("present");
        let mut read_back = Vec::new();
        file.read_to_end(&mut read_back).await.expect("read");
        assert_eq!(read_back, content);

        storage.delete_blob(&digest).await.expect("delete");
        assert_eq!(storage.blob_size(&digest).await.expect("size"), None);
    }

    #[tokio::test]
    async fn put_blob_keeps_existing_and_drops_source() {
        let (_dir, storage) = temp_storage().await;
        let content = b"deduplicated".to_vec();
        let digest = Digest::from_bytes(&content);

        let first = storage.tmp_dir().join("first");
        fs::write(&first, &content).await.expect("write first");
        storage
            .put_blob_from_path(&first, &digest)
            .await
            .expect("first put");

        let second = storage.tmp_dir().join("second");
        fs::write(&second, &content).await.expect("write second");
        let size = storage
            .put_blob_from_path(&second, &digest)
            .await
            .expect("second put");

        assert_eq!(size, content.len() as u64);
        assert!(!second.exists());
    }

    #[tokio::test]
    async fn upload_chunks_are_contiguous_and_finalize() {
        let (_dir, storage) = temp_storage().await;
        let uuid = "upload-abc_123";
        storage
            .create_upload(uuid, Utc::now())
            .await
            .expect("create upload");

        let content = b"chunk one;chunk two;chunk three".to_vec();
        let digest = Digest::from_bytes(&content);
        storage
            .write_upload_chunk(uuid, 0, &content[..10])
            .await
            .expect("first chunk");
        storage
            .write_upload_chunk(uuid, 10, &content[10..20])
            .await
            .expect("second chunk");
        storage
            .write_upload_chunk(uuid, 20, &content[20..])
            .await
            .expect("third chunk");

        let size = storage
            .finalize_upload(uuid, &digest)
            .await
            .expect("finalize");
        assert_eq!(size, content.len() as u64);
        assert_eq!(storage.blob_size(&digest).await.expect("size"), Some(size));
    }

    #[tokio::test]
    async fn finalize_rejects_wrong_digest() {
        let (_dir, storage) = temp_storage().await;
        let uuid = "upload-wrong";
        storage
            .create_upload(uuid, Utc::now())
            .await
            .expect("create upload");
        storage
            .write_upload_chunk(uuid, 0, b"actual content")
            .await
            .expect("chunk");

        let wrong = Digest::from_bytes(b"expected content");
        assert!(storage.finalize_upload(uuid, &wrong).await.is_err());
        assert_eq!(storage.blob_size(&wrong).await.expect("size"), None);
    }

    #[tokio::test]
    async fn rejects_non_contiguous_chunks() {
        let (_dir, storage) = temp_storage().await;
        let uuid = "upload-gap";
        storage
            .create_upload(uuid, Utc::now())
            .await
            .expect("create upload");
        storage
            .write_upload_chunk(uuid, 0, b"abc")
            .await
            .expect("first chunk");
        assert!(storage.write_upload_chunk(uuid, 10, b"xyz").await.is_err());
    }

    #[test]
    fn rejects_unsafe_upload_identifiers() {
        assert!(is_valid_upload_id("abc-123_XYZ="));
        assert!(!is_valid_upload_id(""));
        assert!(!is_valid_upload_id("."));
        assert!(!is_valid_upload_id(".."));
        assert!(!is_valid_upload_id("../../etc/passwd"));
        assert!(!is_valid_upload_id(&"a".repeat(129)));
    }

    #[tokio::test]
    async fn upload_helpers_reject_traversal() {
        let (_dir, storage) = temp_storage().await;
        assert!(storage.upload_dir("../escape").is_err());
        assert!(storage.upload_data_path("..").is_err());
        assert!(storage.discard_upload("nonexistent-but-safe").await.is_ok());
    }

    #[test]
    fn blob_shard_handles_short_encodings() {
        let storage = Storage::at("/tmp/lighthouse-shard-test");
        let digest = Digest::parse("custom:a").expect("grammar valid");
        assert!(storage.blob_dir(&digest).ends_with("custom/a/a"));
    }
}
