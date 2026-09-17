-- Lighthouse — two-factor authentication, app passwords and registry tokens.
--
-- `users` is only ever ALTERed: it has inbound foreign keys from many tables, so
-- a table rebuild would be unsafe. `login_events` is a leaf child table (it
-- references others, nothing references it), so it can be rebuilt to widen its
-- `kind` CHECK — SQLite cannot alter a CHECK constraint in place.
--
-- sqlx wraps a migration and its `_sqlx_migrations` bookkeeping in one
-- transaction, so this file must not rely on `PRAGMA foreign_keys` (a no-op
-- inside a transaction). None is needed: the rebuilt table has no inbound FKs.

-------------------------------------------------------------------------------
-- TOTP enrolment state (one factor per account)
-------------------------------------------------------------------------------

-- Base32 secret. NULL until an enrolment is started. Stored at rest because the
-- server must recompute codes; only `POST /api/auth/2fa/setup` ever returns it.
ALTER TABLE users ADD COLUMN totp_secret TEXT;
ALTER TABLE users ADD COLUMN totp_enabled INTEGER NOT NULL DEFAULT 0;
ALTER TABLE users ADD COLUMN totp_confirmed_at TEXT;
-- Last accepted TOTP time step, so a code cannot be replayed inside its window.
ALTER TABLE users ADD COLUMN totp_last_used_step INTEGER;

-- One-time backup codes. Only an Argon2id hash is stored; the plaintext is
-- returned exactly once, when the codes are generated.
CREATE TABLE totp_backup_codes (
    id         INTEGER PRIMARY KEY AUTOINCREMENT,
    user_id    INTEGER NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    code_hash  TEXT    NOT NULL,
    used_at    TEXT,
    created_at TEXT    NOT NULL
);

CREATE INDEX idx_totp_backup_codes_user ON totp_backup_codes (user_id);

-------------------------------------------------------------------------------
-- App passwords (per-user registry credentials that bypass TOTP)
-------------------------------------------------------------------------------

CREATE TABLE app_passwords (
    id            INTEGER PRIMARY KEY AUTOINCREMENT,
    user_id       INTEGER NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    name          TEXT    NOT NULL,
    -- first/last characters for display, never the whole secret
    token_prefix  TEXT    NOT NULL,
    token_suffix  TEXT    NOT NULL,
    -- SHA-256 of the high-entropy secret, indexed for O(1) lookup
    token_hash    TEXT    NOT NULL,
    created_at    TEXT    NOT NULL,
    last_used_at  TEXT
);

CREATE UNIQUE INDEX idx_app_passwords_user_name ON app_passwords (user_id, name);
CREATE INDEX idx_app_passwords_user_hash ON app_passwords (user_id, token_hash);

-------------------------------------------------------------------------------
-- Widen login_events.kind for app-password and two-factor logins
-------------------------------------------------------------------------------

CREATE TABLE login_events_new (
    id                  INTEGER PRIMARY KEY AUTOINCREMENT,
    user_id             INTEGER REFERENCES users (id) ON DELETE CASCADE,
    service_account_id  INTEGER REFERENCES service_accounts (id) ON DELETE SET NULL,
    username_attempted  TEXT,
    success             INTEGER NOT NULL,
    kind                TEXT    NOT NULL CHECK (kind IN (
        'password', 'service_token', 'refresh', 'app_password', 'two_factor', 'registry_token'
    )),
    ip                  TEXT,
    user_agent          TEXT,
    created_at          TEXT    NOT NULL
);

INSERT INTO login_events_new
    (id, user_id, service_account_id, username_attempted, success, kind, ip, user_agent, created_at)
SELECT
    id, user_id, service_account_id, username_attempted, success, kind, ip, user_agent, created_at
FROM login_events;

DROP TABLE login_events;

ALTER TABLE login_events_new RENAME TO login_events;

CREATE INDEX idx_login_events_user ON login_events (user_id, created_at DESC);
