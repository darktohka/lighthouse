# Lighthouse — Database

SQLite managed through `sqlx`, with migrations in `Backend/migrations/`.

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

### Observability
| Table | Purpose |
|---|---|
| `activity` | timeline entries for the dashboard and profile heatmap |
| `registry_events` | audit log of every push/pull/delete/mount with actor, IP, UA |
| `pull_events` | raw pull records backing analytics |
| `pull_stats` | daily per-repository/per-tag pull rollups |
| `follows` | social graph |

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

## Maintenance

- GC runs on demand from the control plane after deletions; it never touches
  `uploads`.
- Stale upload sessions are reclaimed by `storage::upload::cleanup_stale` using
  `UPLOAD_SESSION_TTL_SECS`.
- Back up by copying `database/` and `data/` while the service is stopped.
