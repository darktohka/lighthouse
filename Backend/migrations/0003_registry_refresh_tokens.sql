-- Lighthouse — registry refresh tokens ("offline tokens").
--
-- `docker login` requests an offline token and stores the returned
-- `refresh_token` as `identitytoken` in the client config; the daemon then
-- trades it for short-lived bearer tokens instead of re-sending the account
-- password. The Docker daemon reuses that same secret for the life of the login
-- (it does not persist a rotated one), so the token is stable: it is hashed at
-- rest, expires, and is revoked whenever the underlying credential changes.
--
-- The principal columns are nullable because a token can belong to a user, an
-- app password or a service account; `credential_source` records which.

CREATE TABLE registry_refresh_tokens (
    id                 INTEGER PRIMARY KEY AUTOINCREMENT,
    -- SHA-256 of the presented secret; the plaintext is never stored.
    token_hash         TEXT    NOT NULL,
    credential_source  TEXT    NOT NULL
                       CHECK (credential_source IN ('password', 'app_password', 'service_account')),
    user_id            INTEGER REFERENCES users (id) ON DELETE CASCADE,
    app_password_id    INTEGER REFERENCES app_passwords (id) ON DELETE CASCADE,
    service_account_id INTEGER REFERENCES service_accounts (id) ON DELETE CASCADE,
    -- Identity copied into the bearer token this refresh token mints.
    subject            INTEGER NOT NULL,
    username           TEXT    NOT NULL,
    -- Original scope strings, space-joined, bounding every later redemption.
    scope              TEXT    NOT NULL DEFAULT '',
    client_id          TEXT,
    created_at         TEXT    NOT NULL,
    expires_at         TEXT    NOT NULL,
    last_used_at       TEXT,
    revoked_at         TEXT
);

CREATE UNIQUE INDEX idx_registry_refresh_tokens_hash ON registry_refresh_tokens (token_hash);
CREATE INDEX idx_registry_refresh_tokens_user ON registry_refresh_tokens (user_id);
CREATE INDEX idx_registry_refresh_tokens_app_password ON registry_refresh_tokens (app_password_id);
CREATE INDEX idx_registry_refresh_tokens_service_account ON registry_refresh_tokens (service_account_id);
