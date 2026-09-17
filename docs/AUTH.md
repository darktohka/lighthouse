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
| Login history | `login_events` | password, service-token and refresh use |

### Credential formats

- **Access token** — JWT signed HS256 with `JWT_SECRET`; claims
  `{ sub, username, kind: "access", iat, exp, jti }`. `sub` is the user id and
  `jti` is the web-session id. TTL `ACCESS_TOKEN_TTL_SECS`.
- **Refresh token** — 32 random bytes, URL-safe base64 (unpadded). Only
  `sha256(token)` is persisted; rotation replaces the stored hash.
- **E-mail token** — 32 random bytes as lowercase hex.
- **Service token** — `lhr_<48 hex chars>`. The plaintext is shown once; only an
  Argon2id hash plus the first 3 and last 3 characters are stored.

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
2. `Authorization: Basic` → a matching `service_accounts.username` is verified
   against its Argon2id token hash; otherwise the username is looked up as a
   human account and the password verified.
3. `Authorization: Bearer` → JWT verify (API clients that cannot use cookies).

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

## 4. Endpoint reference (`/api/auth`)

All responses are JSON. Errors use the control-plane envelope
`{"error":{"code":"…","message":"…"}}`.

| Method | Path | Auth | Success | Notes |
|---|---|---|---|---|
| POST | `/register` | no | `201 {user}` | validates captcha + input; creates user + personal namespace (unverified); sends verification mail |
| POST | `/login` | no | `200 {user, refresh_token}` + cookie | `identifier` may be username or e-mail |
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

When captcha is enabled, `pow-captcha-axum`'s routes are additionally mounted
under `/api/auth/captcha` (`POST /challenge`, `/redeem`, `/consume`). `GET
/captcha` issues a challenge directly so non-browser clients can start the flow.

### Error codes

| Code | HTTP | Trigger |
|---|---|---|
| `bad_request` | 400 | validation failure, invalid captcha or token |
| `invalid_credentials` | 401 | unknown identifier or wrong password |
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
