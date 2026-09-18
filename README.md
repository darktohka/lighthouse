# Lighthouse

A self-hosted container image registry written in Rust, with a GitHub-styled web
control plane for users, namespaces, permissions, analytics and image browsing.

Lighthouse speaks the **OCI Distribution Specification** and the legacy **Docker
Registry HTTP API V2**, and adds a full control plane on top: accounts, e-mail
verification, workspaces, delegated access, service accounts, per-image pull
statistics, storage analytics and an in-browser layer filesystem viewer.

## Hosted instance

A public Lighthouse instance runs at <https://lighthouse.tohka.us>. It is a
regular deployment — register an account there and you get the same personal
namespace, workspaces, delegations and browser UI described below.

The server image is published multi-arch (`linux/amd64`, `linux/arm64`) to
Docker Hub as [`darktohka/lighthouse`](https://hub.docker.com/r/darktohka/lighthouse),
and mirrored to GHCR as `ghcr.io/darktohka/registry`:

```bash
docker pull darktohka/lighthouse:latest   # or pin a release, e.g. :1.2.3
```

Tags are `latest` and `sha-<short-sha>`, plus semver (`1.2.3`, `1.2`, `1`) for
`v*` releases. See [`docs/DEPLOYMENT.md`](docs/DEPLOYMENT.md) for running it.

## Flow

```
┌──────────────┐   /v2/*    ┌─────────────────────────────────────┐
│ docker push  │──────────▶ │  Axum  ·  OCI Distribution API      │
│ docker pull  │            │                                     │
└──────────────┘            │  SQLite (sqlx, WAL)   metadata      │
┌──────────────┐   /api/*   │  DATA_DIR/blobs/<alg>/<hex>  content │
│  Browser UI  │──────────▶ │                                     │
└──────────────┘            └─────────────────────────────────────┘
```

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

For local development, run the backend with `cargo run` in `backend/` and the
frontend with `pnpm dev` in `frontend/` (the dev server proxies `/api` and `/v2`).

## Layout

```
backend/     Rust service (Axum, sqlx/SQLite, Tokio) — binary: registry
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
cargo run --manifest-path backend/Cargo.toml

# terminal B — frontend on :5173 (proxies /api and /v2 to :8080)
cd frontend && pnpm install && pnpm dev
```

Open <http://localhost:5173>. Tests: `cargo test --manifest-path backend/Cargo.toml`
(220 tests) and `pnpm build` in `frontend/`.

## License

AGPL-3.0-or-later.
