# Lighthouse — Development

How to run Lighthouse locally and iterate on it.

---

## 1. Prerequisites

| Tool | Version | Notes |
|---|---|---|
| Rust | 1.85+ (edition 2024) | `rustup toolchain install stable` |
| Node.js | 24 | Vite 8 / TypeScript 6 |
| pnpm | 10+ | `corepack enable pnpm` |
| Docker | optional | only for the container path and for `docker push` testing |
| `sqlite3` | optional | inspecting `database/lighthouse.db` |

---

## 2. TL;DR — two terminals

**Terminal A — backend** (listens on `:8080`):

```bash
cd /path/to/registry
cp .env.example .env
openssl rand -hex 32          # paste the output into JWT_SECRET
$EDITOR .env                  # see the dev values in §3
cargo run --manifest-path backend/Cargo.toml
```

**Terminal B — frontend** (listens on `:5173`):

```bash
cd frontend
pnpm install
pnpm dev
```

Open **http://localhost:5173**. The frontend proxies `/api` and `/v2` to the
backend, so cookies and OCI calls work from one origin during development.

---

## 3. Backend dev server

### Where `.env` is read from

`dotenvy::dotenv()` searches the **current working directory and its parents**.
Running from the repository root therefore loads the root `.env` — the same file
`docker compose` reads — while running from `backend/` would load
`backend/.env` instead.

### `.env.example` is container-oriented

`.env.example` uses in-container paths (`/app/data`, `/app/database`,
`/app/logs`) and `COOKIE_SECURE=true`. Those are correct for Docker and wrong for
local development. Use this development profile instead:

```dotenv
# .env at the repository root — local development
LIGHTHOUSE_HOST=127.0.0.1
LIGHTHOUSE_PORT=8080
PUBLIC_HOST=localhost:8080
PUBLIC_SCHEME=http
# Point BASE_URL at the Vite dev server so verification links in the logs are clickable.
BASE_URL=http://localhost:5173

DATABASE_DIR=./database
DATABASE_URL=sqlite://./database/lighthouse.db?mode=rwc
DATA_DIR=./data
LOGS_DIR=./logs

# REQUIRED — the process exits with a clear message when this is unset.
JWT_SECRET=replace-me-with-openssl-rand-hex-32

# Plain HTTP on localhost: the cookie must not be marked Secure.
COOKIE_SECURE=false
TRUST_PROXY=false

# Development conveniences: no SMTP, no proof-of-work.
EMAIL_ENABLED=false
CAPTCHA_ENABLED=false

RUST_LOG=info
```

`.env` is git-ignored anywhere in the tree.

### Start it

```bash
cargo run --manifest-path backend/Cargo.toml      # from the repository root
```

or, if you prefer the shorter command, keep a `backend/.env` with the same values
and:

```bash
cd backend && cargo run
```

Either way the working directory decides where the data directories land. On
first start the process:

1. creates `database/`, `data/` and `logs/`,
2. applies the migrations in `backend/migrations/`,
3. binds the address, logging `lighthouse registry listening addr=…`.

Verify it is up:

```bash
curl -s http://localhost:8080/healthz        # {"status":"ok","service":"lighthouse"}
curl -s -o /dev/null -w '%{http_code}\n' http://localhost:8080/v2/   # 401 (challenge)
```

Press `Ctrl+C` (SIGINT) or send SIGTERM — the server drains in-flight requests,
closes the SQLite pool and exits cleanly.

### Reloading

There is no file watcher. Either restart with `Ctrl+C`, or install
[cargo-watch](https://crates.io/crates/cargo-watch) and run:

```bash
cargo watch -x 'run --manifest-path backend/Cargo.toml'
```

### Resetting local state

Everything lives in the three flat directories, so a clean slate is:

```bash
rm -rf database data logs        # stops being a registry with any images
```

---

## 4. Frontend dev server

```bash
cd frontend
pnpm install
pnpm dev
```

```
  VITE v8.3.0  ready in 199 ms
  ➜  Local:   http://localhost:5173/
```

`frontend/vite.config.ts` proxies both backends namespaces to the API:

| Proxied prefix | Target | Why |
|---|---|---|
| `/api` | `http://localhost:8080` | control plane; keeps the httpOnly access cookie on a single origin |
| `/v2` | `http://localhost:8080` | OCI endpoints (handy for `curl` against the dev origin) |

**If you change `LIGHTHOUSE_PORT`, update the proxy targets to match** — otherwise
every API call from the dev server fails with a connection error.

Scripts:

| Command | Purpose |
|---|---|
| `pnpm dev` | Vite dev server with HMR on `:5173` |
| `pnpm build` | `tsc -b && vite build` → `frontend/dist` |
| `pnpm preview` | serve the production build on `:4173` |
| `pnpm lint` | oxlint |

---

## 5. First push on a fresh instance

1. Open http://localhost:5173/register and create an account.
2. Verification is mandatory before the first login. With
   `EMAIL_ENABLED=false` no mail is sent — the rendered message, including the
   link, is written to the log:

   ```bash
   grep -m1 'verify-email?token=' logs/$(date +%F).log
   ```

   Open the printed URL (it points at `BASE_URL`, i.e. the dev server), or fetch
   the token directly:

   ```bash
   sqlite3 database/lighthouse.db \
     "SELECT token FROM email_tokens WHERE kind='verify_email' ORDER BY created_at DESC LIMIT 1;"
   ```

3. Log in, then push:

   ```bash
   docker login localhost:8080 -u <username>
   docker tag  alpine  localhost:8080/<username>/demo:v1
   docker push localhost:8080/<username>/demo:v1
   docker pull localhost:8080/<username>/demo:v1
   ```

   Docker treats `localhost` as an insecure (plain-HTTP) registry, so no daemon
   configuration is needed. For a non-localhost hostname you must add it to
   `insecure-registries` in `daemon.json`.

4. Watch the traffic: every request is logged with method, path, status, latency,
   client IP, user agent and the authenticated actor.

   ```bash
   tail -f logs/$(date +%F).log
   ```

---

## 6. Production-like single-origin mode

To run exactly what the container runs — backend serving the built SPA — without
building an image:

```bash
cd frontend && pnpm build && cd ..
FRONTEND_DIR=frontend/dist \
BASE_URL=http://localhost:8080 \
PUBLIC_HOST=localhost:8080 \
cargo run --manifest-path backend/Cargo.toml
```

Open http://localhost:8080. The SPA is served from `frontend/dist` with a
client-side-routing fallback; `/api` and `/v2` stay on the same origin.

---

## 7. Docker Compose

```bash
cp .env.example .env      # set JWT_SECRET; keep COOKIE_SECURE=true behind TLS
docker compose up --build -d
docker compose logs -f
```

Data is bind-mounted to `./database`, `./data` and `./logs` — no named volumes,
so you can inspect or back up the state directly from the host.

---

## 8. Everyday commands

| Task | Command |
|---|---|
| Backend tests | `cargo test --manifest-path backend/Cargo.toml` |
| Backend build | `cargo build --manifest-path backend/Cargo.toml` |
| Backend lint | `cargo clippy --manifest-path backend/Cargo.toml --all-targets` |
| Backend format | `cargo fmt --manifest-path backend/Cargo.toml` |
| Frontend dev | `cd frontend && pnpm dev` |
| Frontend build | `cd frontend && pnpm build` |
| Frontend lint | `cd frontend && pnpm lint` |
| Health | `curl localhost:8080/healthz` |

---

## 9. Troubleshooting

| Symptom | Cause / fix |
|---|---|
| `JWT_SECRET must be set — generate one with openssl rand -hex 32` | required variable missing; add it to `.env` or export it |
| Frontend loads but every request fails | backend not running, or `LIGHTHOUSE_PORT` no longer matches the Vite proxy in `vite.config.ts` |
| `address already in use` | another process holds the port: `ss -ltnp \| grep 8080`, or set `LIGHTHOUSE_PORT` (and update the proxy) |
| Redirected to login immediately after a successful login | `COOKIE_SECURE=true` over plain HTTP — set it to `false` for localhost |
| Verification e-mail never arrives | expected with `EMAIL_ENABLED=false`; read the link from `logs/` or the `email_tokens` table |
| “Verification failed — token is invalid or expired”, but the account is already verified | the page fired the request twice (React StrictMode in dev does this); the first call consumed the token. `/verify-email` is idempotent so both now return `200` — if you are on an older build, just sign in: the account is active |
| `database is locked` | another instance running against the same `DATABASE_DIR`, or an external `sqlite3` write transaction; the pool waits 5 s and retries |
| `docker push` → `unauthorized` | log in first; the registry must answer `401` on `/v2/` (it does) and Docker must hold credentials for that host |
| `docker push` → `connection refused` | the address in the image name is wrong, or the backend is not listening on it |
| Captcha blocks local login | set `CAPTCHA_ENABLED=false` |
| Permission denied writing `data/`, `database/`, `logs/` | when running the container, those bind mounts must be writable by UID/GID `10001` |
| Blobs linger after deleting a tag | reclamation is reference-counted; the image must have no other tags or index parents before its blobs are collected |
