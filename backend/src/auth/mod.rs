//! Identity: registration, login, sessions, tokens and service accounts
//! (`/api/auth/*`).

pub mod app_passwords;
pub mod handlers;
pub mod ip_ranges;
pub mod middleware;
pub mod password;
pub mod registry;
pub mod registry_refresh;
pub mod service_accounts;
pub mod sessions;
pub mod token_endpoint;
pub mod tokens;
pub mod totp;
pub mod two_factor;

use axum::Router;

use crate::db::Db;
use crate::state::AppState;

/// A single login attempt to append to the account's history.
pub struct LoginAttempt<'a> {
    pub user_id: Option<i64>,
    pub service_account_id: Option<i64>,
    pub username: Option<&'a str>,
    pub success: bool,
    pub kind: &'a str,
    pub ip: Option<&'a str>,
    pub user_agent: Option<&'a str>,
}

/// Records a login attempt for the account owner's login history.
///
/// Best-effort: failures are logged and never fail the request.
pub async fn record_login_event(db: &Db, attempt: LoginAttempt<'_>) {
    let result = sqlx::query(
        "INSERT INTO login_events \
         (user_id, service_account_id, username_attempted, success, kind, ip, user_agent, created_at) \
         VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(attempt.user_id)
    .bind(attempt.service_account_id)
    .bind(attempt.username)
    .bind(attempt.success)
    .bind(attempt.kind)
    .bind(attempt.ip)
    .bind(attempt.user_agent)
    .bind(chrono::Utc::now())
    .execute(db)
    .await;

    if let Err(err) = result {
        tracing::warn!(error = %err, "failed to record login event");
    }
}

/// Builds the `/api/auth/*` router.
///
/// The identity middleware is applied globally in [`crate::routes::build`];
/// tests that mount this router standalone should add
/// [`middleware::resolve_identity`] themselves.
pub fn router() -> Router<AppState> {
    let mut router = handlers::router()
        .merge(middleware::router())
        .merge(password::router())
        .merge(service_accounts::router())
        .merge(sessions::router())
        .merge(token_endpoint::router())
        .merge(tokens::router());

    if let Some(captcha) = crate::captcha::service() {
        router = router.nest_service("/api/auth/captcha", captcha);
    }

    router
}

#[cfg(test)]
pub(crate) mod test_support {
    use std::net::SocketAddr;
    use std::path::Path;

    use tempfile::TempDir;

    use crate::config::Config;
    use crate::models::User;
    use crate::state::AppState;

    /// Configuration suitable for an isolated test instance.
    pub fn test_config(root: &Path) -> Config {
        Config {
            bind_addr: "127.0.0.1:0".parse::<SocketAddr>().expect("addr"),
            public_host: "localhost:8080".to_string(),
            base_url: "http://localhost:8080".to_string(),
            database_url: format!("sqlite://{}?mode=rwc", root.join("db").display()),
            database_dir: root.join("database"),
            data_dir: root.join("data"),
            logs_dir: root.join("logs"),
            jwt_secret: "test-secret-do-not-use".to_string(),
            access_token_ttl_secs: 3600,
            refresh_token_ttl_secs: 60 * 60 * 24 * 30,
            email_verification_ttl_secs: 60 * 60 * 24,
            password_reset_ttl_secs: 60 * 60,
            upload_session_ttl_secs: 60 * 60 * 24,
            registry_token_ttl_secs: 300,
            mfa_token_ttl_secs: 300,
            registry_refresh_token_ttl_secs: 60 * 60 * 24 * 30,
            registry_auth_challenge: crate::config::RegistryAuthChallenge::Bearer,
            cookie_name: "lighthouse_token".to_string(),
            cookie_domain: None,
            cookie_secure: false,
            trust_proxy: true,
            trusted_proxy_cidrs: Vec::new(),
            registration_enabled: true,
            email_enabled: false,
            brevo_api_key: None,
            brevo_sender_email: "noreply@example.com".to_string(),
            brevo_sender_name: "Lighthouse Registry".to_string(),
            captcha_enabled: false,
            captcha_difficulty: 4,
            rate_limit_login_per_minute: 10,
            rate_limit_register_per_hour: 20,
            max_blob_size: None,
            layer_cache_max_bytes: 64 * 1024 * 1024,
            layer_cache_ttl_secs: 900,
            layer_max_scan_bytes: 64 * 1024 * 1024,
            layer_max_entries: 100_000,
            libravatar_base_url: "https://seccdn.libravatar.org".to_string(),
            title: "Lighthouse".to_string(),
        }
    }

    /// Builds a migrated SQLite database and the shared application state.
    pub async fn test_state() -> (TempDir, AppState) {
        test_state_with(|_| {}).await
    }

    /// Like [`test_state`] but lets the caller adjust the configuration first.
    pub async fn test_state_with(mutate: impl FnOnce(&mut Config)) -> (TempDir, AppState) {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut config = test_config(dir.path());
        mutate(&mut config);
        let db = crate::db::connect(&config.database_url)
            .await
            .expect("connect");
        crate::db::migrate(&db).await.expect("migrate");
        let state = AppState::new(config, db).await.expect("state");
        (dir, state)
    }

    /// Returns the most recent e-mail token of `kind` for `user_id`.
    pub async fn email_token(state: &AppState, user_id: i64, kind: &str) -> String {
        sqlx::query_scalar(
            "SELECT token FROM email_tokens WHERE user_id = ? AND kind = ? \
             ORDER BY created_at DESC LIMIT 1",
        )
        .bind(user_id)
        .bind(kind)
        .fetch_one(&state.db)
        .await
        .expect("email token")
    }

    /// Inserts a verified user with the personal namespace `username`.
    pub async fn create_user(state: &AppState, username: &str) -> User {
        create_user_with(state, username, true).await
    }

    /// Inserts an unverified user with the personal namespace `username`.
    pub async fn create_unverified_user(state: &AppState, username: &str) -> User {
        create_user_with(state, username, false).await
    }

    async fn create_user_with(state: &AppState, username: &str, verified: bool) -> User {
        let email = format!("{username}@test.local");
        let hash =
            crate::auth::password::hash_password("correct-horse-battery").expect("hash password");
        let now = chrono::Utc::now();

        sqlx::query(
            "INSERT INTO users \
             (username, email, first_name, last_name, password_hash, email_verified, is_admin, theme, created_at, updated_at) \
             VALUES (?, ?, NULL, NULL, ?, ?, 0, 'light', ?, ?)",
        )
        .bind(username)
        .bind(&email)
        .bind(&hash)
        .bind(verified)
        .bind(now)
        .bind(now)
        .execute(&state.db)
        .await
        .expect("insert user");

        let user = sqlx::query_as::<_, User>(
            "SELECT * FROM users WHERE username = ? COLLATE NOCASE LIMIT 1",
        )
        .bind(username)
        .fetch_one(&state.db)
        .await
        .expect("load user");

        sqlx::query(
            "INSERT INTO namespaces (name, kind, owner_user_id, description, is_public, created_at, updated_at) \
             VALUES (?, 'user', ?, NULL, 1, ?, ?)",
        )
        .bind(username)
        .bind(user.id)
        .bind(now)
        .bind(now)
        .execute(&state.db)
        .await
        .expect("insert namespace");

        user
    }

    /// Inserts a namespace and returns its id.
    pub async fn create_namespace(
        state: &AppState,
        name: &str,
        owner_user_id: Option<i64>,
        is_public: bool,
    ) -> i64 {
        let now = chrono::Utc::now();
        sqlx::query(
            "INSERT INTO namespaces (name, kind, owner_user_id, description, is_public, created_at, updated_at) \
             VALUES (?, 'workspace', ?, NULL, ?, ?, ?)",
        )
        .bind(name)
        .bind(owner_user_id)
        .bind(is_public)
        .bind(now)
        .bind(now)
        .execute(&state.db)
        .await
        .expect("insert namespace");

        sqlx::query_scalar("SELECT id FROM namespaces WHERE name = ? COLLATE NOCASE LIMIT 1")
            .bind(name)
            .fetch_one(&state.db)
            .await
            .expect("namespace id")
    }

    /// Inserts a repository and returns its id.
    pub async fn create_repository(
        state: &AppState,
        namespace_id: i64,
        name: &str,
        is_public: bool,
        created_by: Option<i64>,
    ) -> i64 {
        let path = name.split_once('/').map(|(_, path)| path).unwrap_or(name);
        let now = chrono::Utc::now();
        sqlx::query(
            "INSERT INTO repositories \
             (namespace_id, name, path, description, is_public, created_by, created_at, updated_at) \
             VALUES (?, ?, ?, NULL, ?, ?, ?, ?)",
        )
        .bind(namespace_id)
        .bind(name)
        .bind(path)
        .bind(is_public)
        .bind(created_by)
        .bind(now)
        .bind(now)
        .execute(&state.db)
        .await
        .expect("insert repository");

        sqlx::query_scalar("SELECT id FROM repositories WHERE name = ? COLLATE NOCASE LIMIT 1")
            .bind(name)
            .fetch_one(&state.db)
            .await
            .expect("repository id")
    }

    /// Parses a response body as JSON.
    pub async fn body_json(response: axum::response::Response) -> serde_json::Value {
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("body");
        serde_json::from_slice(&bytes).expect("json body")
    }

    /// Extracts the `name=value` pair from a response's `Set-Cookie` header.
    pub fn cookie_pair(response: &axum::response::Response, name: &str) -> Option<String> {
        for value in response.headers().get_all(axum::http::header::SET_COOKIE) {
            let raw = value.to_str().ok()?;
            let pair = raw.split(';').next().unwrap_or(raw);
            if let Some((cookie_name, cookie_value)) = pair.split_once('=') {
                if cookie_name == name && !cookie_value.is_empty() {
                    return Some(pair.to_string());
                }
            }
        }
        None
    }

    /// Returns the full `Set-Cookie` header (including attributes) for `name`.
    pub fn set_cookie(response: &axum::response::Response, name: &str) -> Option<String> {
        let prefix = format!("{name}=");
        for value in response.headers().get_all(axum::http::header::SET_COOKIE) {
            let raw = value.to_str().ok()?;
            if raw.starts_with(&prefix) {
                return Some(raw.to_string());
            }
        }
        None
    }
}
