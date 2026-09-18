# Lighthouse — Database

SQLite managed through `sqlx`, with migrations in `backend/migrations/`.

## Connection settings

| Setting | Value | Why |
|---|---|---|
| `journal_mode` | `WAL` | concurrent readers alongside one writer |
| `busy_timeout` | `5000` ms | a writer waits instead of failing with `SQLITE_BUSY` |
| `foreign_keys` | `ON` | cascades keep link tables consistent |
| `synchronous` | `NORMAL` | durable enough for WAL with far fewer fsyncs |
| pool size | 8 | readers dominate; writes are short |

`db::with_busy_retry` additionally retries lock contention with exponential
backoff for operations that can still collide.

## Conventions

- Timestamps are ISO-8601 UTC strings (`YYYY-MM-DDTHH:MM:SS.sssZ`) mapped to
  `chrono::DateTime<Utc>`.
- Usernames, e-mails and namespace names use `COLLATE NOCASE` uniqueness.
- A repository `name` is the full registry path (`alice/team/api`); the owning
  namespace is always the first path segment.
- Integer columns are `i64` — SQLite has no unsigned type.

## Tables

### Identity
| Table | Purpose |
|---|---|
| `users` | accounts, Argon2id hash, profile, verification flag, TOTP state |
| `sessions` | refresh-token families; `id` is also the access-token `jti` |
| `email_tokens` | single-use verification / password-reset tokens |
| `totp_backup_codes` | single-use Argon2id-hashed 2FA recovery codes |
| `app_passwords` | SHA-256-hashed registry credentials for human accounts |
| `login_events` | password, service-token, app-password, two-factor and refresh usage history |

### Namespaces and repositories
| Table | Purpose |
|---|---|
| `namespaces` | users and workspaces share one unique naming pool |
| `namespace_members` | extra members of a workspace |
| `repositories` | images, keyed by full path with namespace FK |

### Content
| Table | Purpose |
|---|---|
| `blobs` | content-addressed metadata (digest, size, media type) |
| `blob_repositories` | which repositories link a blob |
| `manifests` | manifest/index metadata plus the exact JSON body |
| `manifest_repositories` | which repositories link a manifest |
| `manifest_blobs` | reference edges manifest → config/layer blobs (GC) |
| `manifest_children` | reference edges index → child manifests, with platform |
| `tags` | tag → manifest pointers per repository |
| `uploads` / `upload_chunks` | in-progress upload sessions and received ranges |

### Authorization
| Table | Purpose |
|---|---|
| `namespace_permissions` | grants on a namespace (user or anonymous) |
| `repository_permissions` | grants on a single image |
| `service_accounts` | Basic-auth machine credentials (hash + prefix/suffix) |
| `service_account_grants` | namespace- or repository-scoped grants |
| `service_account_ip_ranges` | per-account CIDR allowlist entries; empty means unrestricted |

### Observability
| Table | Purpose |
|---|---|
| `activity` | timeline entries for the dashboard and profile heatmap |
| `registry_events` | audit log of every push/pull/delete/mount with actor, IP, UA |
| `pull_events` | raw pull records backing analytics |
| `pull_stats` | daily per-repository/per-tag pull rollups |
| `follows` | social graph |

### Registry refresh tokens
| Table | Purpose |
|---|---|
| `registry_refresh_tokens` | offline registry tokens (`identitytoken`); one stable token per login |

| Column | Type | Notes |
|---|---|---|
| `id` | INTEGER | primary key, autoincrement |
| `token_hash` | TEXT | SHA-256 of the presented secret; the plaintext is never stored |
| `credential_source` | TEXT | `password`, `app_password` or `service_account` |
| `user_id` | INTEGER | FK `users(id)`, `ON DELETE CASCADE` |
| `app_password_id` | INTEGER | FK `app_passwords(id)`, `ON DELETE CASCADE` |
| `service_account_id` | INTEGER | FK `service_accounts(id)`, `ON DELETE CASCADE` |
| `subject` | INTEGER | identity copied into the bearer token (`0` for a service account) |
| `username` | TEXT | username copied into the bearer token |
| `scope` | TEXT | original scope strings, space-joined; empty when issued without one |
| `client_id` | TEXT | client identifier recorded at issuance |
| `created_at` | TEXT | issuance time |
| `expires_at` | TEXT | end of life (`REGISTRY_REFRESH_TOKEN_TTL_SECS`) |
| `last_used_at` | TEXT | last redemption |
| `revoked_at` | TEXT | set on revocation; checked on every redemption |

Indexes:

| Index | Columns | Kind |
|---|---|---|
| `idx_registry_refresh_tokens_hash` | `token_hash` | unique |
| `idx_registry_refresh_tokens_user` | `user_id` | |
| `idx_registry_refresh_tokens_app_password` | `app_password_id` | |
| `idx_registry_refresh_tokens_service_account` | `service_account_id` | |

### Service-account IP ranges
| Table | Purpose |
|---|---|
| `service_account_ip_ranges` | normalized CIDRs a service account may authenticate from |

| Column | Type | Notes |
|---|---|---|
| `id` | INTEGER | primary key, autoincrement |
| `service_account_id` | INTEGER | FK `service_accounts(id)`, `ON DELETE CASCADE` |
| `cidr` | TEXT | normalized CIDR; bare addresses become host routes (`/32`, `/128`) |
| `created_at` | TEXT | insertion time |

Indexes:

| Index | Columns | Kind |
|---|---|---|
| `idx_service_account_ip_ranges_unique` | `service_account_id`, `cidr` | unique |

## Reference graph

```
tag ──▶ manifest ──▶ manifest_blobs ──▶ blob (config | layer)
              └────▶ manifest_children ──▶ child manifest (index/list)
```

Reachability over these edges drives the garbage collector: a blob is kept only
while some reachable manifest references it, and a manifest is kept only while a
tag, a repository link, or a parent index references it.

## Migrations

Migrations run in filename order at startup.

- `0002_auth_factors.sql` adds the TOTP columns on `users` (`totp_secret`,
  `totp_enabled`, `totp_confirmed_at`, `totp_last_used_step`), creates
  `totp_backup_codes` and `app_passwords`, and widens the `login_events.kind`
  CHECK to include `app_password`, `two_factor` and `registry_token`. Because
  `login_events` is a leaf child table, the migration rebuilds it inside the
  migration transaction.
- `0003_registry_refresh_tokens.sql` creates `registry_refresh_tokens` for
  offline registry tokens, with a unique hash index plus user, app-password and
  service-account lookup indexes.
- `0004_service_account_ip_ranges.sql` creates `service_account_ip_ranges` for
  per-service-account IP allowlists, with a unique `(service_account_id, cidr)`
  index.

## Maintenance

- GC runs on demand from the control plane after deletions; it never touches
  `uploads`.
- Stale upload sessions are reclaimed by `storage::upload::cleanup_stale` using
  `UPLOAD_SESSION_TTL_SECS`.
- Back up by copying `database/` and `data/` while the service is stopped.
