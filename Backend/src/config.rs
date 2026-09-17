use std::env;
use std::net::SocketAddr;
use std::path::PathBuf;

use anyhow::{Context, Result};

/// Runtime configuration, resolved entirely from environment variables so the
/// same binary can be driven by docker-compose or a systemd unit.
#[derive(Debug, Clone)]
pub struct Config {
    pub bind_addr: SocketAddr,
    /// Host used to build user-facing namespace paths, e.g. `registry.tohka.us`.
    pub public_host: String,
    /// Absolute base URL used in e-mails and redirects.
    pub base_url: String,

    pub database_url: String,
    pub database_dir: PathBuf,
    pub data_dir: PathBuf,
    pub logs_dir: PathBuf,

    pub jwt_secret: String,
    pub access_token_ttl_secs: i64,
    pub refresh_token_ttl_secs: i64,
    pub email_verification_ttl_secs: i64,
    pub password_reset_ttl_secs: i64,
    pub upload_session_ttl_secs: i64,
    /// Lifetime of a registry bearer token handed to `docker`.
    pub registry_token_ttl_secs: i64,
    /// Lifetime of the half-authenticated token minted after a correct password
    /// when the account has TOTP enabled.
    pub mfa_token_ttl_secs: i64,
    /// Lifetime of a registry refresh token — the `identitytoken` `docker login`
    /// stores and later trades for fresh bearer tokens.
    pub registry_refresh_token_ttl_secs: i64,
    /// Which `WWW-Authenticate` challenge `/v2` advertises on a `401`.
    pub registry_auth_challenge: RegistryAuthChallenge,

    pub cookie_name: String,
    pub cookie_domain: Option<String>,
    pub cookie_secure: bool,

    /// When true, `X-Forwarded-For` / `X-Real-IP` are trusted for client IPs.
    pub trust_proxy: bool,

    pub registration_enabled: bool,

    pub email_enabled: bool,
    pub brevo_api_key: Option<String>,
    pub brevo_sender_email: String,
    pub brevo_sender_name: String,

    pub captcha_enabled: bool,
    pub captcha_difficulty: u32,

    pub rate_limit_login_per_minute: u32,
    pub rate_limit_register_per_hour: u32,

    pub max_blob_size: Option<u64>,

    /// Estimated byte budget for the in-memory layer index cache.
    pub layer_cache_max_bytes: usize,
    /// Seconds a cached layer index is served before it is rebuilt.
    pub layer_cache_ttl_secs: u64,

    pub libravatar_base_url: String,
    pub title: String,
}

/// Which authentication challenge the registry advertises on `401` responses.
///
/// `Bearer` is the Docker token flow and the default. `Basic` is for clients
/// that speak the registry API but not the token dance; `Both` sends both
/// challenges, Bearer first, so either kind of client can proceed. Basic
/// credentials are accepted server-side regardless of this setting.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum RegistryAuthChallenge {
    #[default]
    Bearer,
    Basic,
    Both,
}

impl std::str::FromStr for RegistryAuthChallenge {
    type Err = anyhow::Error;

    fn from_str(value: &str) -> Result<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "bearer" => Ok(Self::Bearer),
            "basic" => Ok(Self::Basic),
            "both" => Ok(Self::Both),
            other => Err(anyhow::anyhow!(
                "invalid registry auth challenge {other:?} (expected bearer, basic or both)"
            )),
        }
    }
}

impl Config {
    pub fn from_env() -> Result<Self> {
        let host = env_or("LIGHTHOUSE_HOST", "0.0.0.0");
        let port: u16 = env_parse("LIGHTHOUSE_PORT", 8080)?;
        let bind_addr: SocketAddr = format!("{host}:{port}")
            .parse()
            .with_context(|| format!("invalid bind address {host}:{port}"))?;

        let public_host = env_or("PUBLIC_HOST", "localhost:8080");
        let scheme = env_or("PUBLIC_SCHEME", "http");
        let base_url = env_or("BASE_URL", &format!("{scheme}://{public_host}"));

        let database_dir = PathBuf::from(env_or("DATABASE_DIR", "./database"));
        let data_dir = PathBuf::from(env_or("DATA_DIR", "./data"));
        let logs_dir = PathBuf::from(env_or("LOGS_DIR", "./logs"));

        let default_db = format!("sqlite://{}/lighthouse.db?mode=rwc", database_dir.display());
        let database_url = env_or("DATABASE_URL", &default_db);

        let jwt_secret = env::var("JWT_SECRET").context(
            "JWT_SECRET must be set — generate one with `openssl rand -hex 32`",
        )?;

        Ok(Self {
            bind_addr,
            public_host: public_host.clone(),
            base_url,
            database_url,
            database_dir,
            data_dir,
            logs_dir,
            jwt_secret,

            access_token_ttl_secs: env_parse("ACCESS_TOKEN_TTL_SECS", 3600)?,
            refresh_token_ttl_secs: env_parse("REFRESH_TOKEN_TTL_SECS", 60 * 60 * 24 * 30)?,
            email_verification_ttl_secs: env_parse("EMAIL_VERIFICATION_TTL_SECS", 60 * 60 * 24)?,
            password_reset_ttl_secs: env_parse("PASSWORD_RESET_TTL_SECS", 60 * 60)?,
            upload_session_ttl_secs: env_parse("UPLOAD_SESSION_TTL_SECS", 60 * 60 * 24)?,
            registry_token_ttl_secs: env_parse("REGISTRY_TOKEN_TTL_SECS", 300)?,
            mfa_token_ttl_secs: env_parse("MFA_TOKEN_TTL_SECS", 300)?,
            registry_refresh_token_ttl_secs: env_parse(
                "REGISTRY_REFRESH_TOKEN_TTL_SECS",
                60 * 60 * 24 * 30,
            )?,
            registry_auth_challenge: env_parse(
                "REGISTRY_AUTH_CHALLENGE",
                RegistryAuthChallenge::Bearer,
            )?,

            cookie_name: env_or("COOKIE_NAME", "lighthouse_token"),
            cookie_domain: env::var("COOKIE_DOMAIN").ok().filter(|s| !s.is_empty()),
            cookie_secure: env_parse("COOKIE_SECURE", true)?,

            trust_proxy: env_parse("TRUST_PROXY", true)?,
            registration_enabled: env_parse("REGISTRATION_ENABLED", true)?,

            email_enabled: env_parse("EMAIL_ENABLED", false)?,
            brevo_api_key: env::var("BREVO_API_KEY").ok().filter(|s| !s.is_empty()),
            brevo_sender_email: env_or("BREVO_SENDER_EMAIL", "noreply@example.com"),
            brevo_sender_name: env_or("BREVO_SENDER_NAME", "Lighthouse Registry"),

            captcha_enabled: env_parse("CAPTCHA_ENABLED", false)?,
            captcha_difficulty: env_parse("CAPTCHA_DIFFICULTY", 18)?,

            rate_limit_login_per_minute: env_parse("RATE_LIMIT_LOGIN_PER_MINUTE", 10)?,
            rate_limit_register_per_hour: env_parse("RATE_LIMIT_REGISTER_PER_HOUR", 20)?,

            max_blob_size: env::var("MAX_BLOB_SIZE").ok().and_then(|v| v.parse().ok()),

            layer_cache_max_bytes: env_parse("LAYER_CACHE_MAX_BYTES", 64 * 1024 * 1024)?,
            layer_cache_ttl_secs: env_parse("LAYER_CACHE_TTL_SECS", 900)?,

            libravatar_base_url: env_or("LIBRAVATAR_BASE_URL", "https://seccdn.libravatar.org"),
            title: env_or("LIGHTHOUSE_TITLE", "Lighthouse"),
        })
    }
}

fn env_or(key: &str, default: &str) -> String {
    env::var(key).unwrap_or_else(|_| default.to_string())
}

fn env_parse<T>(key: &str, default: T) -> Result<T>
where
    T: std::str::FromStr,
    T::Err: std::fmt::Display,
{
    match env::var(key) {
        Ok(raw) if !raw.is_empty() => raw
            .parse::<T>()
            .map_err(|e| anyhow::anyhow!("invalid value for {key}: {e}")),
        _ => Ok(default),
    }
}
