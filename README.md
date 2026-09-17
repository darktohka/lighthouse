# Lighthouse

A self-hosted container image registry written in Rust, with a GitHub-styled web
control plane for users, namespaces, permissions, analytics and image browsing.

Lighthouse speaks the **OCI Distribution Specification** and the legacy **Docker
Registry HTTP API V2**, and adds a full control plane on top: accounts, e-mail
verification, workspaces, delegated access, service accounts, per-image pull
statistics, storage analytics and an in-browser layer filesystem viewer.

```
┌──────────────┐   /v2/*    ┌─────────────────────────────────────┐
│ docker push  │──────────▶ │  Axum  ·  OCI Distribution API      │
│ docker pull  │            │                                     │
└──────────────┘            │  SQLite (sqlx, WAL)   metadata      │
┌──────────────┐   /api/*   │  DATA_DIR/blobs/<alg>/<hex>  content │
│  Browser UI  │──────────▶ │                                     │
└──────────────┘            └─────────────────────────────────────┘
```

## Features

- **Registry protocol** — blobs (GET/HEAD/DELETE, `Range`, conditional requests),
  uploads (monolithic, chunked, resumable, status, cancel, cross-repo mount),
  manifests (GET/HEAD/PUT/DELETE, content negotiation), tag listing, repository
  catalog, `n`/`last` pagination, `Docker-Content-Digest`, OCI error envelope.
- **Formats** — OCI image manifest/index and Docker schema2 manifest/list;
  `tar`, `tar+gzip` and `tar+zstd` layers; multi-platform images.
- **Integrity** — digests verified cryptographically on every upload; blobs
  deduplicated and content-addressed.
- **Garbage collection** — reference-graph mark-and-sweep reclaims unreferenced
  blobs when tags or images are deleted.
- **Identity** — Argon2id passwords, TOTP two-factor authentication with
  single-use backup codes, per-account app passwords, e-mail verification
  through Brevo, stateless access cookie + refresh tokens, registry refresh
  tokens (offline tokens), service accounts with per-account IP allowlists,
  login history.
- **Authorization** — personal namespaces, workspaces, per-namespace and
  per-repository delegations (including anonymous), registry bearer tokens for
  `/v2` (anonymous pull included), a configurable `WWW-Authenticate` challenge
  (`Bearer`, `Basic` or both), push implies pull.
- **Control plane** — dashboard and activity timeline, image and tag detail with
  sizes and platforms, JSON manifest/config browsing,
  in-browser layer filesystem, storage analytics, profile pages with a yearly
  contribution heatmap and follows.
- **Operations** — daily-rotated logs with per-request IP/user-agent/timestamp,
  in-memory rate limiting, proof-of-work captcha, graceful SIGINT/SIGTERM
  shutdown, `scratch`-based image, multi-arch CI.

## Documentation

Everything lives in [`docs/`](docs/):

| Document | Contents |
|---|---|
| [`ARCHITECTURE.md`](docs/ARCHITECTURE.md) | module map, contracts, data flow, design decisions |
| [`API.md`](docs/API.md) | control-plane REST API reference |
| [`AUTH.md`](docs/AUTH.md) | identity model, sessions, cookies, auth endpoints |
| [`PERMISSIONS.md`](docs/PERMISSIONS.md) | namespace/repository authorization algorithm |
| [`OCI-COMPLIANCE.md`](docs/OCI-COMPLIANCE.md) | protocol surface and verified Docker-client behaviour |
| [`STORAGE.md`](docs/STORAGE.md) | on-disk layout, reference graph, garbage collection |
| [`LAYER_BROWSER.md`](docs/LAYER_BROWSER.md) | layer filesystem browsing design and safety caps |
| [`DEVELOPMENT.md`](docs/DEVELOPMENT.md) | **run the dev servers**, first push, troubleshooting |
| [`DEPLOYMENT.md`](docs/DEPLOYMENT.md) | building, running, reverse proxy, backups |
| [`FRONTEND.md`](docs/FRONTEND.md) | frontend structure, theming and API layer |
| [`DATABASE.md`](docs/DATABASE.md) | schema overview |

## Quick start

```bash
cp .env.example .env          # then set JWT_SECRET
docker compose up -d
```

The API listens on `:8080`. Register an account, verify the e-mail (with
`EMAIL_ENABLED=false` the message is written to the logs instead), and:

```bash
docker login <host>:8080 -u <username>
docker push <host>:8080/<username>/my-image:v1
docker pull <host>:8080/<username>/my-image:v1
```

For local development, run the backend with `cargo run` in `Backend/` and the
frontend with `pnpm dev` in `frontend/` (the dev server proxies `/api` and `/v2`).

## Layout

```
Backend/     Rust service (Axum, sqlx/SQLite, Tokio) — binary: registry
frontend/    Vite + React + TypeScript + Tailwind control plane
docs/        all documentation
Dockerfile   multi-stage build → scratch
```

Data lives in flat host directories so nothing is hidden inside Docker volumes:

```
database/    SQLite database (WAL)
data/        blobs, uploads and scratch space
logs/        YYYY-MM-DD.log
```

## Development

Two terminals — see [`docs/DEVELOPMENT.md`](docs/DEVELOPMENT.md) for the full
guide (env setup, first push, troubleshooting).

```bash
# terminal A — backend on :8080
cp .env.example .env && $EDITOR .env     # set JWT_SECRET, then use the dev profile
cargo run --manifest-path Backend/Cargo.toml

# terminal B — frontend on :5173 (proxies /api and /v2 to :8080)
cd frontend && pnpm install && pnpm dev
```

Open <http://localhost:5173>. Tests: `cargo test --manifest-path Backend/Cargo.toml`
(220 tests) and `pnpm build` in `frontend/`.

## License

AGPL-3.0-or-later.
