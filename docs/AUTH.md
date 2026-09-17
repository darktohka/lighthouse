# Lighthouse — Identity & Authentication

Owner: wave 2B (`src/auth/*`, `src/permissions.rs`, `src/email.rs`,
`src/captcha.rs`). This document describes the identity model, the credential
formats and the HTTP surface under `/api/auth`.

---

## 1. Model

| Concept | Storage | Notes |
|---|---|---|
| Human account | `users` | Argon2id `password_hash`, `email_verified`, `is_admin` |
| Personal namespace | `namespaces` (`kind='user'`) | created with the account; name == username; private by default |
| Web session | `sessions` | one row per refresh-token family; `id` is also the access-token `jti` |
| Access token | JWT cookie | HS256, stateless, short-lived |
| Refresh token | `sessions.refresh_token_hash` | opaque 32-byte URL-safe value, SHA-256 at rest |
| E-mail token | `email_tokens` | opaque 32-byte hex, single-use, kind `verify_email` / `reset_password` |
| Service account | `service_accounts` | Basic credentials for registry clients |
| Second factor | `users.totp_secret`, `users.totp_enabled` | RFC 6238 TOTP; `totp_confirmed_at` / `totp_last_used_step` |
| Backup code | `totp_backup_codes` | single-use Argon2id hashes, 8 per enrolment |
| App password | `app_passwords` | per-account registry credential, SHA-256 at rest |
| Registry token | JWT | stateless HS256 bearer token for `/v2` |
| MFA token | JWT | short-lived `kind:"mfa"` token for the login second step |
| Login history | `login_events` | password, service-token, app-password, two-factor and refresh use |

### Credential formats

- **Access token** — JWT signed HS256 with `JWT_SECRET`; claims
  `{ sub, username, kind: "access", iat, exp, jti }`. `sub` is the user id and
  `jti` is the web-session id. TTL `ACCESS_TOKEN_TTL_SECS`.
- **Refresh token** — 32 random bytes, URL-safe base64 (unpadded). Only
  `sha256(token)` is persisted; rotation replaces the stored hash.
- **E-mail token** — 32 random bytes as lowercase hex.
- **Service token** — `lhr_<48 hex chars>`. The plaintext is shown once; only an
  Argon2id hash plus the first 3 and last 3 characters are stored.
- **App password**: `lhp_<48 hex chars>`. The plaintext is shown once; only a
  SHA-256 hash plus the first 3 and last 3 characters are stored, indexed by
  `(user_id, token_hash)`. The 192-bit random secret is looked up as a fast
  indexed hash and is never passed through Argon2.

### Password hashing

Argon2id (`m=19456 KiB`, `t=2`, `p=1`) with a fresh 16-byte random salt, stored
as a PHC string. Empty passwords are rejected; verification of a malformed hash
returns `false` rather than erroring.

### Single-use tokens and idempotency

Verification and reset tokens are single-use: the row is claimed with a
conditional `UPDATE … WHERE used_at IS NULL`, so two concurrent requests cannot
both consume it.

Because verification links are routinely fetched twice — React StrictMode
double-invokes effects in development, mail clients prefetch URLs, and users
reload the page — `/verify-email` is deliberately **idempotent**: a token that
has already been consumed still returns `200` while its account is verified, and
only a token that is unknown, expired, or consumed for a *not* yet verified
account returns `400`. Password-reset tokens are intentionally **not**
idempotent: replaying a used reset token is always rejected.

---

## 2. Cookie & session strategy

The access token is carried in an `HttpOnly` cookie; the refresh token is
returned in the JSON body and never set as a cookie.

| Attribute | Value |
|---|---|
| Name | `COOKIE_NAME` (default `lighthouse_token`) |
| `Path` | `/` |
| `HttpOnly` | always |
| `SameSite` | `Lax` |
| `Secure` | `COOKIE_SECURE` |
| `Domain` | `COOKIE_DOMAIN` when set |
| `Max-Age` | `ACCESS_TOKEN_TTL_SECS` |

Logout emits the same cookie with `Max-Age=0`.

Session lifecycle:

1. **Login** creates a `sessions` row (random 32-byte hex id, round-to-millisecond
   timestamps) and returns the refresh token; the access token is installed as a
   cookie with `jti = session.id`.
2. **Refresh** uses the body refresh token to find the session, verifies the
   SHA-256 hash, rotates the refresh secret, updates `last_seen_at`, and issues a
   new access cookie.
3. **Logout** revokes the session identified by the access token's `jti` (or by a
   refresh token in an optional body) and clears the cookie.
4. `DELETE /sessions/{id}` revokes one session; password reset/change revoke all
   sessions for the account.
5. Expired session rows are pruned opportunistically on session creation.

Access tokens are stateless: revoking a session stops refresh, but an already
issued access token remains valid until `exp`. `GET /me` without any credential
returns `401`.

---

## 3. Request identity

`auth::middleware::resolve_identity` runs for every request (mounted globally in
`routes::build`, outermost so the access log can see the actor) and is
best-effort — it never rejects. Credentials are tried in order:

1. Access-token cookie → JWT verify → `{ user_id, username, is_admin }`.
2. `Authorization: Basic` → the username is resolved in order: a matching
   `service_accounts.username` verified against its Argon2id token hash, then
   the human account's app password, then the account password.
3. `Authorization: Bearer` → JWT verify (API clients that cannot use cookies).

App passwords bypass TOTP; when TOTP is enabled a correct account password alone
is rejected, so registry clients must use an app password (see §8). Registry
tokens (`kind:"registry"`) are accepted only under `/v2`, and the control-plane
extractors reject them, so a token cached by `docker` cannot act as a web
session (see §9).

The result is inserted as an `AuthContext` request extension. Handlers consume it
through extractors:

| Extractor | Behaviour |
|---|---|
| `Auth(ctx)` | always succeeds; anonymous when no credentials |
| `Authenticated(ctx)` | `401 unauthorized` when anonymous |
| `Admin(ctx)` | `403 forbidden` unless `is_admin` |

`challenge_headers()` advertises `WWW-Authenticate: Basic realm="Lighthouse Registry"`.
Every Basic service-token or password attempt writes a `login_events` row.

---

## 4. Endpoint reference (`/api/auth`, `/api/app-passwords`)

All responses are JSON. Errors use the control-plane envelope
`{"error":{"code":"…","message":"…"}}`.

| Method | Path | Auth | Success | Notes |
|---|---|---|---|---|
| POST | `/register` | no | `201 {user}` | validates captcha + input; creates user + personal namespace (unverified); sends verification mail |
| POST | `/login` | no | `200 {user, refresh_token}` + cookie, or `200 {two_factor_required:true, mfa_token}` | `identifier` may be username or e-mail; with TOTP enabled and no `code`, returns the 2FA challenge and sets no cookie |
| POST | `/login/2fa` | no | `200 {user, refresh_token}` + cookie | consumes `mfa_token` plus a TOTP or backup code; charged to the `login` bucket |
| POST | `/logout` | cookie/refresh | `204` | revokes session, clears cookie |
| POST | `/refresh` | refresh token | `200 {refresh_token}` + cookie | rotates the refresh secret |
| GET | `/me` | required | `200 {user, namespaces}` | namespaces owned or joined |
| GET | `/captcha` | no | `200 {challenge,token,expires}` | `204` when `CAPTCHA_ENABLED=false` |
| POST | `/verify-email` | no | `200 {verified:true}` | consumes the token, sets `email_verified=1`; **idempotent** — re-verifying an already-verified account returns `200`, a bogus/expired token returns `400` |
| POST | `/resend-verification` | no | `202` | always accepted; never leaks account existence |
| POST | `/forgot-password` | no | `202` | sends reset mail only when the account exists |
| POST | `/reset-password` | no | `200 {reset:true}` | consumes token, rehashes, revokes all sessions |
| POST | `/change-password` | required | `200 {changed:true}` | verifies the current password |
| GET | `/sessions` | required | `200 {sessions:[…]}` | no token hashes are exposed |
| DELETE | `/sessions/{id}` | required | `204` | only the caller's own sessions |
| GET | `/login-history` | required | `200 {events,limit,offset}` | newest first; `?limit=` (≤200) `&offset=` |
| GET, POST | `/token` | no | `200 {token, access_token, expires_in, issued_at}` | registry bearer-token service; see §9 |
| GET | `/2fa` | required | `200 {enabled, backup_codes_remaining}` | status for the caller's account |
| POST | `/2fa/setup` | required | `200 {secret, otpauth_uri, backup_codes:[…8]}` | writes an unconfirmed secret (`totp_enabled=0`), replacing any pending enrolment; `secret` is returned only here |
| POST | `/2fa/enable` | required | `200 {enabled:true}` | verifies a live code, sets `totp_enabled=1`, revokes every other session |
| POST | `/2fa/disable` | required | `200 {enabled:false}` | requires the account password plus a live TOTP or backup code; clears the secret, deletes the backup codes, revokes other sessions |
| POST | `/2fa/backup-codes` | required | `200 {backup_codes:[…8]}` | requires a live TOTP code, never a backup code; invalidates the old set |

App-password endpoints (`/api/app-passwords`, session required, own credentials
only):

| Method | Path | Auth | Success | Notes |
|---|---|---|---|---|
| GET | `/api/app-passwords` | required | `200 [{id, name, token_prefix, token_suffix, created_at, last_used_at}]` | no token hashes are exposed |
| POST | `/api/app-passwords` | required | `201 {app_password, token}` | names are unique per user; `token` is shown once |
| POST | `/api/app-passwords/{id}/token` | required | `200 {app_password, token}` | rotates; the new secret is shown once |
| DELETE | `/api/app-passwords/{id}` | required | `204` | deletes the credential |

When captcha is enabled, `pow-captcha-axum`'s routes are additionally mounted
under `/api/auth/captcha` (`POST /challenge`, `/redeem`, `/consume`). `GET
/captcha` issues a challenge directly so non-browser clients can start the flow.

### Error codes

| Code | HTTP | Trigger |
|---|---|---|
| `bad_request` | 400 | validation failure, invalid captcha or token |
| `invalid_credentials` | 401 | unknown identifier or wrong password |
| `two_factor_required` | 401 | login needs the 2FA step; no session is issued |
| `invalid_two_factor_code` | 401 | wrong, expired or already-used TOTP / backup code |
| `unauthorized` | 401 | missing/expired session |
| `email_not_verified` | 403 | login before e-mail verification |
| `registration_disabled` | 403 | `REGISTRATION_ENABLED=false` |
| `forbidden` | 403 | admin required |
| `username_taken` / `namespace_taken` / `email_taken` / `conflict` | 409 | duplicate identity |
| `rate_limited` | 429 | login/register/forgot-password bucket exhausted |

### Rate limiting

| Bucket | Window | Key | Env |
|---|---|---|---|
| `login` | per minute | `ip:identifier` | `RATE_LIMIT_LOGIN_PER_MINUTE` |
| `register` | per hour | client IP | `RATE_LIMIT_REGISTER_PER_HOUR` |
| `forgot-password` | per hour | client IP | reuses the register budget |

`/api/auth/login/2fa` is charged to the `login` bucket.

The client IP honours `X-Forwarded-For`, `X-Real-IP`, `CF-Connecting-IP` and
`X-Client-IP` when `TRUST_PROXY=true`.

---

## 5. Captcha

`src/captcha.rs` wraps `pow-captcha-axum`:

- `install(db, config)` is called once from `routes::build`; it creates the
  shared `CaptchaState` when the feature is enabled.
- `issue(app, headers)` writes a challenge row and returns the puzzle descriptor.
- `verify(app, token, headers)` consumes a redeemed captcha token.
- Both are pass-throughs when `CAPTCHA_ENABLED=false`, so registration and login
  work unchanged in development.

Challenge parameters: 50 puzzles, 32-hex salts, difficulty `CAPTCHA_DIFFICULTY`,
600 s TTL; redeemed tokens live for 1200 s.

---

## 6. E-mail

`src/email.rs` renders verification and reset messages using a light/dark-friendly
Primer-styled template (`#0969da` accent, `#1f2328` text, `#d0d7de` border) and
delivers them through Brevo (`POST https://api.brevo.com/v3/smtp/email`, header
`api-key`). When `EMAIL_ENABLED=false` or `BREVO_API_KEY` is unset the rendered
message is logged at INFO so local development still works.

---

## 7. Two-factor authentication

TOTP is opt-in per account and state lives on `users`:

| Column | Meaning |
|---|---|
| `totp_secret` | base32 secret, `NULL` until enrolment |
| `totp_enabled` | `0`/`1`; set to `1` only after a live code is confirmed |
| `totp_confirmed_at` | when the factor was enabled |
| `totp_last_used_step` | last accepted time step, for replay rejection |

Codes follow RFC 6238: TOTP, HMAC-SHA1, 6 digits, 30-second period. A code is
accepted within ±1 step of the server clock, and a code whose time step has
already been consumed is rejected (RFC 6238 §5.2).

Backup codes live in `totp_backup_codes` (`id`, `user_id`, `code_hash`,
`used_at`, `created_at`). Eight are generated per enrolment in the format
`XXXX-XXXX` (`A`-`Z`, `0`-`9`), stored as Argon2id hashes, and each is
single-use. The plaintext is returned once, from the call that created it.

Enrolment and management, all requiring a web session:

1. `POST /api/auth/2fa/setup` returns `{ secret, otpauth_uri, backup_codes }`
   and writes the secret with `totp_enabled=0`. It replaces any previous
   unconfirmed enrolment and its backup codes. The secret is returned only
   here.
2. `POST /api/auth/2fa/enable { code }` verifies a live code, sets
   `totp_enabled=1` and revokes every other session.
3. `POST /api/auth/2fa/disable { password, code }` requires the account
   password plus a live TOTP or backup code, then clears the secret, deletes
   the backup codes and revokes other sessions.
4. `POST /api/auth/2fa/backup-codes { code }` issues a fresh set and invalidates
   the old one. It requires a live TOTP code, never a backup code.

`GET /api/auth/2fa` reports `{ enabled, backup_codes_remaining }`.

Login is a two-step flow while the factor is enabled:

1. `POST /api/auth/login` accepts an optional `code`. Without a `code` it
   returns `200 { two_factor_required: true, mfa_token }` and sets no cookie.
2. `POST /api/auth/login/2fa { mfa_token, code }` returns the normal
   `200 { user, refresh_token }` plus the access cookie.

`mfa_token` is a short-lived JWT (`kind:"mfa"`, TTL `MFA_TOKEN_TTL_SECS`,
default 300) and is never an identity on its own. `/api/auth/login/2fa` uses the
`login` rate-limit bucket.

A correct account password alone cannot satisfy a TOTP-enabled account, so
`docker login` requires an app password (see §8).

---

## 8. App passwords

An app password is a registry credential for a human account. It lets a client
authenticate without interactive TOTP and is the required path for
`docker login` once 2FA is on.

Credentials are stored in `app_passwords` (`id`, `user_id`, `name`,
`token_prefix`, `token_suffix`, `token_hash`, `created_at`, `last_used_at`).
The format is `lhp_<48 hex chars>`. Only `sha256(token)` is persisted, indexed
by `(user_id, token_hash)`; the secret is 192-bit random, so lookup is a fast
indexed hash and never Argon2. Names are unique per user.

The endpoints are listed in §4. They all require a web session and operate only
on the caller's own credentials:

- `GET /api/app-passwords` returns
  `[{ id, name, token_prefix, token_suffix, created_at, last_used_at }]`.
- `POST /api/app-passwords { name }` returns `201 { app_password, token }`;
  `token` is shown once.
- `POST /api/app-passwords/{id}/token` rotates and returns the new secret once.
- `DELETE /api/app-passwords/{id}` returns `204`.

In the Basic-auth order (§3) an app password is tried after service accounts and
before the account password. It authenticates and bypasses TOTP, while a correct
account password is rejected once TOTP is enabled. App-password logins are
recorded in `login_events.kind='app_password'`.

---

## 9. Registry token authentication

Registry clients discover the token service from the `401` challenge returned
under `/v2`:

```
WWW-Authenticate: Bearer realm="<base_url>/api/auth/token",service="<public_host>"
```

Where a repository is known the challenge also carries
`scope="repository:<name>:pull"` (`pull,push` for push methods). Basic auth is
still accepted on the wire for compatibility.

`GET|POST /api/auth/token` is public. It takes `service`, a repeatable and
space-joined `scope`, `client_id`, `account` and `offline_token`, passed as a
query string for `GET` and form-encoded for `POST`. Credentials are optional:

- No credentials yields an anonymous token.
- Valid Basic (a service token, an app password, or an account password with
  TOTP disabled) yields a token bound to that identity.
- Present but invalid Basic yields `401` with a Basic challenge.

A cookie is deliberately ignored. The response is
`{ token, access_token, expires_in, issued_at }` and never includes
`identity_token`.

The token is an HS256 JWT signed with a key derived from `JWT_SECRET`
(domain-separated), carrying `kind:"registry"`, `sub` set to the user id or `0`
for anonymous, `sid` set to the service-account id when applicable,
`aud:"lighthouse-registry"`, `iss` set to `base_url`, and
`access:[{type,name,actions}]`. TTL `REGISTRY_TOKEN_TTL_SECS` (default 300).

Scope is resolved against the same permission model as a live request. A
requested scope that is not granted yields a token with an empty `access` list,
and the registry then answers `403 DENIED`. Once a registry token has been
presented, `/v2` answers `403` rather than another `401` challenge, so the
client cannot loop refetching tokens. Registry tokens are accepted only by
`/v2`; the control-plane extractors reject them, so a token cached by `docker`
cannot act as a web session.

Anonymous tokens can pull public repositories and tags; private repositories
return `403 DENIED`.
