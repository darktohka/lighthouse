//! Authentication request handlers (`/api/auth/*`).

use axum::Json;
use axum::Router;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{delete, get, post};
use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::auth::middleware::Authenticated;
use crate::auth::{LoginAttempt, sessions, tokens, two_factor};
use crate::captcha;
use crate::db::Db;
use crate::email;
use crate::error::{ApiError, ApiResult};
use crate::logging;
use crate::models::{LoginEvent, Namespace, Session, User};
use crate::oci::reference::parse_name_and_namespace;
use crate::ratelimit::Limiters;
use crate::state::AppState;

// ---- DTOs ------------------------------------------------------------------

#[derive(Debug, Deserialize)]
struct RegisterRequest {
    email: String,
    username: String,
    #[serde(default)]
    first_name: Option<String>,
    #[serde(default)]
    last_name: Option<String>,
    password: String,
    #[serde(default)]
    captcha: Option<String>,
}

#[derive(Debug, Deserialize)]
struct LoginRequest {
    identifier: String,
    password: String,
    #[serde(default)]
    captcha: Option<String>,
    #[serde(default)]
    code: Option<String>,
}

#[derive(Debug, Deserialize)]
struct TwoFactorLoginRequest {
    mfa_token: String,
    code: String,
}

#[derive(Debug, Deserialize)]
struct TwoFactorCodeRequest {
    code: String,
}

#[derive(Debug, Deserialize)]
struct TwoFactorDisableRequest {
    password: String,
    code: String,
}

#[derive(Debug, Deserialize)]
struct RefreshRequest {
    refresh_token: String,
}

#[derive(Debug, Deserialize)]
struct LogoutRequest {
    #[serde(default)]
    refresh_token: Option<String>,
}

#[derive(Debug, Deserialize)]
struct TokenRequest {
    token: String,
}

#[derive(Debug, Deserialize)]
struct EmailRequest {
    email: String,
    #[serde(default)]
    captcha: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ResetPasswordRequest {
    token: String,
    password: String,
}

#[derive(Debug, Deserialize)]
struct ChangePasswordRequest {
    current_password: String,
    new_password: String,
}

#[derive(Debug, Deserialize)]
struct PageQuery {
    #[serde(default)]
    limit: Option<i64>,
    #[serde(default)]
    offset: Option<i64>,
}

#[derive(Debug, Serialize)]
struct UserView {
    id: i64,
    username: String,
    email: String,
    first_name: Option<String>,
    last_name: Option<String>,
    email_verified: bool,
    is_admin: bool,
    avatar_url: Option<String>,
    theme: String,
    created_at: DateTime<Utc>,
}

impl From<&User> for UserView {
    fn from(user: &User) -> Self {
        Self {
            id: user.id,
            username: user.username.clone(),
            email: user.email.clone(),
            first_name: user.first_name.clone(),
            last_name: user.last_name.clone(),
            email_verified: user.email_verified,
            is_admin: user.is_admin,
            avatar_url: user.avatar_url.clone(),
            theme: user.theme.clone(),
            created_at: user.created_at,
        }
    }
}

#[derive(Debug, Serialize)]
struct SessionView {
    id: String,
    user_agent: Option<String>,
    ip: Option<String>,
    created_at: DateTime<Utc>,
    last_seen_at: DateTime<Utc>,
    expires_at: DateTime<Utc>,
    revoked: bool,
}

impl From<&Session> for SessionView {
    fn from(session: &Session) -> Self {
        Self {
            id: session.id.clone(),
            user_agent: session.user_agent.clone(),
            ip: session.ip.clone(),
            created_at: session.created_at,
            last_seen_at: session.last_seen_at,
            expires_at: session.expires_at,
            revoked: session.revoked_at.is_some(),
        }
    }
}

#[derive(Debug, Serialize)]
struct LoginEventView {
    id: i64,
    kind: String,
    success: bool,
    ip: Option<String>,
    user_agent: Option<String>,
    username_attempted: Option<String>,
    created_at: DateTime<Utc>,
}

impl From<&LoginEvent> for LoginEventView {
    fn from(event: &LoginEvent) -> Self {
        Self {
            id: event.id,
            kind: event.kind.clone(),
            success: event.success,
            ip: event.ip.clone(),
            user_agent: event.user_agent.clone(),
            username_attempted: event.username_attempted.clone(),
            created_at: event.created_at,
        }
    }
}

// ---- router ----------------------------------------------------------------

/// Builds the `/api/auth/*` routes.
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/auth/register", post(register))
        .route("/api/auth/login", post(login))
        .route("/api/auth/login/2fa", post(login_two_factor))
        .route("/api/auth/2fa", get(two_factor_status))
        .route("/api/auth/2fa/setup", post(two_factor_setup))
        .route("/api/auth/2fa/verify", post(two_factor_verify))
        .route("/api/auth/2fa/enable", post(two_factor_enable))
        .route("/api/auth/2fa/disable", post(two_factor_disable))
        .route("/api/auth/2fa/backup-codes", post(two_factor_backup_codes))
        .route("/api/auth/logout", post(logout))
        .route("/api/auth/refresh", post(refresh))
        .route("/api/auth/me", get(me))
        .route("/api/auth/captcha", get(captcha_challenge))
        .route("/api/auth/verify-email", post(verify_email))
        .route("/api/auth/resend-verification", post(resend_verification))
        .route("/api/auth/forgot-password", post(forgot_password))
        .route("/api/auth/reset-password", post(reset_password))
        .route("/api/auth/change-password", post(change_password))
        .route("/api/auth/sessions", get(list_sessions))
        .route("/api/auth/sessions/{id}", delete(revoke_session))
        .route("/api/auth/login-history", get(login_history))
}

// ---- handlers --------------------------------------------------------------

async fn register(
    State(state): State<AppState>,
    headers: HeaderMap,
    client_ip: logging::ClientIp,
    Json(request): Json<RegisterRequest>,
) -> ApiResult<Response> {
    if !state.config.registration_enabled {
        return Err(ApiError::new(
            StatusCode::FORBIDDEN,
            "registration_disabled",
            "registration is disabled",
        ));
    }

    let username = validate_username(&request.username)?;
    let email = validate_email(&request.email)?;
    validate_password(&request.password)?;
    captcha::verify(
        &state,
        request.captcha.as_deref().unwrap_or_default(),
        &headers,
    )
    .await?;

    let ip = client_ip.0;
    enforce_limit(
        state
            .limiters
            .check_register(ip.as_deref().unwrap_or("unknown")),
    )?;

    if username_taken(&state.db, &username).await? {
        return Err(ApiError::new(
            StatusCode::CONFLICT,
            "username_taken",
            "that username is not available",
        ));
    }
    if namespace_taken(&state.db, &username).await? {
        return Err(ApiError::new(
            StatusCode::CONFLICT,
            "namespace_taken",
            "that namespace is not available",
        ));
    }
    if email_taken(&state.db, &email).await? {
        return Err(ApiError::new(
            StatusCode::CONFLICT,
            "email_taken",
            "that e-mail address is already registered",
        ));
    }

    let password_hash = crate::auth::password::hash_password(&request.password)?;
    let now = Utc::now();

    let mut tx = state.db.begin().await?;
    let insert = sqlx::query(
        "INSERT INTO users \
         (username, email, first_name, last_name, password_hash, email_verified, is_admin, theme, created_at, updated_at) \
         VALUES (?, ?, ?, ?, ?, 0, 0, 'light', ?, ?)",
    )
    .bind(&username)
    .bind(&email)
    .bind(request.first_name.as_deref())
    .bind(request.last_name.as_deref())
    .bind(&password_hash)
    .bind(now)
    .bind(now)
    .execute(&mut *tx)
    .await;

    let inserted = match insert {
        Ok(result) => result,
        Err(err) => {
            tx.rollback().await.ok();
            if is_unique_violation(&err) {
                return Err(ApiError::new(
                    StatusCode::CONFLICT,
                    "conflict",
                    "username or e-mail already registered",
                ));
            }
            return Err(err.into());
        }
    };

    let user_id = inserted.last_insert_rowid();
    sqlx::query(
        "INSERT INTO namespaces (name, kind, owner_user_id, description, is_public, created_at, updated_at) \
         VALUES (?, 'user', ?, NULL, 1, ?, ?)",
    )
    .bind(&username)
    .bind(user_id)
    .bind(now)
    .bind(now)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;

    let token = issue_email_token(
        &state.db,
        user_id,
        tokens::EMAIL_KIND_VERIFY,
        state.config.email_verification_ttl_secs,
    )
    .await?;
    email::send_verification_email(&state.config, &email, &username, &token).await;

    let user = load_user(&state.db, user_id).await?;
    Ok((
        StatusCode::CREATED,
        Json(json!({ "user": UserView::from(&user) })),
    )
        .into_response())
}

async fn login(
    State(state): State<AppState>,
    headers: HeaderMap,
    client_ip: logging::ClientIp,
    Json(request): Json<LoginRequest>,
) -> ApiResult<Response> {
    let identifier = request.identifier.trim().to_string();
    captcha::verify(
        &state,
        request.captcha.as_deref().unwrap_or_default(),
        &headers,
    )
    .await?;

    let ip = client_ip.0;
    let user_agent = header_user_agent(&headers);
    enforce_limit(
        state
            .limiters
            .check_login(&Limiters::limit_key(ip.as_deref(), Some(&identifier))),
    )?;

    let user = find_user_by_identifier(&state.db, &identifier).await?;
    let Some(user) = user else {
        crate::auth::record_login_event(
            &state.db,
            LoginAttempt {
                user_id: None,
                service_account_id: None,
                username: Some(&identifier),
                success: false,
                kind: "password",
                ip: ip.as_deref(),
                user_agent: user_agent.as_deref(),
            },
        )
        .await;
        return Err(invalid_credentials());
    };

    if !user.email_verified {
        crate::auth::record_login_event(
            &state.db,
            LoginAttempt {
                user_id: Some(user.id),
                service_account_id: None,
                username: Some(&identifier),
                success: false,
                kind: "password",
                ip: ip.as_deref(),
                user_agent: user_agent.as_deref(),
            },
        )
        .await;
        return Err(ApiError::new(
            StatusCode::FORBIDDEN,
            "email_not_verified",
            "e-mail address has not been verified",
        ));
    }

    if !crate::auth::password::verify_password(&user.password_hash, &request.password) {
        crate::auth::record_login_event(
            &state.db,
            LoginAttempt {
                user_id: Some(user.id),
                service_account_id: None,
                username: Some(&identifier),
                success: false,
                kind: "password",
                ip: ip.as_deref(),
                user_agent: user_agent.as_deref(),
            },
        )
        .await;
        return Err(invalid_credentials());
    }

    let totp_enabled: bool = sqlx::query_scalar("SELECT totp_enabled FROM users WHERE id = ?")
        .bind(user.id)
        .fetch_one(&state.db)
        .await?;

    if !totp_enabled {
        return complete_login(&state, &headers, &user, ip.as_deref(), "password").await;
    }

    if let Some(code) = request.code.as_deref() {
        if two_factor::verify_login_code(&state, user.id, code, Utc::now()).await? {
            return complete_login(&state, &headers, &user, ip.as_deref(), "two_factor").await;
        }
        crate::auth::record_login_event(
            &state.db,
            LoginAttempt {
                user_id: Some(user.id),
                service_account_id: None,
                username: Some(&identifier),
                success: false,
                kind: "two_factor",
                ip: ip.as_deref(),
                user_agent: user_agent.as_deref(),
            },
        )
        .await;
        return Err(invalid_two_factor_code());
    }

    let mfa_token = tokens::issue_mfa_token(&state.config, user.id)?;
    Ok(Json(json!({ "two_factor_required": true, "mfa_token": mfa_token })).into_response())
}

async fn complete_login(
    state: &AppState,
    headers: &HeaderMap,
    user: &User,
    ip: Option<&str>,
    kind: &str,
) -> ApiResult<Response> {
    let user_agent = header_user_agent(headers);
    let (session, refresh_token) = sessions::create(
        state,
        user,
        sessions::SessionAudit {
            user_agent: user_agent.as_deref(),
            ip,
        },
    )
    .await?;
    let access_token =
        tokens::issue_access_token(&state.config, user.id, &user.username, &session.id)?;
    let cookie = sessions::set_access_cookie(&state.config, &access_token)?;

    crate::auth::record_login_event(
        &state.db,
        LoginAttempt {
            user_id: Some(user.id),
            service_account_id: None,
            username: Some(&user.username),
            success: true,
            kind,
            ip,
            user_agent: user_agent.as_deref(),
        },
    )
    .await;

    Ok(json_with_cookie(
        StatusCode::OK,
        json!({ "user": UserView::from(user), "refresh_token": refresh_token }),
        cookie,
    ))
}

async fn login_two_factor(
    State(state): State<AppState>,
    headers: HeaderMap,
    client_ip: logging::ClientIp,
    Json(request): Json<TwoFactorLoginRequest>,
) -> ApiResult<Response> {
    let claims = tokens::verify_mfa_token(&state.config, &request.mfa_token)
        .map_err(|_| ApiError::unauthorized("two-factor challenge is invalid or expired"))?;
    let user = load_user(&state.db, claims.sub).await?;

    let ip = client_ip.0;
    enforce_limit(
        state
            .limiters
            .check_login(&Limiters::limit_key(ip.as_deref(), Some(&user.username))),
    )?;

    if !user.email_verified {
        return Err(ApiError::new(
            StatusCode::FORBIDDEN,
            "email_not_verified",
            "e-mail address has not been verified",
        ));
    }

    if !two_factor::verify_login_code(&state, user.id, &request.code, Utc::now()).await? {
        crate::auth::record_login_event(
            &state.db,
            LoginAttempt {
                user_id: Some(user.id),
                service_account_id: None,
                username: Some(&user.username),
                success: false,
                kind: "two_factor",
                ip: ip.as_deref(),
                user_agent: header_user_agent(&headers).as_deref(),
            },
        )
        .await;
        return Err(invalid_two_factor_code());
    }

    complete_login(&state, &headers, &user, ip.as_deref(), "two_factor").await
}

async fn two_factor_status(
    State(state): State<AppState>,
    Authenticated(ctx): Authenticated,
) -> ApiResult<Response> {
    let user_id = ctx
        .user_id
        .ok_or_else(|| ApiError::unauthorized("user session required"))?;
    let status = two_factor::status(&state.db, user_id).await?;
    Ok(Json(status).into_response())
}

async fn two_factor_setup(
    State(state): State<AppState>,
    Authenticated(ctx): Authenticated,
) -> ApiResult<Response> {
    let user_id = ctx
        .user_id
        .ok_or_else(|| ApiError::unauthorized("user session required"))?;
    let issuer = state.config.title.clone();
    let setup = two_factor::begin_setup(&state, user_id, &issuer).await?;
    Ok(Json(setup).into_response())
}

async fn two_factor_verify(
    State(state): State<AppState>,
    Authenticated(ctx): Authenticated,
    Json(request): Json<TwoFactorCodeRequest>,
) -> ApiResult<Response> {
    let user_id = ctx
        .user_id
        .ok_or_else(|| ApiError::unauthorized("user session required"))?;
    two_factor::verify_setup(&state, user_id, &request.code, Utc::now()).await?;
    Ok(Json(json!({ "verified": true })).into_response())
}

async fn two_factor_enable(
    State(state): State<AppState>,
    Authenticated(ctx): Authenticated,
) -> ApiResult<Response> {
    let user_id = ctx
        .user_id
        .ok_or_else(|| ApiError::unauthorized("user session required"))?;
    two_factor::confirm_enable(&state, user_id, ctx.session_id.as_deref(), Utc::now()).await?;
    Ok(Json(json!({ "enabled": true })).into_response())
}

async fn two_factor_disable(
    State(state): State<AppState>,
    Authenticated(ctx): Authenticated,
    Json(request): Json<TwoFactorDisableRequest>,
) -> ApiResult<Response> {
    let user_id = ctx
        .user_id
        .ok_or_else(|| ApiError::unauthorized("user session required"))?;
    let user = load_user(&state.db, user_id).await?;
    two_factor::disable(
        &state,
        &user,
        &request.password,
        &request.code,
        ctx.session_id.as_deref(),
        Utc::now(),
    )
    .await?;
    Ok(Json(json!({ "enabled": false })).into_response())
}

async fn two_factor_backup_codes(
    State(state): State<AppState>,
    Authenticated(ctx): Authenticated,
    Json(request): Json<TwoFactorCodeRequest>,
) -> ApiResult<Response> {
    let user_id = ctx
        .user_id
        .ok_or_else(|| ApiError::unauthorized("user session required"))?;
    let codes =
        two_factor::regenerate_backup_codes(&state, user_id, &request.code, Utc::now()).await?;
    Ok(Json(json!({ "backup_codes": codes })).into_response())
}

async fn logout(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Option<Json<LogoutRequest>>,
) -> ApiResult<Response> {
    if let Some(token) = sessions::access_token_from_headers(&headers, &state.config) {
        if let Ok(claims) = tokens::verify_access_token(&state.config, &token) {
            sessions::revoke(&state, &claims.jti).await?;
        }
    } else if let Some(json) = body {
        if let Some(refresh) = json.refresh_token.as_deref() {
            if let Some(session) = sessions::find_by_refresh_token(&state.db, refresh).await? {
                sessions::revoke(&state, &session.id).await?;
            }
        }
    }

    let cookie = sessions::clear_access_cookie(&state.config)?;
    Ok(no_content_with_cookie(cookie))
}

async fn refresh(
    State(state): State<AppState>,
    Json(request): Json<RefreshRequest>,
) -> ApiResult<Response> {
    let session = sessions::find_by_refresh_token(&state.db, &request.refresh_token)
        .await?
        .ok_or_else(|| ApiError::unauthorized("refresh token is not valid"))?;

    let rotated = sessions::rotate_refresh(&state, &session.id, &request.refresh_token).await?;
    let cookie = sessions::set_access_cookie(&state.config, &rotated.access_token)?;

    crate::auth::record_login_event(
        &state.db,
        LoginAttempt {
            user_id: Some(session.user_id),
            service_account_id: None,
            username: None,
            success: true,
            kind: "refresh",
            ip: session.ip.as_deref(),
            user_agent: session.user_agent.as_deref(),
        },
    )
    .await;

    Ok(json_with_cookie(
        StatusCode::OK,
        json!({ "refresh_token": rotated.refresh_token }),
        cookie,
    ))
}

async fn me(
    State(state): State<AppState>,
    Authenticated(ctx): Authenticated,
) -> ApiResult<Response> {
    let user_id = ctx
        .user_id
        .ok_or_else(|| ApiError::unauthorized("user session required"))?;
    let user = load_user(&state.db, user_id).await?;
    let rows = sqlx::query_as::<_, Namespace>(
        "SELECT * FROM namespaces \
         WHERE owner_user_id = ? \
            OR id IN (SELECT namespace_id FROM namespace_members WHERE user_id = ?) \
         ORDER BY name COLLATE NOCASE",
    )
    .bind(user_id)
    .bind(user_id)
    .fetch_all(&state.db)
    .await?;

    // Serialize each row through the documented `Namespace` view (owner and
    // repository_count included) so `/auth/me` keeps the same contract as
    // `/api/namespaces` — the frontend parses both with one schema.
    let mut namespaces = Vec::with_capacity(rows.len());
    for namespace in &rows {
        namespaces.push(crate::api::namespaces::namespace_view(&state, &ctx, namespace).await?);
    }

    Ok(Json(json!({
        "user": UserView::from(&user),
        "namespaces": namespaces,
    }))
    .into_response())
}

async fn captcha_challenge(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> ApiResult<Response> {
    match captcha::issue(&state, &headers).await? {
        Some(challenge) => Ok(Json(challenge).into_response()),
        None => Ok(StatusCode::NO_CONTENT.into_response()),
    }
}

async fn verify_email(
    State(state): State<AppState>,
    Json(request): Json<TokenRequest>,
) -> ApiResult<Response> {
    verify_email_token(&state.db, &request.token).await?;
    Ok(Json(json!({ "verified": true })).into_response())
}

/// Consumes a verification token and marks the account verified.
///
/// Idempotent by design: verification links get opened twice by mail clients
/// that prefetch URLs, and browsers re-issue the request on reload. Re-verifying
/// an account that is already verified is a success, not an error, so a
/// duplicated request can never turn a successful verification into a failure.
async fn verify_email_token(db: &Db, token: &str) -> ApiResult<()> {
    let now = Utc::now();
    let mut tx = db.begin().await?;

    let row: Option<(i64, Option<DateTime<Utc>>, DateTime<Utc>)> = sqlx::query_as(
        "SELECT user_id, used_at, expires_at FROM email_tokens \
         WHERE token = ? AND kind = ? LIMIT 1",
    )
    .bind(token)
    .bind(tokens::EMAIL_KIND_VERIFY)
    .fetch_optional(&mut *tx)
    .await?;

    let Some((user_id, used_at, expires_at)) = row else {
        tx.rollback().await.ok();
        return Err(ApiError::bad_request("token is invalid or expired"));
    };

    if expires_at <= now {
        tx.rollback().await.ok();
        return Err(ApiError::bad_request("token is invalid or expired"));
    }

    let already_verified: bool =
        sqlx::query_scalar("SELECT email_verified FROM users WHERE id = ?")
            .bind(user_id)
            .fetch_one(&mut *tx)
            .await?;

    if used_at.is_some() {
        tx.rollback().await.ok();
        return if already_verified {
            Ok(())
        } else {
            Err(ApiError::bad_request("token is invalid or expired"))
        };
    }

    let claimed =
        sqlx::query("UPDATE email_tokens SET used_at = ? WHERE token = ? AND used_at IS NULL")
            .bind(now)
            .bind(token)
            .execute(&mut *tx)
            .await?;

    if claimed.rows_affected() == 0 {
        tx.rollback().await.ok();
        return if already_verified {
            Ok(())
        } else {
            Err(ApiError::bad_request("token is invalid or expired"))
        };
    }

    sqlx::query("UPDATE users SET email_verified = 1, updated_at = ? WHERE id = ?")
        .bind(now)
        .bind(user_id)
        .execute(&mut *tx)
        .await?;

    tx.commit().await?;
    Ok(())
}

async fn resend_verification(
    State(state): State<AppState>,
    client_ip: logging::ClientIp,
    Json(request): Json<EmailRequest>,
) -> ApiResult<Response> {
    let ip = client_ip.0;
    enforce_limit(
        state
            .limiters
            .check_register(ip.as_deref().unwrap_or("unknown")),
    )?;

    let email = request.email.trim();
    if let Some(user) = find_user_by_email(&state.db, email).await? {
        if !user.email_verified {
            let token = issue_email_token(
                &state.db,
                user.id,
                tokens::EMAIL_KIND_VERIFY,
                state.config.email_verification_ttl_secs,
            )
            .await?;
            email::send_verification_email(&state.config, &user.email, &user.username, &token)
                .await;
        }
    }
    Ok(accepted())
}

async fn forgot_password(
    State(state): State<AppState>,
    headers: HeaderMap,
    client_ip: logging::ClientIp,
    Json(request): Json<EmailRequest>,
) -> ApiResult<Response> {
    captcha::verify(
        &state,
        request.captcha.as_deref().unwrap_or_default(),
        &headers,
    )
    .await?;

    let ip = client_ip.0;
    enforce_limit(
        state
            .limiters
            .check_forgot_password(ip.as_deref().unwrap_or("unknown")),
    )?;

    let email = request.email.trim();
    if let Some(user) = find_user_by_email(&state.db, email).await? {
        let token = issue_email_token(
            &state.db,
            user.id,
            tokens::EMAIL_KIND_RESET,
            state.config.password_reset_ttl_secs,
        )
        .await?;
        email::send_password_reset_email(&state.config, &user.email, &user.username, &token).await;
    }
    Ok(accepted())
}

async fn reset_password(
    State(state): State<AppState>,
    Json(request): Json<ResetPasswordRequest>,
) -> ApiResult<Response> {
    validate_password(&request.password)?;
    let user_id = consume_email_token(&state.db, &request.token, tokens::EMAIL_KIND_RESET).await?;
    let hash = crate::auth::password::hash_password(&request.password)?;
    sqlx::query("UPDATE users SET password_hash = ?, updated_at = ? WHERE id = ?")
        .bind(&hash)
        .bind(Utc::now())
        .bind(user_id)
        .execute(&state.db)
        .await?;
    sessions::revoke_all_for_user(&state, user_id).await?;
    crate::auth::registry_refresh::revoke_for_user(&state, user_id).await?;
    Ok(Json(json!({ "reset": true })).into_response())
}

async fn change_password(
    State(state): State<AppState>,
    Authenticated(ctx): Authenticated,
    Json(request): Json<ChangePasswordRequest>,
) -> ApiResult<Response> {
    let user_id = ctx
        .user_id
        .ok_or_else(|| ApiError::unauthorized("user session required"))?;
    let user = load_user(&state.db, user_id).await?;
    if !crate::auth::password::verify_password(&user.password_hash, &request.current_password) {
        return Err(ApiError::new(
            StatusCode::UNAUTHORIZED,
            "invalid_credentials",
            "current password is incorrect",
        ));
    }
    validate_password(&request.new_password)?;
    let hash = crate::auth::password::hash_password(&request.new_password)?;
    sqlx::query("UPDATE users SET password_hash = ?, updated_at = ? WHERE id = ?")
        .bind(&hash)
        .bind(Utc::now())
        .bind(user_id)
        .execute(&state.db)
        .await?;
    sessions::revoke_all_for_user(&state, user_id).await?;
    crate::auth::registry_refresh::revoke_for_user(&state, user_id).await?;
    Ok(Json(json!({ "changed": true })).into_response())
}

async fn list_sessions(
    State(state): State<AppState>,
    Authenticated(ctx): Authenticated,
) -> ApiResult<Response> {
    let user_id = ctx
        .user_id
        .ok_or_else(|| ApiError::unauthorized("user session required"))?;
    let sessions = sessions::list_for_user(&state.db, user_id).await?;
    let views: Vec<SessionView> = sessions.iter().map(SessionView::from).collect();
    Ok(Json(json!({ "sessions": views })).into_response())
}

async fn revoke_session(
    State(state): State<AppState>,
    Authenticated(ctx): Authenticated,
    Path(id): Path<String>,
) -> ApiResult<Response> {
    let user_id = ctx
        .user_id
        .ok_or_else(|| ApiError::unauthorized("user session required"))?;
    let session = sessions::find(&state.db, &id)
        .await?
        .ok_or_else(|| ApiError::not_found("session not found"))?;
    if session.user_id != user_id {
        return Err(ApiError::not_found("session not found"));
    }
    sessions::revoke(&state, &id).await?;
    Ok(StatusCode::NO_CONTENT.into_response())
}

async fn login_history(
    State(state): State<AppState>,
    Authenticated(ctx): Authenticated,
    Query(query): Query<PageQuery>,
) -> ApiResult<Response> {
    let user_id = ctx
        .user_id
        .ok_or_else(|| ApiError::unauthorized("user session required"))?;
    let limit = query.limit.unwrap_or(50).clamp(1, 200);
    let offset = query.offset.unwrap_or(0).max(0);

    let events = sqlx::query_as::<_, LoginEvent>(
        "SELECT * FROM login_events WHERE user_id = ? \
         ORDER BY created_at DESC, id DESC LIMIT ? OFFSET ?",
    )
    .bind(user_id)
    .bind(limit)
    .bind(offset)
    .fetch_all(&state.db)
    .await?;
    let views: Vec<LoginEventView> = events.iter().map(LoginEventView::from).collect();

    Ok(Json(json!({
        "events": views,
        "limit": limit,
        "offset": offset,
    }))
    .into_response())
}

// ---- shared helpers --------------------------------------------------------

fn json_with_cookie(status: StatusCode, body: Value, cookie: HeaderValue) -> Response {
    let mut response = (status, Json(body)).into_response();
    response.headers_mut().append(header::SET_COOKIE, cookie);
    response
}

fn no_content_with_cookie(cookie: HeaderValue) -> Response {
    let mut response = StatusCode::NO_CONTENT.into_response();
    response.headers_mut().append(header::SET_COOKIE, cookie);
    response
}

fn accepted() -> Response {
    (StatusCode::ACCEPTED, Json(json!({ "status": "accepted" }))).into_response()
}

fn invalid_credentials() -> ApiError {
    ApiError::new(
        StatusCode::UNAUTHORIZED,
        "invalid_credentials",
        "invalid username or password",
    )
}

fn invalid_two_factor_code() -> ApiError {
    ApiError::new(
        StatusCode::UNAUTHORIZED,
        "invalid_two_factor_code",
        "invalid authentication code",
    )
}

fn enforce_limit(outcome: crate::ratelimit::LimitOutcome) -> ApiResult<()> {
    match outcome {
        crate::ratelimit::LimitOutcome::Allowed => Ok(()),
        crate::ratelimit::LimitOutcome::Limited { .. } => Err(ApiError::too_many_requests(
            "too many requests; try again later",
        )),
    }
}

fn header_user_agent(headers: &HeaderMap) -> Option<String> {
    headers
        .get(header::USER_AGENT)
        .and_then(|v| v.to_str().ok())
        .map(str::to_string)
}

fn validate_username(raw: &str) -> ApiResult<String> {
    let username = raw.trim().to_ascii_lowercase();
    if username.len() < 2 || username.len() > 39 || username.contains('/') {
        return Err(ApiError::bad_request(
            "username must be 2-39 lowercase characters",
        ));
    }
    match parse_name_and_namespace(&username) {
        Some((_, name)) if name == username => Ok(username),
        _ => Err(ApiError::bad_request(
            "username must be lowercase alphanumeric with `-` or `_`",
        )),
    }
}

fn validate_email(raw: &str) -> ApiResult<String> {
    let email = raw.trim().to_ascii_lowercase();
    let Some((local, domain)) = email.split_once('@') else {
        return Err(ApiError::bad_request("invalid e-mail address"));
    };
    if email.len() > 254
        || local.is_empty()
        || domain.is_empty()
        || !domain.contains('.')
        || domain.starts_with('.')
        || domain.ends_with('.')
        || email.contains(' ')
    {
        return Err(ApiError::bad_request("invalid e-mail address"));
    }
    Ok(email)
}

fn validate_password(password: &str) -> ApiResult<()> {
    if password.len() < 8 || password.len() > 128 {
        return Err(ApiError::bad_request(
            "password must be between 8 and 128 characters",
        ));
    }
    Ok(())
}

async fn username_taken(db: &Db, username: &str) -> ApiResult<bool> {
    let found: Option<i64> =
        sqlx::query_scalar("SELECT id FROM users WHERE username = ? COLLATE NOCASE LIMIT 1")
            .bind(username)
            .fetch_optional(db)
            .await?;
    Ok(found.is_some())
}

async fn namespace_taken(db: &Db, name: &str) -> ApiResult<bool> {
    let found: Option<i64> =
        sqlx::query_scalar("SELECT id FROM namespaces WHERE name = ? COLLATE NOCASE LIMIT 1")
            .bind(name)
            .fetch_optional(db)
            .await?;
    Ok(found.is_some())
}

async fn email_taken(db: &Db, email: &str) -> ApiResult<bool> {
    let found: Option<i64> =
        sqlx::query_scalar("SELECT id FROM users WHERE email = ? COLLATE NOCASE LIMIT 1")
            .bind(email)
            .fetch_optional(db)
            .await?;
    Ok(found.is_some())
}

fn is_unique_violation(err: &sqlx::Error) -> bool {
    matches!(err, sqlx::Error::Database(db) if db.message().contains("UNIQUE constraint failed"))
}

async fn load_user(db: &Db, user_id: i64) -> ApiResult<User> {
    sqlx::query_as::<_, User>("SELECT * FROM users WHERE id = ? LIMIT 1")
        .bind(user_id)
        .fetch_optional(db)
        .await?
        .ok_or_else(|| ApiError::not_found("user not found"))
}

async fn find_user_by_identifier(db: &Db, identifier: &str) -> ApiResult<Option<User>> {
    let user = sqlx::query_as::<_, User>(
        "SELECT * FROM users \
         WHERE username = ? COLLATE NOCASE OR email = ? COLLATE NOCASE LIMIT 1",
    )
    .bind(identifier)
    .bind(identifier)
    .fetch_optional(db)
    .await?;
    Ok(user)
}

async fn find_user_by_email(db: &Db, email: &str) -> ApiResult<Option<User>> {
    let user =
        sqlx::query_as::<_, User>("SELECT * FROM users WHERE email = ? COLLATE NOCASE LIMIT 1")
            .bind(email)
            .fetch_optional(db)
            .await?;
    Ok(user)
}

async fn issue_email_token(db: &Db, user_id: i64, kind: &str, ttl_secs: i64) -> ApiResult<String> {
    let token = tokens::generate_email_token();
    let now = Utc::now();
    sqlx::query(
        "INSERT INTO email_tokens (token, user_id, kind, created_at, expires_at, used_at) \
         VALUES (?, ?, ?, ?, ?, NULL)",
    )
    .bind(&token)
    .bind(user_id)
    .bind(kind)
    .bind(now)
    .bind(now + Duration::seconds(ttl_secs))
    .execute(db)
    .await?;
    Ok(token)
}

async fn consume_email_token(db: &Db, token: &str, kind: &str) -> ApiResult<i64> {
    let now = Utc::now();
    let mut tx = db.begin().await?;
    let user_id: Option<i64> = sqlx::query_scalar(
        "SELECT user_id FROM email_tokens \
         WHERE token = ? AND kind = ? AND used_at IS NULL AND expires_at > ? LIMIT 1",
    )
    .bind(token)
    .bind(kind)
    .bind(now)
    .fetch_optional(&mut *tx)
    .await?;

    let Some(user_id) = user_id else {
        tx.rollback().await.ok();
        return Err(ApiError::bad_request("token is invalid or expired"));
    };

    sqlx::query("UPDATE email_tokens SET used_at = ? WHERE token = ?")
        .bind(now)
        .bind(token)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(user_id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::Request as HttpRequest;
    use base64::Engine as _;
    use base64::engine::general_purpose::STANDARD;
    use serde_json::json;
    use tower::ServiceExt;

    use crate::auth::middleware::Auth;
    use crate::auth::test_support::{
        body_json, cookie_pair, create_user, email_token, set_cookie, test_state, test_state_with,
    };

    fn http_app(state: &AppState) -> Router {
        Router::new()
            .merge(crate::auth::router())
            .layer(axum::middleware::from_fn_with_state(
                state.clone(),
                crate::auth::middleware::resolve_identity,
            ))
            .with_state(state.clone())
    }

    async fn send(
        app: &Router,
        method: &str,
        uri: &str,
        body: Option<Value>,
        cookie: Option<&str>,
    ) -> Response {
        let mut builder = HttpRequest::builder().method(method).uri(uri);
        let body = match body {
            Some(value) => {
                builder = builder.header("content-type", "application/json");
                Body::from(value.to_string())
            }
            None => Body::empty(),
        };
        if let Some(cookie) = cookie {
            builder = builder.header("cookie", cookie);
        }
        app.clone()
            .oneshot(builder.body(body).expect("request"))
            .await
            .expect("response")
    }

    async fn send_with_header(app: &Router, uri: &str, name: &str, value: &str) -> Response {
        app.clone()
            .oneshot(
                HttpRequest::builder()
                    .uri(uri)
                    .header(name, value)
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response")
    }

    async fn whoami(auth: Auth) -> Json<Value> {
        let ctx = auth.0;
        Json(json!({
            "user_id": ctx.user_id,
            "username": ctx.username,
            "service_account_id": ctx.service_account_id,
            "is_admin": ctx.is_admin,
        }))
    }

    #[test]
    fn validates_usernames() {
        assert_eq!(validate_username(" Alice ").expect("ok"), "alice");
        assert!(validate_username("a").is_err());
        assert!(validate_username("bad name").is_err());
        assert!(validate_username("v2").is_err());
        assert!(validate_username("alice/bob").is_err());
    }

    #[test]
    fn validates_emails() {
        assert_eq!(
            validate_email(" A@Example.COM ").expect("ok"),
            "a@example.com"
        );
        assert!(validate_email("no-at-sign").is_err());
        assert!(validate_email("a@localhost").is_err());
        assert!(validate_email("a b@example.com").is_err());
    }

    #[test]
    fn enforces_password_length() {
        assert!(validate_password("short").is_err());
        assert!(validate_password("long-enough-password").is_ok());
        assert!(validate_password(&"x".repeat(129)).is_err());
    }

    #[tokio::test]
    async fn register_verify_login_me_logout_flow() {
        let (_dir, state) = test_state().await;
        let app = http_app(&state);

        let response = send(
            &app,
            "POST",
            "/api/auth/register",
            Some(json!({
                "email": "alice@example.com",
                "username": "alice",
                "first_name": "Alice",
                "last_name": "Doe",
                "password": "supersecret",
            })),
            None,
        )
        .await;
        assert_eq!(response.status(), StatusCode::CREATED);
        let user_id = body_json(response).await["user"]["id"]
            .as_i64()
            .expect("user id");

        let response = send(
            &app,
            "POST",
            "/api/auth/login",
            Some(json!({ "identifier": "alice", "password": "supersecret" })),
            None,
        )
        .await;
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
        assert_eq!(
            body_json(response).await["error"]["code"],
            "email_not_verified"
        );

        let token = email_token(&state, user_id, "verify_email").await;
        let response = send(
            &app,
            "POST",
            "/api/auth/verify-email",
            Some(json!({ "token": token })),
            None,
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);

        let response = send(
            &app,
            "POST",
            "/api/auth/login",
            Some(json!({ "identifier": "alice", "password": "supersecret" })),
            None,
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);
        assert!(
            set_cookie(&response, "lighthouse_token")
                .expect("access cookie")
                .contains("HttpOnly")
        );
        let cookie = cookie_pair(&response, "lighthouse_token").expect("access cookie");
        let login = body_json(response).await;
        assert_eq!(login["user"]["username"], "alice");
        assert!(
            login["refresh_token"]
                .as_str()
                .is_some_and(|v| !v.is_empty())
        );

        let response = send(&app, "GET", "/api/auth/me", None, Some(&cookie)).await;
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(body_json(response).await["user"]["username"], "alice");

        let session_id: String = sqlx::query_scalar(
            "SELECT id FROM sessions WHERE user_id = ? ORDER BY created_at DESC LIMIT 1",
        )
        .bind(user_id)
        .fetch_one(&state.db)
        .await
        .expect("session id");
        let response = send(&app, "POST", "/api/auth/logout", None, Some(&cookie)).await;
        assert_eq!(response.status(), StatusCode::NO_CONTENT);
        let revoked: Option<String> =
            sqlx::query_scalar("SELECT revoked_at FROM sessions WHERE id = ?")
                .bind(&session_id)
                .fetch_one(&state.db)
                .await
                .expect("revoked_at");
        assert!(revoked.is_some(), "logout must revoke the session");

        let response = send(&app, "GET", "/api/auth/me", None, None).await;
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn login_accepts_email_or_username() {
        let (_dir, state) = test_state().await;
        create_user(&state, "alice").await;
        let app = http_app(&state);

        for identifier in ["alice", "alice@test.local"] {
            let response = send(
                &app,
                "POST",
                "/api/auth/login",
                Some(json!({
                    "identifier": identifier,
                    "password": "correct-horse-battery",
                })),
                None,
            )
            .await;
            assert_eq!(response.status(), StatusCode::OK, "identifier={identifier}");
        }
    }

    #[tokio::test]
    async fn wrong_password_is_rejected() {
        let (_dir, state) = test_state().await;
        create_user(&state, "alice").await;
        let app = http_app(&state);

        let response = send(
            &app,
            "POST",
            "/api/auth/login",
            Some(json!({ "identifier": "alice", "password": "not-the-password" })),
            None,
        )
        .await;
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        assert_eq!(
            body_json(response).await["error"]["code"],
            "invalid_credentials"
        );
    }

    #[tokio::test]
    async fn rate_limiting_triggers_429() {
        let (_dir, state) = test_state_with(|config| {
            config.rate_limit_login_per_minute = 1;
        })
        .await;
        let app = http_app(&state);

        let first = send(
            &app,
            "POST",
            "/api/auth/login",
            Some(json!({ "identifier": "ghost", "password": "whatever" })),
            None,
        )
        .await;
        assert_eq!(first.status(), StatusCode::UNAUTHORIZED);

        let second = send(
            &app,
            "POST",
            "/api/auth/login",
            Some(json!({ "identifier": "ghost", "password": "whatever" })),
            None,
        )
        .await;
        assert_eq!(second.status(), StatusCode::TOO_MANY_REQUESTS);
    }

    #[tokio::test]
    async fn refresh_rotates_and_invalidates_the_old_token() {
        let (_dir, state) = test_state().await;
        create_user(&state, "alice").await;
        let app = http_app(&state);

        let response = send(
            &app,
            "POST",
            "/api/auth/login",
            Some(json!({ "identifier": "alice", "password": "correct-horse-battery" })),
            None,
        )
        .await;
        let refresh_token = body_json(response).await["refresh_token"]
            .as_str()
            .expect("refresh token")
            .to_string();

        let response = send(
            &app,
            "POST",
            "/api/auth/refresh",
            Some(json!({ "refresh_token": refresh_token })),
            None,
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);
        assert!(cookie_pair(&response, "lighthouse_token").is_some());
        let body = body_json(response).await;
        assert_ne!(body["refresh_token"].as_str(), Some(refresh_token.as_str()));

        let response = send(
            &app,
            "POST",
            "/api/auth/refresh",
            Some(json!({ "refresh_token": refresh_token })),
            None,
        )
        .await;
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn service_account_basic_auth_resolves_to_an_auth_context() {
        let (_dir, state) = test_state().await;
        let alice = create_user(&state, "alice").await;
        let (account, token) = crate::auth::service_accounts::create(
            &state,
            crate::auth::service_accounts::NewServiceAccount {
                owner_user_id: alice.id,
                name: "ci",
                username: "alice-ci",
                description: None,
            },
        )
        .await
        .expect("service account");

        let app = Router::new()
            .route("/whoami", axum::routing::get(whoami))
            .merge(crate::auth::router())
            .layer(axum::middleware::from_fn_with_state(
                state.clone(),
                crate::auth::middleware::resolve_identity,
            ))
            .with_state(state.clone());

        let credentials = STANDARD.encode(format!("alice-ci:{token}"));
        let response = send_with_header(
            &app,
            "/whoami",
            "authorization",
            &format!("Basic {credentials}"),
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);
        let body = body_json(response).await;
        assert_eq!(body["service_account_id"], account.id);
        assert_eq!(body["username"], "alice-ci");
        assert!(body["user_id"].is_null());

        let credentials = STANDARD.encode("alice:correct-horse-battery");
        let response = send_with_header(
            &app,
            "/whoami",
            "authorization",
            &format!("Basic {credentials}"),
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);
        let body = body_json(response).await;
        assert_eq!(body["user_id"], alice.id);
        assert!(body["service_account_id"].is_null());

        let logged: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM login_events WHERE service_account_id = ?")
                .bind(account.id)
                .fetch_one(&state.db)
                .await
                .expect("login events");
        assert!(logged >= 1);
    }

    #[tokio::test]
    async fn resend_and_forgot_never_reveal_account_existence() {
        let (_dir, state) = test_state().await;
        create_user(&state, "alice").await;
        let app = http_app(&state);

        for email in ["alice@test.local", "missing@example.com"] {
            let response = send(
                &app,
                "POST",
                "/api/auth/resend-verification",
                Some(json!({ "email": email })),
                None,
            )
            .await;
            assert_eq!(response.status(), StatusCode::ACCEPTED);

            let response = send(
                &app,
                "POST",
                "/api/auth/forgot-password",
                Some(json!({ "email": email })),
                None,
            )
            .await;
            assert_eq!(response.status(), StatusCode::ACCEPTED);
        }
    }

    #[tokio::test]
    async fn login_history_lists_attempts() {
        let (_dir, state) = test_state().await;
        let alice = create_user(&state, "alice").await;
        let app = http_app(&state);

        let response = send(
            &app,
            "POST",
            "/api/auth/login",
            Some(json!({ "identifier": "alice", "password": "wrong-password" })),
            None,
        )
        .await;
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);

        let response = send(
            &app,
            "POST",
            "/api/auth/login",
            Some(json!({ "identifier": "alice", "password": "correct-horse-battery" })),
            None,
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);
        let cookie = cookie_pair(&response, "lighthouse_token").expect("cookie");

        let response = send(&app, "GET", "/api/auth/login-history", None, Some(&cookie)).await;
        assert_eq!(response.status(), StatusCode::OK);
        let body = body_json(response).await;
        let events = body["events"].as_array().expect("events");
        assert!(!events.is_empty());
        assert_eq!(events[0]["success"], true);
        assert_eq!(events[0]["kind"], "password");

        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM login_events WHERE user_id = ?")
            .bind(alice.id)
            .fetch_one(&state.db)
            .await
            .expect("count");
        assert!(count >= 2, "success and failure attempts are both recorded");
    }

    #[tokio::test]
    async fn sessions_endpoint_lists_and_revokes() {
        let (_dir, state) = test_state().await;
        create_user(&state, "alice").await;
        let app = http_app(&state);

        let response = send(
            &app,
            "POST",
            "/api/auth/login",
            Some(json!({ "identifier": "alice", "password": "correct-horse-battery" })),
            None,
        )
        .await;
        let cookie = cookie_pair(&response, "lighthouse_token").expect("cookie");

        let response = send(&app, "GET", "/api/auth/sessions", None, Some(&cookie)).await;
        assert_eq!(response.status(), StatusCode::OK);
        let body = body_json(response).await;
        let sessions = body["sessions"].as_array().expect("sessions");
        let id = sessions[0]["id"].as_str().expect("id").to_string();

        let response = send(
            &app,
            "DELETE",
            &format!("/api/auth/sessions/{id}"),
            None,
            Some(&cookie),
        )
        .await;
        assert_eq!(response.status(), StatusCode::NO_CONTENT);

        let revoked: Option<String> =
            sqlx::query_scalar("SELECT revoked_at FROM sessions WHERE id = ?")
                .bind(&id)
                .fetch_one(&state.db)
                .await
                .expect("revoked");
        assert!(revoked.is_some());
    }

    #[tokio::test]
    async fn verify_email_is_idempotent() {
        let (_dir, state) = test_state().await;
        let app = http_app(&state);

        let response = send(
            &app,
            "POST",
            "/api/auth/register",
            Some(json!({
                "email": "carol@example.com",
                "username": "carol",
                "first_name": "Carol",
                "last_name": "Doe",
                "password": "supersecret",
            })),
            None,
        )
        .await;
        assert_eq!(response.status(), StatusCode::CREATED);

        let user_id: i64 = sqlx::query_scalar("SELECT id FROM users WHERE username = ?")
            .bind("carol")
            .fetch_one(&state.db)
            .await
            .expect("registered user");
        let token = email_token(&state, user_id, "verify_email").await;

        for attempt in 0..2 {
            let response = send(
                &app,
                "POST",
                "/api/auth/verify-email",
                Some(json!({ "token": token })),
                None,
            )
            .await;
            assert_eq!(response.status(), StatusCode::OK, "attempt {attempt}");
        }

        let verified: bool = sqlx::query_scalar("SELECT email_verified FROM users WHERE id = ?")
            .bind(user_id)
            .fetch_one(&state.db)
            .await
            .expect("verified flag");
        assert!(verified);

        let response = send(
            &app,
            "POST",
            "/api/auth/verify-email",
            Some(json!({ "token": "0".repeat(64) })),
            None,
        )
        .await;
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }

    async fn login_code(state: &AppState, user_id: i64) -> String {
        let secret: String = sqlx::query_scalar("SELECT totp_secret FROM users WHERE id = ?")
            .bind(user_id)
            .fetch_one(&state.db)
            .await
            .expect("secret");
        let bytes = crate::auth::totp::base32_decode(&secret).expect("decode");
        crate::auth::totp::format_code(crate::auth::totp::totp(&bytes, Utc::now().timestamp()))
    }

    #[tokio::test]
    async fn two_factor_login_requires_a_valid_code() {
        let (_dir, state) = test_state().await;
        let alice = create_user(&state, "alice").await;
        two_factor::begin_setup(&state, alice.id, "Lighthouse")
            .await
            .expect("setup");
        let secret: String = sqlx::query_scalar("SELECT totp_secret FROM users WHERE id = ?")
            .bind(alice.id)
            .fetch_one(&state.db)
            .await
            .expect("secret");
        let bytes = crate::auth::totp::base32_decode(&secret).expect("decode");
        // Enrol with a previous-step code so the current-step code stays unused
        // for the login that follows (TOTP codes are single-use).
        let enabled_at = Utc::now() - Duration::seconds(30);
        let enable_code =
            crate::auth::totp::format_code(crate::auth::totp::totp(&bytes, enabled_at.timestamp()));
        two_factor::verify_setup(&state, alice.id, &enable_code, enabled_at)
            .await
            .expect("verify");
        two_factor::confirm_enable(&state, alice.id, None, enabled_at)
            .await
            .expect("enable");

        let app = http_app(&state);

        let response = send(
            &app,
            "POST",
            "/api/auth/login",
            Some(json!({ "identifier": "alice", "password": "correct-horse-battery" })),
            None,
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);
        let body = body_json(response).await;
        assert_eq!(body["two_factor_required"], true);
        assert!(body["refresh_token"].is_null());
        let mfa_token = body["mfa_token"].as_str().expect("mfa token").to_string();

        let response = send(
            &app,
            "POST",
            "/api/auth/login/2fa",
            Some(json!({ "mfa_token": mfa_token, "code": "000000" })),
            None,
        )
        .await;
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        assert_eq!(
            body_json(response).await["error"]["code"],
            "invalid_two_factor_code"
        );

        let response = send(
            &app,
            "POST",
            "/api/auth/login/2fa",
            Some(json!({ "mfa_token": mfa_token, "code": login_code(&state, alice.id).await })),
            None,
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);
        let body = body_json(response).await;
        assert_eq!(body["user"]["username"], "alice");
        assert!(body["refresh_token"].as_str().is_some());
    }

    #[tokio::test]
    async fn two_factor_endpoints_require_a_session() {
        let (_dir, state) = test_state().await;
        let app = http_app(&state);
        for uri in ["/api/auth/2fa", "/api/auth/2fa/setup"] {
            let response = send(
                &app,
                if uri.ends_with("setup") {
                    "POST"
                } else {
                    "GET"
                },
                uri,
                None,
                None,
            )
            .await;
            assert_eq!(response.status(), StatusCode::UNAUTHORIZED, "{uri}");
        }
    }
}
