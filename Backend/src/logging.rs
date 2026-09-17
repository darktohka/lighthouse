use std::env;
use std::fs::{File, OpenOptions};
use std::io::{self, Write};
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Instant;

use anyhow::Result;
use axum::extract::{ConnectInfo, Request, State};
use axum::middleware::Next;
use axum::response::Response;
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

/// Resolves the client IP, honouring reverse-proxy headers when configured.
pub fn client_ip(req: &Request, trust_proxy: bool) -> Option<String> {
    if trust_proxy {
        if let Some(forwarded) = req.headers().get("x-forwarded-for") {
            if let Ok(value) = forwarded.to_str() {
                if let Some(first) = value.split(',').map(str::trim).find(|s| !s.is_empty()) {
                    return Some(first.to_string());
                }
            }
        }
        for header in ["x-real-ip", "cf-connecting-ip", "x-client-ip"] {
            if let Some(value) = req.headers().get(header) {
                if let Ok(value) = value.to_str() {
                    if !value.is_empty() {
                        return Some(value.to_string());
                    }
                }
            }
        }
    }

    req.extensions()
        .get::<ConnectInfo<SocketAddr>>()
        .map(|ConnectInfo(addr)| addr.ip().to_string())
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

    let ip = client_ip(&req, state.config.trust_proxy);
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
