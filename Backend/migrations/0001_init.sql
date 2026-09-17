-- Lighthouse registry — initial schema.
--
-- Conventions:
--   * Timestamps are ISO-8601 UTC strings ("YYYY-MM-DDTHH:MM:SS.SSSZ") so that
--     sqlx can map them to `chrono::DateTime<Utc>`.
--   * Usernames/emails/namespace names use COLLATE NOCASE uniqueness so that
--     "Darktohka" and "darktohka" collide.
--   * Repository `name` is the full registry path, e.g. "darktohka/team/api".
--     The owning namespace is always the first path segment.

-------------------------------------------------------------------------------
-- Users & identity
-------------------------------------------------------------------------------

CREATE TABLE users (
    id              INTEGER PRIMARY KEY AUTOINCREMENT,
    username        TEXT    NOT NULL COLLATE NOCASE,
    email           TEXT    NOT NULL COLLATE NOCASE,
    first_name      TEXT,
    last_name       TEXT,
    password_hash   TEXT    NOT NULL,
    email_verified  INTEGER NOT NULL DEFAULT 0,
    is_admin        INTEGER NOT NULL DEFAULT 0,
    -- profile
    bio             TEXT,
    company         TEXT,
    location        TEXT,
    website         TEXT,
    avatar_url      TEXT,
    -- preferences
    theme           TEXT    NOT NULL DEFAULT 'light',
    created_at      TEXT    NOT NULL,
    updated_at      TEXT    NOT NULL
);

CREATE UNIQUE INDEX idx_users_username ON users (username COLLATE NOCASE);
CREATE UNIQUE INDEX idx_users_email    ON users (email COLLATE NOCASE);

-- Web sessions. The opaque `id` doubles as the refresh-token family id; the
-- access token is a stateless JWT carried in an httpOnly cookie.
CREATE TABLE sessions (
    id                  TEXT    PRIMARY KEY,
    user_id             INTEGER NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    refresh_token_hash  TEXT,
    user_agent          TEXT,
    ip                  TEXT,
    created_at          TEXT    NOT NULL,
    last_seen_at        TEXT    NOT NULL,
    expires_at          TEXT    NOT NULL,
    revoked_at          TEXT
);

CREATE INDEX idx_sessions_user ON sessions (user_id);
CREATE INDEX idx_sessions_expires ON sessions (expires_at);

-- Single-use, short-lived tokens for e-mail verification and password reset.
CREATE TABLE email_tokens (
    token       TEXT    PRIMARY KEY,
    user_id     INTEGER NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    kind        TEXT    NOT NULL CHECK (kind IN ('verify_email', 'reset_password')),
    created_at  TEXT    NOT NULL,
    expires_at  TEXT    NOT NULL,
    used_at     TEXT
);

CREATE INDEX idx_email_tokens_user ON email_tokens (user_id, kind);

-------------------------------------------------------------------------------
-- Namespaces (users + workspaces share one unique naming pool)
-------------------------------------------------------------------------------

CREATE TABLE namespaces (
    id             INTEGER PRIMARY KEY AUTOINCREMENT,
    name           TEXT    NOT NULL COLLATE NOCASE,
    kind           TEXT    NOT NULL CHECK (kind IN ('user', 'workspace')),
    owner_user_id  INTEGER REFERENCES users (id) ON DELETE CASCADE,
    description    TEXT,
    is_public      INTEGER NOT NULL DEFAULT 0,
    created_at     TEXT    NOT NULL,
    updated_at     TEXT    NOT NULL
);

CREATE UNIQUE INDEX idx_namespaces_name ON namespaces (name COLLATE NOCASE);
CREATE INDEX idx_namespaces_owner ON namespaces (owner_user_id);

-- Extra members of a workspace (the owner is implicit via owner_user_id).
CREATE TABLE namespace_members (
    namespace_id  INTEGER NOT NULL REFERENCES namespaces (id) ON DELETE CASCADE,
    user_id       INTEGER NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    role          TEXT    NOT NULL DEFAULT 'member' CHECK (role IN ('admin', 'member')),
    created_at    TEXT    NOT NULL,
    PRIMARY KEY (namespace_id, user_id)
);

-------------------------------------------------------------------------------
-- Repositories
-------------------------------------------------------------------------------

CREATE TABLE repositories (
    id            INTEGER PRIMARY KEY AUTOINCREMENT,
    namespace_id  INTEGER NOT NULL REFERENCES namespaces (id) ON DELETE CASCADE,
    -- full registry path: "<namespace>/<path>"
    name          TEXT    NOT NULL COLLATE NOCASE,
    -- the path portion after the namespace segment
    path          TEXT    NOT NULL,
    description   TEXT,
    is_public     INTEGER NOT NULL DEFAULT 0,
    created_by    INTEGER REFERENCES users (id) ON DELETE SET NULL,
    created_at    TEXT    NOT NULL,
    updated_at    TEXT    NOT NULL
);

CREATE UNIQUE INDEX idx_repositories_name ON repositories (name COLLATE NOCASE);
CREATE INDEX idx_repositories_namespace ON repositories (namespace_id);

-------------------------------------------------------------------------------
-- Blobs (content-addressed, deduplicated by digest)
-------------------------------------------------------------------------------

CREATE TABLE blobs (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    digest      TEXT    NOT NULL,
    size        INTEGER NOT NULL,
    media_type  TEXT,
    created_at  TEXT    NOT NULL
);

CREATE UNIQUE INDEX idx_blobs_digest ON blobs (digest);

-- Which repositories reference a blob. Drives per-repo exclusivity and the
-- garbage collector (a blob with zero repository links is dangling).
CREATE TABLE blob_repositories (
    blob_id        INTEGER NOT NULL REFERENCES blobs (id) ON DELETE CASCADE,
    repository_id  INTEGER NOT NULL REFERENCES repositories (id) ON DELETE CASCADE,
    created_at     TEXT    NOT NULL,
    PRIMARY KEY (blob_id, repository_id)
);

CREATE INDEX idx_blob_repositories_repo ON blob_repositories (repository_id);

-------------------------------------------------------------------------------
-- Manifests (and OCI indexes / Docker manifest lists)
-------------------------------------------------------------------------------

CREATE TABLE manifests (
    id             INTEGER PRIMARY KEY AUTOINCREMENT,
    digest         TEXT    NOT NULL,
    media_type     TEXT    NOT NULL,
    size           INTEGER NOT NULL,
    artifact_type  TEXT,
    -- JSON blob of the manifest body, kept for fast rendering
    content        BLOB    NOT NULL,
    created_at     TEXT    NOT NULL
);

CREATE UNIQUE INDEX idx_manifests_digest ON manifests (digest);

CREATE TABLE manifest_repositories (
    manifest_id    INTEGER NOT NULL REFERENCES manifests (id) ON DELETE CASCADE,
    repository_id  INTEGER NOT NULL REFERENCES repositories (id) ON DELETE CASCADE,
    created_at     TEXT    NOT NULL,
    PRIMARY KEY (manifest_id, repository_id)
);

CREATE INDEX idx_manifest_repositories_repo ON manifest_repositories (repository_id);

-- Reference edges: manifest -> config/layer blobs (used by GC mark phase).
CREATE TABLE manifest_blobs (
    manifest_id  INTEGER NOT NULL REFERENCES manifests (id) ON DELETE CASCADE,
    blob_id      INTEGER NOT NULL REFERENCES blobs (id) ON DELETE CASCADE,
    -- 'config' | 'layer' | 'subject'
    role         TEXT    NOT NULL DEFAULT 'layer',
    PRIMARY KEY (manifest_id, blob_id)
);

CREATE INDEX idx_manifest_blobs_blob ON manifest_blobs (blob_id);

-- Reference edges: index/list -> child manifests, with platform info.
CREATE TABLE manifest_children (
    parent_manifest_id  INTEGER NOT NULL REFERENCES manifests (id) ON DELETE CASCADE,
    child_manifest_id   INTEGER NOT NULL REFERENCES manifests (id) ON DELETE CASCADE,
    platform_os         TEXT,
    platform_architecture TEXT,
    platform_variant    TEXT,
    PRIMARY KEY (parent_manifest_id, child_manifest_id)
);

CREATE INDEX idx_manifest_children_child ON manifest_children (child_manifest_id);

CREATE TABLE tags (
    id             INTEGER PRIMARY KEY AUTOINCREMENT,
    repository_id  INTEGER NOT NULL REFERENCES repositories (id) ON DELETE CASCADE,
    name           TEXT    NOT NULL,
    manifest_id    INTEGER NOT NULL REFERENCES manifests (id) ON DELETE CASCADE,
    created_at     TEXT    NOT NULL,
    updated_at     TEXT    NOT NULL
);

CREATE UNIQUE INDEX idx_tags_repo_name ON tags (repository_id, name);
CREATE INDEX idx_tags_manifest ON tags (manifest_id);

-------------------------------------------------------------------------------
-- Upload sessions
-------------------------------------------------------------------------------

CREATE TABLE uploads (
    uuid           TEXT    PRIMARY KEY,
    repository_id  INTEGER NOT NULL REFERENCES repositories (id) ON DELETE CASCADE,
    -- total bytes durably received (== next expected offset)
    offset         INTEGER NOT NULL DEFAULT 0,
    created_by     INTEGER REFERENCES users (id) ON DELETE SET NULL,
    started_at     TEXT    NOT NULL,
    updated_at     TEXT    NOT NULL
);

CREATE INDEX idx_uploads_repo ON uploads (repository_id);
CREATE INDEX idx_uploads_updated ON uploads (updated_at);

-- Received byte ranges for an upload, enabling resumable/out-of-order PATCH.
CREATE TABLE upload_chunks (
    id             INTEGER PRIMARY KEY AUTOINCREMENT,
    upload_uuid    TEXT    NOT NULL REFERENCES uploads (uuid) ON DELETE CASCADE,
    start_offset   INTEGER NOT NULL,
    end_offset     INTEGER NOT NULL,
    UNIQUE (upload_uuid, start_offset)
);

-------------------------------------------------------------------------------
-- Authorization: namespace-level and repository-level grants
-------------------------------------------------------------------------------

CREATE TABLE namespace_permissions (
    id               INTEGER PRIMARY KEY AUTOINCREMENT,
    namespace_id     INTEGER NOT NULL REFERENCES namespaces (id) ON DELETE CASCADE,
    subject_type     TEXT    NOT NULL CHECK (subject_type IN ('user', 'anonymous')),
    subject_user_id  INTEGER REFERENCES users (id) ON DELETE CASCADE,
    can_pull         INTEGER NOT NULL DEFAULT 1,
    can_push         INTEGER NOT NULL DEFAULT 0,
    created_by       INTEGER REFERENCES users (id) ON DELETE SET NULL,
    created_at       TEXT    NOT NULL
);

CREATE UNIQUE INDEX idx_namespace_permissions_unique
    ON namespace_permissions (namespace_id, subject_type, COALESCE(subject_user_id, 0));

CREATE TABLE repository_permissions (
    id               INTEGER PRIMARY KEY AUTOINCREMENT,
    repository_id    INTEGER NOT NULL REFERENCES repositories (id) ON DELETE CASCADE,
    subject_type     TEXT    NOT NULL CHECK (subject_type IN ('user', 'anonymous')),
    subject_user_id  INTEGER REFERENCES users (id) ON DELETE CASCADE,
    can_pull         INTEGER NOT NULL DEFAULT 1,
    can_push         INTEGER NOT NULL DEFAULT 0,
    created_by       INTEGER REFERENCES users (id) ON DELETE SET NULL,
    created_at       TEXT    NOT NULL
);

CREATE UNIQUE INDEX idx_repository_permissions_unique
    ON repository_permissions (repository_id, subject_type, COALESCE(subject_user_id, 0));

-------------------------------------------------------------------------------
-- Service accounts (registry credentials scoped to namespaces/repos)
-------------------------------------------------------------------------------

CREATE TABLE service_accounts (
    id             INTEGER PRIMARY KEY AUTOINCREMENT,
    owner_user_id  INTEGER NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    name           TEXT    NOT NULL,
    -- login username, unique across the whole service-account pool and namespaces
    username       TEXT    NOT NULL COLLATE NOCASE,
    description    TEXT,
    token_prefix   TEXT    NOT NULL,
    token_suffix   TEXT    NOT NULL,
    token_hash     TEXT    NOT NULL,
    created_at     TEXT    NOT NULL,
    last_used_at   TEXT
);

CREATE UNIQUE INDEX idx_service_accounts_username ON service_accounts (username COLLATE NOCASE);
CREATE UNIQUE INDEX idx_service_accounts_owner_name ON service_accounts (owner_user_id, name);

CREATE TABLE service_account_grants (
    id                  INTEGER PRIMARY KEY AUTOINCREMENT,
    service_account_id  INTEGER NOT NULL REFERENCES service_accounts (id) ON DELETE CASCADE,
    namespace_id        INTEGER REFERENCES namespaces (id) ON DELETE CASCADE,
    repository_id       INTEGER REFERENCES repositories (id) ON DELETE CASCADE,
    can_pull            INTEGER NOT NULL DEFAULT 1,
    can_push            INTEGER NOT NULL DEFAULT 0
);

CREATE INDEX idx_service_account_grants_sa ON service_account_grants (service_account_id);

-- Login history, visible to the account owner. Covers password logins,
-- service-token logins and refresh-token use.
CREATE TABLE login_events (
    id                  INTEGER PRIMARY KEY AUTOINCREMENT,
    user_id             INTEGER REFERENCES users (id) ON DELETE CASCADE,
    service_account_id  INTEGER REFERENCES service_accounts (id) ON DELETE SET NULL,
    username_attempted  TEXT,
    success             INTEGER NOT NULL,
    kind                TEXT    NOT NULL CHECK (kind IN ('password', 'service_token', 'refresh')),
    ip                  TEXT,
    user_agent          TEXT,
    created_at          TEXT    NOT NULL
);

CREATE INDEX idx_login_events_user ON login_events (user_id, created_at DESC);

-------------------------------------------------------------------------------
-- Activity feed
-------------------------------------------------------------------------------

CREATE TABLE activity (
    id             INTEGER PRIMARY KEY AUTOINCREMENT,
    actor_user_id  INTEGER REFERENCES users (id) ON DELETE SET NULL,
    namespace_id   INTEGER REFERENCES namespaces (id) ON DELETE CASCADE,
    repository_id  INTEGER REFERENCES repositories (id) ON DELETE SET NULL,
    kind           TEXT    NOT NULL,
    summary        TEXT    NOT NULL,
    metadata       TEXT,
    is_public      INTEGER NOT NULL DEFAULT 0,
    created_at     TEXT    NOT NULL
);

CREATE INDEX idx_activity_actor ON activity (actor_user_id, created_at DESC);
CREATE INDEX idx_activity_namespace ON activity (namespace_id, created_at DESC);
CREATE INDEX idx_activity_repository ON activity (repository_id, created_at DESC);

-------------------------------------------------------------------------------
-- Registry operation log + pull statistics
-------------------------------------------------------------------------------

-- Audit log of every registry mutation/access, per the specification.
CREATE TABLE registry_events (
    id                  INTEGER PRIMARY KEY AUTOINCREMENT,
    action              TEXT    NOT NULL,
    repository_id       INTEGER REFERENCES repositories (id) ON DELETE SET NULL,
    repository_name     TEXT,
    reference           TEXT,
    digest              TEXT,
    media_type          TEXT,
    user_id             INTEGER REFERENCES users (id) ON DELETE SET NULL,
    service_account_id  INTEGER REFERENCES service_accounts (id) ON DELETE SET NULL,
    is_anonymous        INTEGER NOT NULL DEFAULT 0,
    ip                  TEXT,
    user_agent          TEXT,
    status              INTEGER,
    created_at          TEXT    NOT NULL
);

CREATE INDEX idx_registry_events_repo ON registry_events (repository_id, created_at DESC);
CREATE INDEX idx_registry_events_user ON registry_events (user_id, created_at DESC);
CREATE INDEX idx_registry_events_action ON registry_events (action, created_at DESC);

-- Raw pull events (retained for detailed analytics), and daily rollups.
CREATE TABLE pull_events (
    id             INTEGER PRIMARY KEY AUTOINCREMENT,
    repository_id  INTEGER NOT NULL REFERENCES repositories (id) ON DELETE CASCADE,
    manifest_id    INTEGER REFERENCES manifests (id) ON DELETE SET NULL,
    tag_name       TEXT,
    digest         TEXT,
    user_id        INTEGER REFERENCES users (id) ON DELETE SET NULL,
    is_anonymous   INTEGER NOT NULL DEFAULT 0,
    ip             TEXT,
    user_agent     TEXT,
    created_at     TEXT    NOT NULL
);

CREATE INDEX idx_pull_events_repo ON pull_events (repository_id, created_at DESC);
CREATE INDEX idx_pull_events_tag ON pull_events (repository_id, tag_name, created_at DESC);

CREATE TABLE pull_stats (
    id             INTEGER PRIMARY KEY AUTOINCREMENT,
    repository_id  INTEGER NOT NULL REFERENCES repositories (id) ON DELETE CASCADE,
    tag_name       TEXT,
    day            TEXT    NOT NULL,
    pulls          INTEGER NOT NULL DEFAULT 0
);

CREATE UNIQUE INDEX idx_pull_stats_unique
    ON pull_stats (repository_id, COALESCE(tag_name, ''), day);

-------------------------------------------------------------------------------
-- Social graph
-------------------------------------------------------------------------------

CREATE TABLE follows (
    follower_user_id  INTEGER NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    followed_user_id  INTEGER NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    created_at        TEXT    NOT NULL,
    PRIMARY KEY (follower_user_id, followed_user_id)
);

CREATE INDEX idx_follows_followed ON follows (followed_user_id);
