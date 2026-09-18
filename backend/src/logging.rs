use std::env;
use std::fs::{File, OpenOptions};
use std::io::{self, Write};
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Instant;

use anyhow::Result;
use axum::extract::{ConnectInfo, FromRequestParts, Request, State};
use axum::http::HeaderMap;
use axum::http::request::Parts;
use axum::middleware::Next;
use axum::response::Response;
use ipnet::IpNet;
use tracing_subscriber::EnvFilter;
use tracing_subscriber::fmt::MakeWriter;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;

use crate::config::Config;
use crate::state::{AppState, AuthContext};

struct CachedFile {
    date: String,
    file: File,
}

static FILE: OnceLock<Mutex<Option<CachedFile>>> = OnceLock::new();

fn file_cell() -> &'static Mutex<Option<CachedFile>> {
    FILE.get_or_init(|| Mutex::new(None))
}

/// Writes each log line to `LOGS_DIR/YYYY-MM-DD.log`, rolling over at UTC
/// midnight. The open handle is cached and only reopened when the date changes.
#[derive(Clone)]
pub struct DailyFileWriter {
    dir: Arc<PathBuf>,
}

impl<'a> MakeWriter<'a> for DailyFileWriter {
    type Writer = DailyFileWriterHandle;

    fn make_writer(&'a self) -> Self::Writer {
        DailyFileWriterHandle {
            dir: Arc::clone(&self.dir),
        }
    }
}

pub struct DailyFileWriterHandle {
    dir: Arc<PathBuf>,
}

impl Write for DailyFileWriterHandle {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let today = chrono::Utc::now().format("%Y-%m-%d").to_string();
        let mut guard = file_cell().lock().unwrap_or_else(|e| e.into_inner());

        let stale = guard.as_ref().map(|c| c.date != today).unwrap_or(true);
        if stale {
            let path = self.dir.join(format!("{today}.log"));
            let file = OpenOptions::new().create(true).append(true).open(&path)?;
            *guard = Some(CachedFile { date: today, file });
        }

        guard
            .as_mut()
            .expect("log file initialised above")
            .file
            .write(buf)
    }

    fn flush(&mut self) -> io::Result<()> {
        let mut guard = file_cell().lock().unwrap_or_else(|e| e.into_inner());
        match guard.as_mut() {
            Some(cached) => cached.file.flush(),
            None => Ok(()),
        }
    }
}

pub fn init(config: &Config) -> Result<()> {
    std::fs::create_dir_all(&config.logs_dir)?;

    let level = env::var("RUST_LOG").unwrap_or_else(|_| "info".to_string());
    let filter = EnvFilter::try_new(&level).unwrap_or_else(|_| EnvFilter::new("info"));

    let stdout_layer = tracing_subscriber::fmt::layer()
        .with_ansi(true)
        .with_target(true);

    let file_layer = tracing_subscriber::fmt::layer()
        .with_ansi(false)
        .with_target(true)
        .with_writer(DailyFileWriter {
            dir: Arc::new(config.logs_dir.clone()),
        });

    tracing_subscriber::registry()
        .with(filter)
        .with(stdout_layer)
        .with(file_layer)
        .try_init()
        .ok();

    Ok(())
}

/// Client IP resolved by [`request_log`]/`resolve_identity` and stashed in the
/// request extensions so handlers and later middleware can read it without
/// parsing headers again. Its extractor never rejects.
#[derive(Clone, Debug, Default)]
pub struct ClientIp(pub Option<String>);

impl FromRequestParts<AppState> for ClientIp {
    type Rejection = std::convert::Infallible;

    async fn from_request_parts(
        parts: &mut Parts,
        _state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        Ok(parts
            .extensions
            .get::<ClientIp>()
            .cloned()
            .unwrap_or_default())
    }
}

/// Resolves the client IP from reverse-proxy headers.
///
/// For `X-Forwarded-For` the **rightmost** non-empty entry is used: Caddy (the
/// last proxy) appends the IP it actually observed last, whereas the leftmost
/// values can be attacker-supplied when proxies are chained.
pub fn forwarded_ip(headers: &HeaderMap) -> Option<String> {
    if let Some(value) = headers.get("x-forwarded-for").and_then(|v| v.to_str().ok()) {
        if let Some(last) = value
            .split(',')
            .map(str::trim)
            .rfind(|entry| !entry.is_empty())
        {
            return Some(last.to_string());
        }
    }
    for header in ["x-real-ip", "cf-connecting-ip", "x-client-ip"] {
        if let Some(value) = headers
            .get(header)
            .and_then(|v| v.to_str().ok())
            .filter(|value| !value.is_empty())
        {
            return Some(value.to_string());
        }
    }
    None
}

/// True when the connecting peer may be trusted to set forwarded headers.
///
/// An empty trusted list trusts every peer (backward compatible). A non-empty
/// list trusts only peers inside it; a missing peer cannot be verified, so it is
/// rejected (fail closed).
fn peer_is_trusted(peer: Option<SocketAddr>, trusted: &[IpNet]) -> bool {
    if trusted.is_empty() {
        return true;
    }
    let Some(peer) = peer else {
        return false;
    };
    trusted
        .iter()
        .any(|network| network.contains(&peer.ip().to_canonical()))
}

/// Resolves the client IP, honouring reverse-proxy headers only when the
/// connecting peer is trusted.
pub fn client_ip(req: &Request, config: &crate::config::Config) -> Option<String> {
    let peer = req
        .extensions()
        .get::<ConnectInfo<SocketAddr>>()
        .map(|ConnectInfo(addr)| *addr);

    if config.trust_proxy && peer_is_trusted(peer, &config.trusted_proxy_cidrs) {
        if let Some(forwarded) = forwarded_ip(req.headers()) {
            return Some(forwarded);
        }
    }

    peer.map(|addr| addr.ip().to_string())
}

pub fn user_agent(req: &Request) -> Option<String> {
    req.headers()
        .get(http::header::USER_AGENT)
        .and_then(|v| v.to_str().ok())
        .map(str::to_string)
}

/// Per-request access log: method, path, status, latency, client IP, user agent
/// and the authenticated actor (if any).
pub async fn request_log(State(state): State<AppState>, req: Request, next: Next) -> Response {
    let started = Instant::now();
    let method = req.method().clone();
    let path = req.uri().path().to_string();
    let query = req.uri().query().map(str::to_string);

    let ip = client_ip(&req, &state.config);
    let agent = user_agent(&req);
    let auth = req.extensions().get::<AuthContext>().cloned();

    let response = next.run(req).await;
    let status = response.status();
    let latency_ms = started.elapsed().as_millis() as u64;

    let (user_id, service_account_id) = match &auth {
        Some(ctx) => (ctx.user_id, ctx.service_account_id),
        None => (None, None),
    };

    tracing::info!(
        method = %method,
        path = %path,
        query = query.as_deref().unwrap_or(""),
        status = status.as_u16(),
        latency_ms,
        ip = ip.as_deref().unwrap_or("-"),
        user_agent = agent.as_deref().unwrap_or("-"),
        user_id = user_id.map(|v| v.to_string()).unwrap_or_else(|| "-".into()),
        service_account_id = service_account_id
            .map(|v| v.to_string())
            .unwrap_or_else(|| "-".into()),
        "http"
    );

    response
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::HeaderValue;

    fn config_with(trust_proxy: bool, cidrs: &[&str]) -> crate::config::Config {
        let mut config = crate::auth::test_support::test_config(std::path::Path::new("/tmp"));
        config.trust_proxy = trust_proxy;
        config.trusted_proxy_cidrs = cidrs
            .iter()
            .map(|cidr| cidr.parse().expect("trusted cidr"))
            .collect();
        config
    }

    fn request(peer: Option<SocketAddr>, forwarded: Option<&str>) -> Request {
        let mut builder = Request::builder().uri("/");
        if let Some(forwarded) = forwarded {
            builder = builder.header("x-forwarded-for", forwarded);
        }
        let mut request = builder.body(axum::body::Body::empty()).expect("request");
        if let Some(peer) = peer {
            request.extensions_mut().insert(ConnectInfo(peer));
        }
        request
    }

    #[test]
    fn forwarded_ip_uses_the_rightmost_entry() {
        let mut headers = HeaderMap::new();
        headers.insert(
            "x-forwarded-for",
            HeaderValue::from_static("203.0.113.9, 198.51.100.7, 10.0.0.1"),
        );
        assert_eq!(forwarded_ip(&headers).as_deref(), Some("10.0.0.1"));
    }

    #[test]
    fn forwarded_ip_falls_back_to_single_value_headers() {
        let mut headers = HeaderMap::new();
        headers.insert("x-real-ip", HeaderValue::from_static("192.0.2.4"));
        assert_eq!(forwarded_ip(&headers).as_deref(), Some("192.0.2.4"));
        assert_eq!(forwarded_ip(&HeaderMap::new()), None);
    }

    #[test]
    fn client_ip_honours_forwarded_headers_only_from_trusted_peers() {
        let peer: SocketAddr = "10.0.0.1:5555".parse().expect("peer");

        let trusted = config_with(true, &["10.0.0.0/8"]);
        let req = request(Some(peer), Some("203.0.113.9"));
        assert_eq!(client_ip(&req, &trusted).as_deref(), Some("203.0.113.9"));

        let untrusted = config_with(true, &["192.168.0.0/16"]);
        let req = request(Some(peer), Some("203.0.113.9"));
        assert_eq!(
            client_ip(&req, &untrusted).as_deref(),
            Some("10.0.0.1"),
            "an untrusted peer cannot spoof X-Forwarded-For"
        );

        let no_trust = config_with(false, &[]);
        let req = request(Some(peer), Some("203.0.113.9"));
        assert_eq!(client_ip(&req, &no_trust).as_deref(), Some("10.0.0.1"));
    }

    #[test]
    fn empty_trusted_list_is_backward_compatible() {
        let config = config_with(true, &[]);
        let req = request(None, Some("203.0.113.9"));
        assert_eq!(client_ip(&req, &config).as_deref(), Some("203.0.113.9"));
    }

    #[test]
    fn missing_peer_fails_closed_when_a_trust_list_is_configured() {
        let config = config_with(true, &["10.0.0.0/8"]);
        let req = request(None, Some("203.0.113.9"));
        assert_eq!(client_ip(&req, &config), None);
    }
}
