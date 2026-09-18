# Lighthouse — Architecture

Lighthouse is a self-hostable container registry: a Rust/Axum service that speaks
the **OCI Distribution Specification** (plus the legacy Docker v2 schema) and
serves a React control plane for users, namespaces, permissions, analytics and
image browsing.

---

## 1. Goals and constraints

| Requirement | Decision |
|---|---|
| Registry protocol | OCI Distribution Spec v1.1 + Docker Registry HTTP API V2 (schema2 + manifest list) |
| Runtime | Tokio multi-threaded, Axum 0.8 |
| Metadata | SQLite via `sqlx` (WAL, `busy_timeout=5000`, migrations) |
| Content | Flat file store under `DATA_DIR` (blobs/manifests/uploads), addressed by digest |
| Auth | Password (Argon2id) + JWT access cookie + refresh token + Basic-auth service tokens |
| E-mail | Brevo transactional API over `reqwest` |
| Bot defence | `pow-captcha-axum` (server) + `pow-captcha-react` (client) |
| Logging | `tracing` + daily-rotated `LOGS_DIR/YYYY-MM-DD.log`, stdout mirror |
| Deployment | Single static binary on `scratch`, flat host directories, no Docker volumes |

Non-goals: Helm charts, proxy/pull-through cache, notifications, replication.

---

## 2. Repository layout

```
/
├── backend/                 Rust service (binary: `registry`)
│   ├── Cargo.toml
│   ├── migrations/          sqlx migrations
│   └── src/
├── frontend/                Vite + React + TypeScript control plane
├── docs/                    All documentation (this folder)
├── Dockerfile               Multi-stage build → scratch
├── docker-compose.yaml      Host-directory deployment
└── .github/workflows/       Multi-arch image build & push
```

The two reference directories are inputs, never modified.

---

## 3. Backend module map

Each module below is the **single owner** of its files; parallel work never edits
another module's files, and `routes.rs` is the only integration point.

```
src/
├── main.rs                  boot: config → logging → dirs → db → migrate → serve; graceful shutdown
├── config.rs                Config::from_env()
├── error.rs                 ErrorCode, RegistryError (OCI envelope), ApiError (control-plane envelope)
├── logging.rs               tracing init (stdout + daily file), request_log middleware, client_ip/ua
├── db.rs                    pool (WAL/busy_timeout/FK), migrate(), with_busy_retry()
├── state.rs                 AppState, AuthContext
├── routes.rs                integration: mounts oci::router + api::router + static SPA fallback
├── models.rs                sqlx row structs shared by all modules
│
├── oci/                     OCI Distribution API  (owner: wave 2A)
│   ├── mod.rs               router for /v2/*
│   ├── digest.rs            Digest newtype: parse, algorithm, verify, from_bytes
│   ├── media_types.rs       media-type constants + Accept negotiation
│   ├── reference.rs         repository-name / tag / reference validation
│   ├── blobs.rs             GET/HEAD/DELETE /v2/<name>/blobs/<digest>
│   ├── uploads.rs           POST/PATCH/PUT/GET/DELETE upload lifecycle
│   ├── manifests.rs         GET/HEAD/PUT/DELETE /v2/<name>/manifests/<reference>
│   ├── tags.rs              GET /v2/<name>/tags/list
│   └── catalog.rs           GET /v2/_catalog
│
├── storage/                 content + metadata  (owner: wave 1)
│   ├── mod.rs               Storage: filesystem layout primitives
│   ├── registry.rs          Registry service: repositories/blobs/manifests/tags DB ops
│   ├── upload.rs            upload session files + chunk bookkeeping
│   └── gc.rs                mark-and-sweep reference graph collection
│
├── auth/                    identity  (owner: wave 2B)
│   ├── mod.rs               router for /api/auth/*
│   ├── password.rs          Argon2id hash/verify
│   ├── tokens.rs            JWT issue/verify, opaque token generation
│   ├── sessions.rs          cookie + refresh-token sessions, DB persistence
│   ├── middleware.rs        credential extraction → AuthContext (cookie | Basic | Bearer)
│   ├── handlers.rs          register/login/logout/verify/reset/refresh/me/captcha
│   └── service_accounts.rs  service-account CRUD + token auth
│
├── permissions.rs           namespace/repository authorization + delegation queries
├── email.rs                 Brevo client, e-mail templates
├── ratelimit.rs             in-memory per-IP/per-account limiters (login, register)
├── captcha.rs               proof-of-work challenge issue/verify
│
├── api/                     control plane  (owner: wave 2C)
│   ├── mod.rs               router for /api/*
│   ├── users.rs             profile, follow, heatmap, login history
│   ├── namespaces.rs        workspaces, membership, visibility
│   ├── repositories.rs      image + tag listing, detail, deletion, tag cleanup
│   ├── permissions.rs       delegation CRUD + user autocomplete
│   ├── service_accounts.rs  service-account management UI API
│   ├── analytics.rs         disk usage, unique/shared, largest tags, pull stats
│   ├── activity.rs          timeline feed
│   └── layers.rs            manifest/config JSON + layer filesystem browser
│
└── static_files.rs          serves the built Vite `dist/` with SPA fallback
```

Frontend ownership mirrors this: `frontend/src/{api,pages,components,lib}`.

---

## 4. Core contracts

These signatures are frozen; implementation lives in the owning module.

### 4.1 `AppState` (`state.rs`)

```rust
#[derive(Clone)]
pub struct AppState {
    pub config: Arc<Config>,
    pub db: Db,                     // sqlx::SqlitePool
    pub storage: Arc<Storage>,      // filesystem primitives
    pub registry: Arc<Registry>,    // DB-backed content service
    pub limiters: Arc<Limiters>,    // in-memory rate limiters
}

#[derive(Debug, Clone, Default)]
pub struct AuthContext {            // inserted as a request extension
    pub user_id: Option<i64>,
    pub username: Option<String>,
    pub service_account_id: Option<i64>,
    pub is_admin: bool,
}
```

### 4.2 `Digest` (`oci/digest.rs`)

Grammar: `^[a-z0-9]+(?:[+._-][a-z0-9]+)*:[a-zA-Z0-9=_-]+$`.
Known algorithms require lowercase hex of a fixed length (`sha256` 64, `sha512` 128).
Unknown but grammar-conforming algorithms are accepted (per spec).

```rust
pub struct Digest { /* algorithm + encoded */ }
impl Digest {
    pub fn parse(s: &str) -> Result<Self, RegistryError>;
    pub fn algorithm(&self) -> &str;
    pub fn encoded(&self) -> &str;
    pub fn from_bytes_sha256(bytes: &[u8]) -> Self;
    pub fn from_bytes(bytes: &[u8]) -> Self;            // canonical = sha256
    pub fn verify(&self, bytes: &[u8]) -> bool;
    pub fn verifier(&self) -> impl Verifier;            // streaming verification
}
```

### 4.3 `Storage` (`storage/mod.rs`) — filesystem only

```rust
pub struct Storage { /* DATA_DIR layout */ }
impl Storage {
    pub fn blob_path(&self, d: &Digest) -> PathBuf;     // blobs/sha256/ab/<hex>
    pub fn upload_dir(&self, uuid: &str) -> PathBuf;    // uploads/<uuid>
    pub fn upload_data_path(&self, uuid: &str) -> PathBuf;
    pub async fn ensure_layout(&self) -> Result<()>;
    pub async fn open_blob(&self, d: &Digest) -> Result<Option<tokio::fs::File>>;
    pub async fn blob_size(&self, d: &Digest) -> Result<Option<u64>>;
    pub async fn put_blob_from_path(&self, src: &Path, d: &Digest) -> Result<u64>;
    pub async fn delete_blob(&self, d: &Digest) -> Result<()>;
    pub async fn write_upload_chunk(&self, uuid: &str, offset: u64, data: &[u8]) -> Result<()>;
    pub async fn finalize_upload(&self, uuid: &str, d: &Digest) -> Result<u64>;
}
```

On-disk layout (flat directories, all under `DATA_DIR`):

```
data/
├── blobs/<algorithm>/<hex[0..2]>/<hex>/data
├── uploads/<uuid>/data
├── uploads/<uuid>/startedat
└── tmp/
```

### 4.4 `Registry` (`storage/registry.rs`) — DB-backed content service

```rust
pub struct Registry { db: Db, storage: Arc<Storage> }

impl Registry {
    // repositories
    pub async fn ensure_repository(&self, name: &str) -> Result<Repository>;
    pub async fn find_repository(&self, name: &str) -> Result<Option<Repository>>;
    pub async fn list_repository_names(&self, n: usize, last: Option<&str>) -> Result<Vec<String>>;

    // blobs
    pub async fn blob_stat(&self, d: &Digest) -> Result<Option<Blob>>;
    pub async fn register_blob(&self, d: &Digest, size: u64, media_type: Option<&str>) -> Result<Blob>;
    pub async fn link_blob(&self, repo_id: i64, d: &Digest) -> Result<bool>;   // false if already linked
    pub async fn blob_in_repository(&self, repo_id: i64, d: &Digest) -> Result<bool>;
    pub async fn mount_blob(&self, from_repo_id: i64, to_repo_id: i64, d: &Digest) -> Result<bool>;

    // manifests
    pub async fn manifest(&self, d: &Digest) -> Result<Option<Manifest>>;
    pub async fn put_manifest(&self, repo_id: i64, media_type: &str, content: &[u8]) -> Result<Manifest>;
    pub async fn manifest_in_repository(&self, repo_id: i64, d: &Digest) -> Result<bool>;
    pub async fn resolve_tag(&self, repo_id: i64, tag: &str) -> Result<Option<Digest>>;
    pub async fn set_tag(&self, repo_id: i64, tag: &str, d: &Digest) -> Result<()>;
    pub async fn delete_tag(&self, repo_id: i64, tag: &str) -> Result<bool>;
    pub async fn delete_manifest(&self, repo_id: i64, d: &Digest) -> Result<bool>;
    pub async fn list_tags(&self, repo_id: i64, n: usize, last: Option<&str>) -> Result<Vec<String>>;

    // reference graph
    pub async fn record_manifest_references(&self, manifest_id: i64, content: &[u8], media_type: &str) -> Result<()>;
}
```

### 4.5 Errors

- `RegistryError` → `{"errors":[{"code":"…","message":"…","detail":…}]}` with the
  status from `ErrorCode::status()`.
- `ApiError` → `{"error":{"code":"…","message":"…"}}}`.
- Status mapping matches the Go reference:

| code | HTTP | code | HTTP |
|---|---|---|---|
| `UNKNOWN` | 500 | `MANIFEST_INVALID` | 400 |
| `UNSUPPORTED` | 405 | `MANIFEST_UNVERIFIED` | 400 |
| `UNAUTHORIZED` | 401 | `MANIFEST_BLOB_UNKNOWN` | 400 |
| `DENIED` | 403 | `BLOB_UNKNOWN` | 404 |
| `UNAVAILABLE` | 503 | `BLOB_UPLOAD_UNKNOWN` | 404 |
| `TOOMANYREQUESTS` | 429 | `BLOB_UPLOAD_INVALID` | 404 |
| `DIGEST_INVALID` | 400 | `NAME_UNKNOWN` | 404 |
| `SIZE_INVALID` | 400 | `MANIFEST_UNKNOWN` | 404 |
| `RANGE_INVALID` | 416 | `PAGINATION_NUMBER_INVALID` | 400 |
| `NAME_INVALID` / `TAG_INVALID` | 400 | | |

---

## 5. HTTP surface

### 5.1 Registry (`/v2/*`)

Always: `Docker-Distribution-API-Version: registry/2.0` on every response.

| Method | Path | Success | Key headers |
|---|---|---|---|
| *any* | `/v2/` | 200 `{}` | `Content-Type: application/json` |
| GET | `/v2/<name>/tags/list` | 200 `{name,tags}` | `Link` when paginated |
| GET/HEAD | `/v2/<name>/manifests/<ref>` | 200 | `Content-Type`, `Content-Length`, `Docker-Content-Digest`, `ETag`, `304` on `If-None-Match` |
| PUT | `/v2/<name>/manifests/<ref>` | 201 | `Location` (by digest), `Docker-Content-Digest` |
| DELETE | `/v2/<name>/manifests/<ref>` | 202 | — |
| GET/HEAD | `/v2/<name>/blobs/<digest>` | 200/206/304 | `Content-Length`, `Content-Type`, `Docker-Content-Digest`, `ETag`, `Accept-Ranges`, `Content-Range` |
| DELETE | `/v2/<name>/blobs/<digest>` | 202 | `Content-Length: 0` |
| POST | `/v2/<name>/blobs/uploads/` | 202 / 201 (mount) | `Location`, `Range: 0-0`, `Docker-Upload-UUID`, `Content-Length: 0` |
| GET/HEAD | `/v2/<name>/blobs/uploads/<uuid>` | 204 | `Range: 0-<size-1>`, `Location`, `Docker-Upload-UUID` |
| PATCH | `/v2/<name>/blobs/uploads/<uuid>` | 202 | `Range`, `Location`, `Docker-Upload-UUID` |
| PUT | `/v2/<name>/blobs/uploads/<uuid>?digest=` | 201 | `Location`, `Docker-Content-Digest` |
| DELETE | `/v2/<name>/blobs/uploads/<uuid>` | 204 | `Docker-Upload-UUID` |
| GET | `/v2/_catalog` | 200 `{repositories}` | `Link` when paginated |

Rules:

- Pagination: `?n=` (default 100, max 1000, `n<0` or non-integer → 400
  `PAGINATION_NUMBER_INVALID`), `?last=`; emit
  `Link: </v2/…?n=…&last=…>; rel="next"` only when more entries remain.
- Manifest `Accept` negotiation: serve the stored OCI media type when acceptable;
  if the stored type is OCI and the client does not accept it → 404
  `MANIFEST_UNKNOWN`; if a Docker manifest list is requested without list support,
  fall back to the default `linux/amd64` manifest; never emit 406.
- Path parameters accept the full OCI grammar, including nested names
  (`darktohka/more/complicated/project2`).
- Unmatched `/v2/*` returns an OCI 404 envelope, never the SPA.

### 5.2 Control plane (`/api/*`)

JSON only, cookie-authenticated. Groups: `auth`, `users`, `namespaces`,
`repositories`, `permissions`, `service-accounts`, `analytics`, `activity`,
`layers`. Full endpoint list lives in `docs/API.md`.

---

## 6. Identity, permissions, naming

- **Namespaces** are the first path segment of a repository name. Users and
  workspaces share one unique naming pool (`namespaces.name`), so a workspace may
  not shadow a username.
- Default is **private**: only the owner can pull/push.
- Grants exist at namespace level and repository level, for a user or for
  `anonymous`; **push implies pull**.
- Service accounts authenticate with Basic auth (`username` + token); the token
  is shown once and stored only as a hash plus 3-char prefix/suffix.
- Authorization order for `/v2` requests: owner/member → explicit grant →
  public visibility (pull only) → deny. Pull/push/delete actions map from the
  HTTP method; `?from=` on a mount additionally requires pull on the source.
- Anonymous → 401 with `WWW-Authenticate: Basic realm="Lighthouse"`; authenticated
  but unauthorized → 401 with `error="insufficient_scope"` for scope failures,
  403 `DENIED` only for policy denials.

---

## 7. Configuration

All settings come from environment variables (see `docs/CONFIGURATION.md`).
Important ones: `LIGHTHOUSE_HOST`, `LIGHTHOUSE_PORT`, `PUBLIC_HOST`, `BASE_URL`,
`DATABASE_DIR`, `DATABASE_URL`, `DATA_DIR`, `LOGS_DIR`, `JWT_SECRET`,
`ACCESS_TOKEN_TTL_SECS`, `REFRESH_TOKEN_TTL_SECS`, `COOKIE_NAME`,
`COOKIE_DOMAIN`, `COOKIE_SECURE`, `TRUST_PROXY`, `REGISTRATION_ENABLED`,
`EMAIL_ENABLED`, `BREVO_API_KEY`, `BREVO_SENDER_EMAIL`, `BREVO_SENDER_NAME`,
`CAPTCHA_ENABLED`, `CAPTCHA_DIFFICULTY`, `RATE_LIMIT_LOGIN_PER_MINUTE`,
`RATE_LIMIT_REGISTER_PER_HOUR`, `LIBRAVATAR_BASE_URL`, `LIGHTHOUSE_TITLE`.

---

## 8. SQLite concurrency strategy

- `PRAGMA journal_mode=WAL`, `busy_timeout=5000`, `foreign_keys=ON`,
  `synchronous=NORMAL`; pool of 8 connections.
- All multi-statement mutations run in a transaction; long GC work is chunked.
- `db::with_busy_retry` wraps operations that can still hit `SQLITE_BUSY`.
- Uploads write bytes to disk first; the DB row is updated after the write is
  durable, so a crashed upload never leaves a half-registered blob.

---

## 9. Garbage collection

Reference graph, evaluated in one pass:

```
roots      = every manifest referenced by a tag, plus every revision link
mark(d)    = d ∈ roots ∪ ⋃ mark(ref) for ref ∈ references(d)
references = manifest → {config blob, layer blobs} ; index → {child manifests}
sweep      = delete blobs with no mark, delete orphan manifests, delete empty tags
```

Reachability is stored in `manifest_blobs` (config/layer edges) and
`manifest_children` (index child edges); `blob_repositories` and
`manifest_repositories` record per-repository linkage. GC runs on demand from the
control plane and never touches in-progress upload rows.

---

## 10. Extension points (declared, not implemented)

Pull-through proxy, webhook notifications, multi-region replication and
an S3-backed driver are explicitly out of scope; the storage trait boundary in
`storage/mod.rs` is where a second driver would plug in.
