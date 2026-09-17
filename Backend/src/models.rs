//! Shared SQLite row structs.
//!
//! Every struct maps one-to-one onto a table in `migrations/0001_init.sql`; the
//! derive-generated `FromRow` implementation matches columns by name, so field
//! names and nullability must stay in lockstep with the schema. Integer columns
//! are `i64` (SQLite has no unsigned integers) and timestamps are
//! `chrono::DateTime<Utc>` decoded from the ISO-8601 TEXT columns.

#![allow(dead_code)]

use chrono::{DateTime, Utc};

#[derive(Debug, Clone, sqlx::FromRow, serde::Serialize)]
pub struct User {
    pub id: i64,
    pub username: String,
    pub email: String,
    pub first_name: Option<String>,
    pub last_name: Option<String>,
    pub password_hash: String,
    pub email_verified: bool,
    pub is_admin: bool,
    pub bio: Option<String>,
    pub company: Option<String>,
    pub location: Option<String>,
    pub website: Option<String>,
    pub avatar_url: Option<String>,
    pub theme: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, sqlx::FromRow, serde::Serialize)]
pub struct Session {
    pub id: String,
    pub user_id: i64,
    pub refresh_token_hash: Option<String>,
    pub user_agent: Option<String>,
    pub ip: Option<String>,
    pub created_at: DateTime<Utc>,
    pub last_seen_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
    pub revoked_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, sqlx::FromRow, serde::Serialize)]
pub struct EmailToken {
    pub token: String,
    pub user_id: i64,
    pub kind: String,
    pub created_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
    pub used_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, sqlx::FromRow, serde::Serialize)]
pub struct Namespace {
    pub id: i64,
    pub name: String,
    pub kind: String,
    pub owner_user_id: Option<i64>,
    pub description: Option<String>,
    pub is_public: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, sqlx::FromRow, serde::Serialize)]
pub struct NamespaceMember {
    pub namespace_id: i64,
    pub user_id: i64,
    pub role: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, sqlx::FromRow, serde::Serialize)]
pub struct Repository {
    pub id: i64,
    pub namespace_id: i64,
    pub name: String,
    pub path: String,
    pub description: Option<String>,
    pub is_public: bool,
    pub created_by: Option<i64>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, sqlx::FromRow, serde::Serialize)]
pub struct Blob {
    pub id: i64,
    pub digest: String,
    pub size: i64,
    pub media_type: Option<String>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, sqlx::FromRow, serde::Serialize)]
pub struct Manifest {
    pub id: i64,
    pub digest: String,
    pub media_type: String,
    pub size: i64,
    pub artifact_type: Option<String>,
    pub content: Vec<u8>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, sqlx::FromRow, serde::Serialize)]
pub struct Tag {
    pub id: i64,
    pub repository_id: i64,
    pub name: String,
    pub manifest_id: i64,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, sqlx::FromRow, serde::Serialize)]
pub struct Upload {
    pub uuid: String,
    pub repository_id: i64,
    pub offset: i64,
    pub created_by: Option<i64>,
    pub started_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, sqlx::FromRow, serde::Serialize)]
pub struct UploadChunk {
    pub id: i64,
    pub upload_uuid: String,
    pub start_offset: i64,
    pub end_offset: i64,
}

#[derive(Debug, Clone, sqlx::FromRow, serde::Serialize)]
pub struct NamespacePermission {
    pub id: i64,
    pub namespace_id: i64,
    pub subject_type: String,
    pub subject_user_id: Option<i64>,
    pub can_pull: bool,
    pub can_push: bool,
    pub created_by: Option<i64>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, sqlx::FromRow, serde::Serialize)]
pub struct RepositoryPermission {
    pub id: i64,
    pub repository_id: i64,
    pub subject_type: String,
    pub subject_user_id: Option<i64>,
    pub can_pull: bool,
    pub can_push: bool,
    pub created_by: Option<i64>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, sqlx::FromRow, serde::Serialize)]
pub struct ServiceAccount {
    pub id: i64,
    pub owner_user_id: i64,
    pub name: String,
    pub username: String,
    pub description: Option<String>,
    pub token_prefix: String,
    pub token_suffix: String,
    pub token_hash: String,
    pub created_at: DateTime<Utc>,
    pub last_used_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, sqlx::FromRow, serde::Serialize)]
pub struct ServiceAccountGrant {
    pub id: i64,
    pub service_account_id: i64,
    pub namespace_id: Option<i64>,
    pub repository_id: Option<i64>,
    pub can_pull: bool,
    pub can_push: bool,
}

#[derive(Debug, Clone, sqlx::FromRow, serde::Serialize)]
pub struct ServiceAccountIpRange {
    pub id: i64,
    pub service_account_id: i64,
    pub cidr: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, sqlx::FromRow, serde::Serialize)]
pub struct AppPassword {
    pub id: i64,
    pub user_id: i64,
    pub name: String,
    pub token_prefix: String,
    pub token_suffix: String,
    pub token_hash: String,
    pub created_at: DateTime<Utc>,
    pub last_used_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, sqlx::FromRow, serde::Serialize)]
pub struct LoginEvent {
    pub id: i64,
    pub user_id: Option<i64>,
    pub service_account_id: Option<i64>,
    pub username_attempted: Option<String>,
    pub success: bool,
    pub kind: String,
    pub ip: Option<String>,
    pub user_agent: Option<String>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, sqlx::FromRow, serde::Serialize)]
pub struct Activity {
    pub id: i64,
    pub actor_user_id: Option<i64>,
    pub namespace_id: Option<i64>,
    pub repository_id: Option<i64>,
    pub kind: String,
    pub summary: String,
    pub metadata: Option<String>,
    pub is_public: bool,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, sqlx::FromRow, serde::Serialize)]
pub struct RegistryEvent {
    pub id: i64,
    pub action: String,
    pub repository_id: Option<i64>,
    pub repository_name: Option<String>,
    pub reference: Option<String>,
    pub digest: Option<String>,
    pub media_type: Option<String>,
    pub user_id: Option<i64>,
    pub service_account_id: Option<i64>,
    pub is_anonymous: bool,
    pub ip: Option<String>,
    pub user_agent: Option<String>,
    pub status: Option<i64>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, sqlx::FromRow, serde::Serialize)]
pub struct PullEvent {
    pub id: i64,
    pub repository_id: i64,
    pub manifest_id: Option<i64>,
    pub tag_name: Option<String>,
    pub digest: Option<String>,
    pub user_id: Option<i64>,
    pub is_anonymous: bool,
    pub ip: Option<String>,
    pub user_agent: Option<String>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, sqlx::FromRow, serde::Serialize)]
pub struct PullStat {
    pub id: i64,
    pub repository_id: i64,
    pub tag_name: Option<String>,
    pub day: String,
    pub pulls: i64,
}

#[derive(Debug, Clone, sqlx::FromRow, serde::Serialize)]
pub struct Follow {
    pub follower_user_id: i64,
    pub followed_user_id: i64,
    pub created_at: DateTime<Utc>,
}
