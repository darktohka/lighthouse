# Lighthouse — Control-Plane API

Base path `/api`. Every response is JSON. Authentication is the access-token
cookie (or `Authorization: Bearer`); see `docs/AUTH.md` for the identity surface
under `/api/auth`.

Errors use the envelope:

```json
{ "error": { "code": "not_found", "message": "repository not found" } }
```

| Code | HTTP | Meaning |
|---|---|---|
| `bad_request` | 400 | validation failure |
| `unauthorized` | 401 | no/expired credential |
| `forbidden` | 403 | authenticated but not permitted |
| `not_found` | 404 | resource does not exist or is not visible |
| `conflict` | 409 | duplicate name / already exists |
| `rate_limited` | 429 | limiter exhausted |
| `internal_error` | 500 | unexpected failure |

## Conventions

- Timestamps are RFC 3339 UTC strings (`2026-09-17T10:30:00.000Z`).
- Sizes are integers in **bytes**.
- List endpoints accept `?page=` (1-based) and `?per_page=` (default 25, max 100)
  and return:

```json
{ "items": [], "total": 0, "page": 1, "per_page": 25 }
```

- Repository paths are variable-length (`darktohka/more/complicated/project2`), so
  repository endpoints place the full name in a capture-all segment and the
  namespace is always the first path segment.
- Visibility: a private namespace is invisible unless the caller is the owner, a
  member, or holds a grant. A public namespace exposes only public repositories
  plus everything the caller may pull.

---

## 1. Users & social

| Method | Path | Auth | Description |
|---|---|---|---|
| GET | `/users/{username}` | optional | profile |
| PATCH | `/users/me` | required | update own profile |
| GET | `/users/{username}/heatmap?year=YYYY` | optional | daily contribution counts |
| GET | `/users/{username}/followers` | optional | paginated `UserSummary` |
| GET | `/users/{username}/following` | optional | paginated `UserSummary` |
| POST | `/users/{username}/follow` | required | follow (204) |
| DELETE | `/users/{username}/follow` | required | unfollow (204) |
| GET | `/users/search?q=&limit=` | optional | username autocomplete (delegations) |

```
UserSummary   { id, username, first_name, last_name, avatar_url }
UserProfile   { id, username, first_name, last_name, bio, company, location,
                website, avatar_url, created_at, namespace,
                repository_count, public_repository_count, total_pulls,
                follower_count, following_count, is_following, is_self }
HeatmapDay    { date, count }
Heatmap       { year, days: [HeatmapDay], total }
UpdateProfile { first_name?, last_name?, bio?, company?, location?, website?, theme? }
```

`avatar_url` is resolved by the frontend from `LIBRAVATAR_BASE_URL` + the
SHA-256 of the e-mail unless the user supplied an explicit override.

---

## 2. Namespaces

| Method | Path | Auth | Description |
|---|---|---|---|
| GET | `/namespaces` | optional | visible namespaces |
| POST | `/namespaces` | required | create workspace |
| GET | `/namespaces/{name}` | optional | detail |
| PATCH | `/namespaces/{name}` | required (owner) | description / visibility |
| DELETE | `/namespaces/{name}` | required (owner) | delete workspace (not personal namespaces) |
| GET | `/namespaces/{name}/members` | optional | members |
| POST | `/namespaces/{name}/members` | required (owner) | add member `{username, role}` |
| DELETE | `/namespaces/{name}/members/{username}` | required (owner) | remove member |

```
Namespace       { id, name, kind: "user"|"workspace", owner: UserSummary|null,
                  description, is_public, repository_count, created_at }
NamespaceMember { user: UserSummary, role: "admin"|"member", created_at }
```

Creating a workspace whose name collides with an existing username or workspace
returns `409 conflict` (`namespace_taken`). Reserved first segments
(`v2`, `api`, `admin`, `static`, `assets`, `login`, `register`, `settings`,
`explore`, `analytics`, `search`, `new`, `notifications`, `account`,
`organizations`, `libraries`) are rejected with `400 bad_request`.

---

## 3. Repositories (images) & tags

| Method | Path | Auth | Description |
|---|---|---|---|
| GET | `/namespaces/{name}/repositories` | optional | list images in a namespace |
| GET | `/repositories/{namespace}/{*repo}` | optional | image detail |
| PATCH | `/repositories/{namespace}/{*repo}` | required | description / visibility |
| DELETE | `/repositories/{namespace}/{*repo}` | required | delete the image and all its tags |
| GET | `/repositories/{namespace}/{*repo}/tags` | optional | paginated tags |
| GET | `/repositories/{namespace}/{*repo}/tags/{tag}` | optional | tag detail |
| DELETE | `/repositories/{namespace}/{*repo}/tags/{tag}` | required | delete one tag |
| POST | `/repositories/{namespace}/{*repo}/tags/batch-delete` | required | `{tags:[…]}` → `{deleted:n}` |
| GET | `/repositories/{namespace}/{*repo}/pulls?days=30` | optional | pull statistics |
| GET | `/tags?sort=total_size\|unique_size&order=desc\|asc&namespace=` | optional | all visible tags ranked by size |
| POST | `/tags/batch-delete` | required | `{items:[{repository,tag}]}` → `{deleted:n}` |

```
RepositorySummary { id, namespace, path, name, description, is_public,
                    tag_count, size, pull_count, updated_at }
RepositoryDetail  { ...RepositorySummary, manifest_count, platform_count,
                    total_size, unique_size, shared_size, created_at,
                    created_by: UserSummary|null,
                    permissions: PublicPermission[], can_pull, can_push }
TagSummary  { name, digest, media_type, size, compressed_size,
              platforms: Platform[], pull_count, updated_at }
TagDetail   { ...TagSummary, manifest: object,
              config: object|null, layers: LayerInfo[], can_pull, can_push }
Platform    { os, architecture, variant|null, digest, size }
LayerInfo   { digest, media_type, size, role: "config"|"layer" }
TagSizeEntry{ repository, namespace, tag, total_size, unique_size, shared_size,
              platforms: Platform[], updated_at }
```

- `can_pull` / `can_push` report the calling actor's effective access to the
  repository (push implies pull); the UI uses them to offer reads to pullers and
  mutations only to pushers.
- `size` is the sum of the compressed blob sizes referenced by the tag.
- `unique_size` is the storage owned by the entity — every blob for which its
  earliest referencing tag (by `tags.created_at`) lives here; `shared_size` is
  `size - unique_size`, bytes an earlier tag already owned. Reuse between tags
  of the same repository does not count as shared.
- Deleting an image or tag runs reference-counted cleanup: blobs with no remaining
  references and no remaining repository links are deleted from disk immediately.
- `GET /tags` powers the "all my tags by size" page. `sort=unique_size` ranks by
  the storage each tag owns.

---

## 4. Manifest & layer browsing

| Method | Path | Auth | Description |
|---|---|---|---|
| GET | `/repositories/{namespace}/{*repo}/manifests/{digest}` | optional | raw manifest/index JSON |
| GET | `/repositories/{namespace}/{*repo}/manifests/{digest}/references` | optional | referenced descriptors |
| GET | `/blobs/{digest}` | optional | raw blob bytes (`application/octet-stream`) |
| GET | `/blobs/{digest}/json` | optional | blob parsed as JSON (config blobs) |
| GET | `/repositories/{namespace}/{*repo}/layers/{digest}/tree?path=` | optional | directory listing inside the layer tar |
| GET | `/repositories/{namespace}/{*repo}/layers/{digest}/file?path=` | optional | one file's bytes (`Content-Type` guessed) |
| GET | `/repositories/{namespace}/{*repo}/layers/{digest}/download` | optional | raw layer archive |

```
LayerTreeEntry { name, path, kind: "file"|"dir"|"symlink", size,
                 mode, link_target, link_resolved, link_kind }
LayerReference { digest, media_type, size, role }
```

`size` is the uncompressed byte count for a file or symlink; for a directory it
is the recursive total of every file below it, computed once when the layer index
is built and served from the cache. For a symlink, `link_resolved` is the
normalized layer path it points at (following chains) and `link_kind` is the
resolved entry's kind (`file`/`dir`); both are `null` for a dangling or cyclic
link and for every non-symlink entry.

Layer archives are decompressed transparently for browsing — `tar`,
`tar+gzip` and `tar+zstd` are supported, selected by the layer media type. Paths
are sanitized (no `..`, no absolute paths, no symlink traversal) before use. Large
layers are streamed and entries are capped per request.

---

## 5. Permissions & delegations

| Method | Path | Auth | Description |
|---|---|---|---|
| GET | `/namespaces/{name}/permissions` | required (owner) | grants on a namespace |
| POST | `/namespaces/{name}/permissions` | required (owner) | add grant |
| DELETE | `/namespaces/{name}/permissions/{id}` | required (owner) | revoke grant |
| GET | `/repositories/{namespace}/{*repo}/permissions` | required (owner) | grants on an image |
| POST | `/repositories/{namespace}/{*repo}/permissions` | required (owner) | add grant |
| DELETE | `/repositories/{namespace}/{*repo}/permissions/{id}` | required (owner) | revoke grant |

```
CreateGrant { subject_type: "user"|"anonymous", subject?: string, can_push: bool }
PublicPermission { id, subject_type, subject: UserSummary|null,
                   can_pull, can_push, created_at }
```

`can_push` implies `can_pull` server-side. `subject` is a username and is
required only when `subject_type == "user"`.

---

## 6. Service accounts

| Method | Path | Auth | Description |
|---|---|---|---|
| GET | `/service-accounts` | required | caller's service accounts |
| POST | `/service-accounts` | required | create; token returned **once** |
| GET | `/service-accounts/{id}` | required | detail with grants |
| DELETE | `/service-accounts/{id}` | required | delete |
| POST | `/service-accounts/{id}/token` | required | rotate; new token returned once |
| POST | `/service-accounts/{id}/grants` | required | add a grant |
| DELETE | `/service-accounts/{id}/grants/{grant_id}` | required | remove a grant |
| POST | `/service-accounts/{id}/ip-ranges` | required | add an IP allowlist entry `{cidr}` |
| DELETE | `/service-accounts/{id}/ip-ranges/{range_id}` | required | remove an IP allowlist entry |

```
ServiceAccount { id, name, username, description, token_prefix, token_suffix,
                 created_at, last_used_at, grants: ServiceAccountGrant[],
                 ip_ranges: ServiceAccountIpRange[] }
CreatedServiceAccount { account: ServiceAccount, token: string }
ServiceAccountGrant { id, namespace: string|null, repository: string|null,
                      can_pull, can_push }
ServiceAccountIpRange { id, cidr }
CreateServiceAccount { name, description?, ip_ranges?: string[] }
CreateServiceAccountGrant { namespace?: string, repository?: string, can_push: bool }
CreateServiceAccountIpRange { cidr: string }
```

The plaintext token (`lhr_…`) is present only in the create/rotate response.
Thereafter only `token_prefix` + `token_suffix` are exposed for identification.

`ip_ranges` holds normalized CIDRs: a bare address is stored as a host route
(`/32` for IPv4, `/128` for IPv6) and a network is truncated to its base
(`10.1.2.3/24` becomes `10.1.2.0/24`). An empty list means unrestricted. At most
64 ranges are allowed per account. `POST .../ip-ranges` returns
`201 { id, cidr }` and rejects an invalid CIDR or an IPv4-mapped IPv6 range with
`400 bad_request`, a duplicate (after normalization) with `409 conflict`, an
account not owned by the caller with `404 not_found`, and a list that exceeds the
cap with `400 bad_request`. `DELETE .../ip-ranges/{range_id}` returns `204`, or
`404 not_found` for an unknown range. See `docs/AUTH.md` for how the allowlist is
enforced during authentication.

---

## 7. Analytics

`GET /api/analytics/overview?namespace=` (auth optional; scoped to what the
caller may read)

```json
{
  "total_size": 0,
  "unique_size": 0,
  "shared_size": 0,
  "shared_percentage": 0.0,
  "blob_count": 0,
  "manifest_count": 0,
  "repository_count": 0,
  "tag_count": 0,
  "pull_count": 0,
  "pull_count_30d": 0,
  "largest_tags": [],
  "disk_usage_by_repository": [],
  "pulls_over_time": [],
  "top_repositories": []
}
```

```
DiskUsageEntry   { repository, size, unique_size }
PullsOverTime    { date, pulls }
TopRepository    { repository, pulls }
```

`shared_percentage = shared_size / total_size * 100` (0 when total is 0).
`disk_usage_by_repository` and `largest_tags` are ordered descending.

---

## 8. Activity & dashboard

| Method | Path | Auth | Description |
|---|---|---|---|
| GET | `/activity` | optional | public activity feed |
| GET | `/activity/me` | required | the caller's activity |
| GET | `/dashboard` | required | own repositories + timeline + counters |

```
ActivityEntry { id, kind, summary, actor: UserSummary|null, namespace,
                repository, metadata, created_at }
Dashboard     { repositories: RepositorySummary[],
                activity: ActivityEntry[],
                stats: { repository_count, tag_count, total_size,
                         pull_count_30d } }
```

Activity kinds include `user.registered`, `namespace.created`,
`repository.created`, `tag.pushed`, `tag.deleted`, `manifest.deleted`,
`permission.granted`, `permission.revoked`, `service_account.created`.

---

## 9. App passwords

Per-account registry credentials for human users. The plaintext token
(`lhp_…`) is returned once; only a SHA-256 hash plus the first and last three
characters are stored. See `docs/AUTH.md` for the full model.

| Method | Path | Auth | Description |
|---|---|---|---|
| GET | `/app-passwords` | required | caller's app passwords |
| POST | `/app-passwords` | required | create `{name}`; token returned **once** |
| POST | `/app-passwords/{id}/token` | required | rotate; new token returned once |
| DELETE | `/app-passwords/{id}` | required | delete |

```
AppPassword        { id, name, token_prefix, token_suffix, created_at, last_used_at }
CreatedAppPassword { app_password: AppPassword, token: string }
```

Names are unique per user. An app password authenticates over Basic auth and
bypasses TOTP, so it is the credential to use for `docker login` once 2FA is
enabled.

---

## 10. Two-factor authentication

All endpoints require a web session and live under `/api/auth`. Full behaviour
is in `docs/AUTH.md`.

| Method | Path | Auth | Description |
|---|---|---|---|
| GET | `/auth/2fa` | required | `{enabled, backup_codes_remaining}` |
| POST | `/auth/2fa/setup` | required | returns `{secret, otpauth_uri, backup_codes:[…8]}` |
| POST | `/auth/2fa/enable` | required | confirm `{code}`; revokes other sessions |
| POST | `/auth/2fa/disable` | required | `{password, code}`; clears the factor |
| POST | `/auth/2fa/backup-codes` | required | reissue `{code}` → `{backup_codes:[…8]}` |
| POST | `/auth/login/2fa` | none | `{mfa_token, code}` → `{user, refresh_token}` + cookie |

```
TwoFactorStatus { enabled, backup_codes_remaining }
TwoFactorSetup  { secret, otpauth_uri, backup_codes: string[] }
AuthResponse    { user, refresh_token }
```

When 2FA is enabled, `POST /api/auth/login` without a `code` returns
`{ two_factor_required: true, mfa_token }` and no cookie; the client then calls
`/auth/login/2fa`.

---

## 11. Health

| Method | Path | Auth | Description |
|---|---|---|---|
| GET | `/healthz` | none | liveness |
| GET | `/api/health` | none | liveness (JSON) |

---

## 12. Registry token endpoint

| Method | Path | Auth | Description |
|---|---|---|---|
| GET, POST | `/auth/token` | optional (Basic) | registry bearer-token service |

Public. `GET` reads the query string; `POST` reads a form-encoded body and any
query string. The access-token cookie is ignored. Credentials come from an
`Authorization: Basic` header or, for `grant_type=password`, from the body.
`grant_type=refresh_token` must be sent in a POST body; a GET carrying it is
rejected with `invalid_request`.

Parameters:

| Name | Meaning |
|---|---|
| `service` | service name from the registry challenge; accepted, not validated |
| `scope` | repeatable and space-joined; `repository:<name>:pull[,push]` or `registry:catalog:*` |
| `client_id` | client identifier recorded with an issued refresh token |
| `account` | account name supplied by the client; accepted |
| `offline_token` | request a refresh token; empty, `1`, `true` or `yes` |
| `access_type` | `offline` requests a refresh token when `offline_token` is absent |
| `grant_type` | `password` or `refresh_token` |
| `refresh_token` | required with `grant_type=refresh_token`, which must be a POST |
| `username`, `password` | required with `grant_type=password` when no Basic header is sent |

Response:

```json
{
  "token": "<jwt>",
  "access_token": "<jwt>",
  "expires_in": 300,
  "issued_at": "<rfc3339>"
}
```

`token` and `access_token` are the same HS256 registry bearer JWT. `expires_in`
is `REGISTRY_TOKEN_TTL_SECS` (default 300). `issued_at` is an RFC 3339
timestamp. A `refresh_token` field is added only on an authenticated offline
issuance; it is the stable value `docker login` stores as `identitytoken`. A
`grant_type=refresh_token` response never contains `refresh_token`, because the
secret does not rotate. No credentials yields an anonymous token. See
`docs/AUTH.md` §9 and §10 for the token claims and the offline-token model.

Request-validation failures return HTTP `400` with
`{"error": "...", "error_description": "..."}`:

| `error` | Trigger |
|---|---|
| `invalid_request` | `grant_type=refresh_token` without `refresh_token`, or sent as a GET; `grant_type=password` without `username`/`password` |
| `invalid_grant` | wrong `username`/`password` under `grant_type=password` |
| `unsupported_grant_type` | `grant_type` other than `password` or `refresh_token` |

A rejected refresh redemption (unknown, expired or revoked refresh token) is
answered `401` with the OAuth2 body `{"error":"invalid_grant",...}` and a
`WWW-Authenticate: Basic realm="Lighthouse Registry"` header, so the client
prompts for credentials again.

A request carrying Basic credentials that do not authenticate is also answered
`401` with a `WWW-Authenticate: Basic realm="Lighthouse Registry"` header, but
with the control-plane envelope
`{"error":{"code":"unauthorized","message":"authentication required"}}`.
