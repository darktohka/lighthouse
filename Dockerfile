# syntax=docker/dockerfile:1

# =============================================================================
# Lighthouse — multi-stage container build
#
#   frontend : Vite/React single-page app      -> /app/dist
#   builder  : statically linked Rust binary   -> /app/registry
#   final    : scratch (no shell, no libc)     -> /app/registry + /app/dist
#
# The final image carries a CA bundle because rustls (Brevo e-mail, Libravatar)
# needs trust anchors and `scratch` ships none.
# =============================================================================

# -----------------------------------------------------------------------------
# Stage 1 — frontend build (Vite + pnpm, npm fallback)
# -----------------------------------------------------------------------------
FROM node:24-alpine AS frontend

WORKDIR /app

# Node 24 bundles corepack. Disable the download prompt so the pnpm shim can
# fetch its release non-interactively. If corepack or pnpm is unavailable the
# install/build commands fall back to npm below.
ENV COREPACK_ENABLE_DOWNLOAD_PROMPT=0
RUN corepack enable || true

# The pnpm lockfile may not exist yet: the `*` glob matches it when present and
# is a no-op when absent (BuildKit tolerates an unmatched COPY glob).
COPY frontend/package.json frontend/pnpm-lock.yaml* ./
RUN pnpm install --no-frozen-lockfile || npm install

COPY frontend/ ./
RUN pnpm run build || npm run build

# -----------------------------------------------------------------------------
# Stage 2 — backend builder (static musl)
# -----------------------------------------------------------------------------
FROM rust:1.98.1-alpine AS builder

# aws-lc-rs (pulled in by reqwest/rustls) needs cmake/make/perl; libsqlite3-sys
# and the other *-sys crates compile C with gcc/g++/musl-dev. reqwest is built
# against rustls, so no openssl-dev is required.
RUN apk add --no-cache \
        musl-dev \
        gcc \
        g++ \
        make \
        binutils \
        cmake \
        perl \
        pkgconf \
        linux-headers \
        ca-certificates

# Map the buildx target architecture to its musl triple. The `uname -m` fallback
# keeps a plain `docker build` working if TARGETARCH is not injected.
ARG TARGETARCH
RUN case "${TARGETARCH:-$(uname -m)}" in \
        amd64 | x86_64) echo "x86_64-unknown-linux-musl" > /tmp/rust-target ;; \
        arm64 | aarch64) echo "aarch64-unknown-linux-musl" > /tmp/rust-target ;; \
        *) echo "unsupported architecture: ${TARGETARCH:-$(uname -m)}" >&2; exit 1 ;; \
    esac \
 && rustup target add "$(cat /tmp/rust-target)"

# Fully static output; the release profile also strips symbols.
ENV RUSTFLAGS="-C target-feature=+crt-static"

WORKDIR /app

# Dependency cache layer: cargo-chef is not part of the base image, so build a
# throwaway binary against the real manifests to warm the crate cache.
COPY backend/Cargo.toml backend/Cargo.lock ./
RUN mkdir -p src \
 && echo 'fn main() {}' > src/main.rs \
 && cargo build --profile release-lto --target "$(cat /tmp/rust-target)" \
 && rm -rf src

# Real sources plus migrations: sqlx::migrate!("./migrations") embeds these at
# compile time, so no database is needed during the build.
COPY backend/src ./src
COPY backend/migrations ./migrations

# COPY preserves the source mtimes, which can be older than the throwaway
# binary's artifacts — cargo would then wrongly consider the crate fresh. Drop
# the package's artifacts so the real server is always recompiled.
RUN cargo clean -p lighthouse-registry --profile release-lto --target "$(cat /tmp/rust-target)" \
 && cargo build --profile release-lto --target "$(cat /tmp/rust-target)" \
 && cp "target/$(cat /tmp/rust-target)/release-lto/registry" /app/registry

# Tiny static HTTP probe: the final image has no shell or curl and the server
# binary has no health subcommand, so /healthz is checked from this helper.
COPY <<'RS' /tmp/healthcheck.rs
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::process::exit;
use std::time::Duration;

fn main() {
    let port = std::env::var("LIGHTHOUSE_PORT").unwrap_or_else(|_| "8080".to_string());
    let host = std::env::var("LIGHTHOUSE_HOST").unwrap_or_else(|_| "127.0.0.1".to_string());
    let host = match host.as_str() {
        "0.0.0.0" | "::" | "[::]" | "localhost" => "127.0.0.1",
        other => other,
    };

    let addr: SocketAddr = match format!("{host}:{port}").parse() {
        Ok(addr) => addr,
        Err(_) => exit(1),
    };
    let mut stream = match TcpStream::connect_timeout(&addr, Duration::from_secs(3)) {
        Ok(stream) => stream,
        Err(_) => exit(1),
    };
    let _ = stream.set_read_timeout(Some(Duration::from_secs(3)));

    let request = format!("GET /healthz HTTP/1.0\r\nHost: {host}\r\nConnection: close\r\n\r\n");
    if stream.write_all(request.as_bytes()).is_err() {
        exit(1);
    }

    let mut response = String::new();
    if stream.read_to_string(&mut response).is_err() {
        exit(1);
    }

    if response.starts_with("HTTP/") && response.contains(" 200") {
        exit(0);
    }
    exit(1);
}
RS

RUN rustc --edition 2021 \
        -C opt-level=z -C lto=fat -C codegen-units=1 -C panic=abort -C strip=symbols \
        -C target-feature=+crt-static \
        --target "$(cat /tmp/rust-target)" \
        /tmp/healthcheck.rs -o /app/healthcheck \
 && mkdir -p /app/data /app/logs /app/database \
 && touch /app/data/.keep /app/logs/.keep /app/database/.keep \
 && printf 'lighthouse:x:10001:10001:Lighthouse:/app:/sbin/nologin\n' > /app/passwd \
 && printf 'lighthouse:x:10001:\n' > /app/group

# -----------------------------------------------------------------------------
# Stage 3 — final image (scratch)
# -----------------------------------------------------------------------------
FROM scratch AS final

LABEL org.opencontainers.image.title="Lighthouse" \
      org.opencontainers.image.description="Lighthouse — a Rust/Axum OCI registry with a Vite/React control plane" \
      org.opencontainers.image.source="https://github.com/darktohka/registry" \
      org.opencontainers.image.licenses="AGPL-3.0-or-later"

# Server binary and health probe (statically linked, symbols stripped).
COPY --from=builder /app/registry /app/registry
COPY --from=builder /app/healthcheck /app/healthcheck
# Built single-page app, served by the backend at /
COPY --from=frontend /app/dist /app/dist
# scratch has no trust store; copy the Alpine CA bundle for outbound TLS.
COPY --from=builder /etc/ssl/certs/ca-certificates.crt /etc/ssl/certs/ca-certificates.crt
# Give the unprivileged UID a name and pre-create writable runtime dirs.
# The docker-compose bind mounts shadow these paths on the host.
COPY --from=builder /app/passwd /etc/passwd
COPY --from=builder /app/group /etc/group
COPY --from=builder --chown=10001:10001 /app/data /app/data
COPY --from=builder --chown=10001:10001 /app/logs /app/logs
COPY --from=builder --chown=10001:10001 /app/database /app/database

WORKDIR /app
EXPOSE 8080

ENV LOGS_DIR=/app/logs \
    DATA_DIR=/app/data \
    DATABASE_DIR=/app/database \
    FRONTEND_DIR=/app/dist

# Non-root. Host bind mounts must be owned by UID/GID 10001 — see docs/DEPLOYMENT.md.
USER 10001:10001

HEALTHCHECK --interval=30s --timeout=5s --start-period=20s --retries=3 \
    CMD ["/app/healthcheck"]

ENTRYPOINT ["/app/registry"]
