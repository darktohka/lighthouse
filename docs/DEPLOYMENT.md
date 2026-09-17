# Deploying Lighthouse

Lighthouse is a single Rust/Axum process that serves the OCI registry API, the
web control plane and the built SPA. Its only runtime dependency is a writable
directory tree — no separate database or cache service is required. Everything
is stored in three flat host directories.

---

## Prerequisites

- Docker Engine 24+ with the Compose v2 plugin (`docker compose version`).
- A host with at least 1 vCPU and 1 GB RAM; 2 vCPU / 2 GB is comfortable.
- Disk space for image blobs under `./data` (plan the size of the largest
  repositories you expect to mirror).
- For production: a domain name and a TLS-terminating reverse proxy (Caddy is
  shown below).

No Rust or Node toolchain is needed on the host — both are used only inside the
build stages of the image.

---

## Quick start (docker compose)

```sh
git clone https://github.com/darktohka/registry.git
cd registry

cp .env.example .env
openssl rand -hex 32            # put the output in JWT_SECRET inside .env
$EDITOR .env                    # set PUBLIC_HOST, BASE_URL, COOKIE_SECURE, ...

mkdir -p database data logs
sudo chown -R 10001:10001 database data logs

docker compose up -d
docker compose logs -f lighthouse
```

The registry is now on `http://localhost:8080`. Open it, register an account,
and follow [First run](#first-run) to activate the account.

The image runs as the unprivileged UID/GID **10001**. Docker creates missing
bind-mount source directories as `root`, so the `chown` step above is what makes
the three directories writable. If you prefer to run as root instead, add
`user: "0:0"` to the service in `docker-compose.yaml` and skip the `chown`.

---

## Building the image locally

```sh
# Multi-stage build: Vite frontend, static musl Rust binary, scratch runtime.
docker build -t darktohka/registry:dev .

# Build a single stage while iterating:
docker build --target frontend -t lighthouse-frontend:dev .
docker build --target builder  -t lighthouse-builder:dev  .
```

Then point compose at the local image by commenting out `image:` and
uncommenting `build: { context: . }` in `docker-compose.yaml`.

The final stage is `scratch`, so the runtime image contains only:

| Path                                  | Contents                                  |
| ------------------------------------- | ----------------------------------------- |
| `/app/registry`                       | statically linked server binary           |
| `/app/healthcheck`                    | tiny static probe for `GET /healthz`      |
| `/app/dist`                           | built SPA served at `/`                   |
| `/etc/ssl/certs/ca-certificates.crt`  | CA bundle for outbound TLS                |

There is no shell, package manager or build tooling in the runtime image.

---

## Directory layout

All state lives in the repository directory and is bind-mounted into the
container. No Docker named volumes are created.

```
.
├── docker-compose.yaml
├── .env                     # your configuration (git-ignored)
├── database/                # SQLite database (WAL journaling)
│   ├── lighthouse.db
│   ├── lighthouse.db-wal
│   └── lighthouse.db-shm
├── data/                    # OCI blobs, manifests and resumable uploads
└── logs/                    # daily application logs: YYYY-MM-DD.log (UTC)
```

| Host path   | Container path  | Environment variable |
| ----------- | --------------- | -------------------- |
| `./database`| `/app/database` | `DATABASE_DIR`       |
| `./data`    | `/app/data`     | `DATA_DIR`           |
| `./logs`    | `/app/logs`     | `LOGS_DIR`           |

---

## First run

Registration is open by default (`REGISTRATION_ENABLED=true`).

1. Open the web UI and register. The account is created in an **unverified**
   state and the server issues a verification token.
2. Login is rejected with `403 email_not_verified` until the address is
   verified.
3. Verification uses the link `{BASE_URL}/verify-email?token=…`.

### When `EMAIL_ENABLED=false` (the default)

No mail leaves the server. Instead the fully rendered message — including the
verification link — is written to the application log at `INFO` level, both to
stdout and to `logs/<date>.log`:

```sh
docker compose logs lighthouse | grep -A5 "e-mail delivery disabled"
# or
grep "verify-email" logs/*.log
```

Copy the `/verify-email?token=…` link into a browser to activate the account,
then log in. The same mechanism applies to password-reset e-mails. Use
`/api/auth/resend-verification` if the token expired.

### Delivering real e-mail (Brevo)

Set `EMAIL_ENABLED=true`, `BREVO_API_KEY=<key>`, `BREVO_SENDER_EMAIL` and
`BREVO_SENDER_NAME`. The sender address must be a validated sender in your
Brevo account. `BASE_URL` must be the externally reachable URL, otherwise the
links in the mail will not resolve.

### Administrator accounts

Registration always creates a non-admin user (`is_admin = 0`); there is no
first-user bootstrap. If you need administrator access, promote an account
directly in SQLite while the container is stopped:

```sh
docker compose stop lighthouse
sqlite3 database/lighthouse.db \
  "UPDATE users SET is_admin = 1 WHERE username = 'your-user';"
docker compose start lighthouse
```

---

## Reverse proxy (Caddy)

Terminate TLS in front of Lighthouse and forward the real client information.
The application trusts `X-Forwarded-For` (first value), `X-Real-IP`,
`CF-Connecting-IP` and `X-Client-IP` only when `TRUST_PROXY=true`. The client IP
is used for rate limiting and the login audit log, so it must be set correctly.

`/etc/caddy/Caddyfile`:

```caddyfile
registry.example.com {
    encode zstd gzip

    reverse_proxy 127.0.0.1:8080 {
        # Caddy sets these by default; shown explicitly for clarity.
        header_up X-Forwarded-For   {remote_host}
        header_up X-Forwarded-Proto {scheme}
        header_up X-Real-IP         {remote_host}
        header_up Host              {host}
    }
}
```

Matching `.env` values for the configuration above:

```sh
PUBLIC_HOST=registry.example.com
PUBLIC_SCHEME=https
BASE_URL=https://registry.example.com
TRUST_PROXY=true
COOKIE_SECURE=true
```

`X-Forwarded-Proto` is what tells the application the original request was
HTTPS; it is used when deciding how links and redirects are built. Make sure the
proxy sets it (Caddy does automatically) and that clients cannot spoof it, i.e.
Lighthouse is reachable only through the proxy.

Docker clients then use the registry directly:

```sh
docker tag myimage registry.example.com/<namespace>/myimage:latest
docker push registry.example.com/<namespace>/myimage:latest
```

---

## Cookies and `COOKIE_SECURE`

- `COOKIE_SECURE=true` (default) marks the session cookie `Secure`, so browsers
  only send it over HTTPS. Use it for every deployment behind TLS. You **must**
  also serve the UI over HTTPS, otherwise login silently fails because the
  cookie is dropped.
- `COOKIE_SECURE=false` is only for plain-HTTP local development
  (`http://localhost:8080`).
- `COOKIE_DOMAIN` can stay blank; the cookie is then scoped to the request host.
  Set it (e.g. `.example.com`) only if you serve the registry and the API on
  different subdomains.

If you change `PUBLIC_HOST`, `BASE_URL` or `COOKIE_*`, recreate the container so
the new environment is applied: `docker compose up -d --force-recreate`.

---

## Backups

State is entirely in `./database` and `./data`. Back up both together, and stop
the service first so the SQLite WAL is checkpointed and no upload is in flight:

```sh
docker compose stop lighthouse
tar czf lighthouse-backup-$(date +%F).tar.gz database data
docker compose start lighthouse
```

`./logs` is useful for auditing but not required for restore.

To restore, stop the container, replace `./database` and `./data` with the
archived copies, make sure `chown -R 10001:10001 database data`, and start again.
Migrations run automatically at startup.

---

## Upgrading

```sh
docker compose stop lighthouse                 # optional but recommended: backup first
docker compose pull                            # or rebuild with `build:`
docker compose up -d
docker compose logs -f lighthouse              # watch migrations complete
```

Database migrations are applied on boot. Pinning a specific release is
recommended for production — use the `:<git-sha>` or semver tag instead of
`latest`:

```yaml
image: darktohka/registry:1.2.3
```

---

## Resource sizing

- **CPU/RAM**: the server is I/O bound. 1 vCPU / 1 GB handles small instances;
  2 vCPU / 2 GB is a safe default. Argon2 password hashing is intentionally
  CPU-heavy and spikes briefly on login and registration.
- **Disk**: dominated by `./data` (stored blobs). Size it for the total size of
  the repositories you expect to hold, plus headroom; SQLite itself stays small.
- **Connections**: SQLite uses a bounded pool of 8 connections with a 5s busy
  timeout. A single instance is the supported topology.
- **Limits**: set `MAX_BLOB_SIZE` (bytes) to cap individual blob uploads, and
  tune `RATE_LIMIT_*` for public-facing instances.

---

## Troubleshooting

### `database is locked`

The server already runs SQLite in WAL mode with a 5s busy timeout and retries on
busy errors. Persistent locking almost always means the database directory is
shared incorrectly:

- Run **one** Lighthouse instance per `./database` directory.
- Do not put `./database` on NFS, SMB or other network filesystems; SQLite
  locking is unreliable there. Keep it on a local disk or a block volume.
- Check for a second process (an old container, a host `sqlite3` shell holding a
  write transaction) and stop it.
- Verify the container user can write the directory (see the next section).

### Permission errors on bind mounts

The image runs as UID/GID `10001`, but Docker creates missing bind-mount source
directories as `root`. Symptoms include crashes at startup, an empty database,
or `attempt to write a readonly database`.

```sh
docker compose stop lighthouse
sudo chown -R 10001:10001 database data logs
docker compose start lighthouse
```

Alternatively, run the service as root by adding `user: "0:0"` under the service
in `docker-compose.yaml` (not recommended for production).

### TLS certificate errors (e.g. from the e-mail or avatar client)

The `scratch` image has no system trust store, so the build copies
`/etc/ssl/certs/ca-certificates.crt` from the builder stage. Errors such as
`invalid peer certificate: UnknownIssuer` mean the bundle is missing or
incomplete:

- Confirm you are using the shipped image and did not replace the final stage.
- For a private/internal CA, append its root certificate to the bundle in an
  image layer, or point the runtime at it with `SSL_CERT_FILE`
  (`rustls-platform-verifier` / `rustls-native-certs` read `SSL_CERT_FILE` and
  `SSL_CERT_DIR`).
- Check that the container can reach the internet on 443 — outbound egress is
  often blocked by default on hardened hosts.

### Health check

The image health check and `docker-compose.yaml` call the bundled probe, which
performs `GET /healthz` and exits non-zero on failure. You can inspect it with:

```sh
docker inspect --format '{{json .State.Health}}' lighthouse
```

`/healthz` and `/api/health` return `200` when the process is up.
