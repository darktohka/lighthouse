//! Credential extraction middleware producing [`AuthContext`].
//!
//! The middleware is best-effort: it never rejects a request. Credentials are
//! tried in priority order — access-token cookie, HTTP Basic (service account
//! first, then a password account) and finally a `Bearer` access token — and the
//! resolved identity is inserted as a request extension. Handlers enforce
//! authentication through the [`Auth`], [`Authenticated`] and [`Admin`]
//! extractors.

use std::convert::Infallible;

use axum::Router;
use axum::extract::{FromRequestParts, Request, State};
use axum::http::header;
use axum::http::request::Parts;
use axum::middleware::Next;
use axum::response::Response;
use base64::Engine as _;
use base64::engine::general_purpose::{STANDARD, STANDARD_NO_PAD};

use crate::auth::{
    LoginAttempt, app_passwords, password, record_login_event, service_accounts, sessions, tokens,
};
use crate::error::ApiError;
use crate::logging;
use crate::models::User;
use crate::state::{AppState, AuthContext, CredentialSource};

struct Credentials {
    cookie: Option<String>,
    basic: Option<BasicCredentials>,
    bearer: Option<String>,
}

struct BasicCredentials {
    username: String,
    password: String,
}

struct Audit {
    ip: Option<String>,
    agent: Option<String>,
}

/// Resolves the request identity and makes it available to downstream
/// middleware and handlers.
pub async fn resolve_identity(
    State(state): State<AppState>,
    mut req: Request,
    next: Next,
) -> Response {
    if req.extensions().get::<AuthContext>().is_none() {
        let announce = req.uri().path() != crate::auth::token_endpoint::TOKEN_PATH;
        let credentials = Credentials {
            cookie: sessions::access_token_from_headers(req.headers(), &state.config),
            basic: basic_credentials(req.headers()),
            bearer: bearer_token(req.headers()),
        };
        let audit = Audit {
            ip: logging::client_ip(&req, state.config.trust_proxy),
            agent: logging::user_agent(&req),
        };
        let ctx = resolve(&state, credentials, audit, announce).await;
        req.extensions_mut().insert(ctx);
    }
    next.run(req).await
}

async fn resolve(
    state: &AppState,
    credentials: Credentials,
    audit: Audit,
    announce: bool,
) -> AuthContext {
    if let Some(token) = credentials.cookie {
        if let Some(ctx) = user_from_access_token(state, &token).await {
            return ctx;
        }
    }

    if let Some(basic) = credentials.basic {
        return from_basic(state, &basic, &audit, announce).await;
    }

    if let Some(token) = credentials.bearer {
        if let Some(ctx) = user_from_access_token(state, &token).await {
            return ctx;
        }
        if let Ok(claims) = tokens::verify_registry_token(&state.config, &token) {
            return from_registry_token(claims);
        }
    }

    AuthContext::anonymous()
}

async fn user_from_access_token(state: &AppState, token: &str) -> Option<AuthContext> {
    let claims = tokens::verify_access_token(&state.config, token).ok()?;
    let is_admin: Option<bool> = sqlx::query_scalar("SELECT is_admin FROM users WHERE id = ?")
        .bind(claims.sub)
        .fetch_optional(&state.db)
        .await
        .ok()?;
    let is_admin = is_admin?;
    Some(AuthContext {
        user_id: Some(claims.sub),
        username: Some(claims.username),
        service_account_id: None,
        app_password_id: None,
        is_admin,
        credential: CredentialSource::AccessToken,
        session_id: Some(claims.jti),
    })
}

fn from_registry_token(claims: tokens::RegistryClaims) -> AuthContext {
    AuthContext {
        user_id: (claims.sub != 0).then_some(claims.sub),
        username: claims.username,
        service_account_id: claims.sid,
        app_password_id: None,
        is_admin: false,
        credential: CredentialSource::RegistryToken,
        session_id: None,
    }
}

async fn from_basic(
    state: &AppState,
    basic: &BasicCredentials,
    audit: &Audit,
    announce: bool,
) -> AuthContext {
    authenticate_basic(
        state,
        &basic.username,
        &basic.password,
        audit.ip.as_deref(),
        audit.agent.as_deref(),
        announce,
    )
    .await
}

/// Verifies a username/password pair against service accounts, app passwords and
/// account passwords, in that order. Shared by the request middleware and the
/// token endpoint's `password` grant, which receives credentials in the body.
pub async fn authenticate_basic(
    state: &AppState,
    username: &str,
    password: &str,
    ip: Option<&str>,
    user_agent: Option<&str>,
    announce: bool,
) -> AuthContext {
    if let Ok(Some(account)) = service_accounts::authenticate(state, username, password).await {
        if announce {
            record_login_event(
                &state.db,
                LoginAttempt {
                    user_id: None,
                    service_account_id: Some(account.id),
                    username: Some(username),
                    success: true,
                    kind: "service_token",
                    ip,
                    user_agent,
                },
            )
            .await;
        }
        return AuthContext {
            user_id: None,
            username: Some(account.username),
            service_account_id: Some(account.id),
            app_password_id: None,
            is_admin: false,
            credential: CredentialSource::ServiceAccount,
            session_id: None,
        };
    }

    let user =
        sqlx::query_as::<_, User>("SELECT * FROM users WHERE username = ? COLLATE NOCASE LIMIT 1")
            .bind(username)
            .fetch_optional(&state.db)
            .await
            .ok()
            .flatten();

    if let Some(user) = user {
        if user.email_verified {
            if let Ok(Some(account)) = app_passwords::authenticate(state, user.id, password).await {
                if announce {
                    record_login_event(
                        &state.db,
                        LoginAttempt {
                            user_id: Some(user.id),
                            service_account_id: None,
                            username: Some(username),
                            success: true,
                            kind: "app_password",
                            ip,
                            user_agent,
                        },
                    )
                    .await;
                }
                return AuthContext {
                    is_admin: user.is_admin,
                    user_id: Some(user.id),
                    username: Some(user.username),
                    service_account_id: None,
                    credential: CredentialSource::AppPassword,
                    session_id: None,
                    app_password_id: Some(account.id),
                };
            }

            if password::verify_password(&user.password_hash, password) {
                let totp_enabled: bool =
                    sqlx::query_scalar("SELECT totp_enabled FROM users WHERE id = ?")
                        .bind(user.id)
                        .fetch_one(&state.db)
                        .await
                        .unwrap_or(false);

                if !totp_enabled {
                    if announce {
                        record_login_event(
                            &state.db,
                            LoginAttempt {
                                user_id: Some(user.id),
                                service_account_id: None,
                                username: Some(username),
                                success: true,
                                kind: "password",
                                ip,
                                user_agent,
                            },
                        )
                        .await;
                    }
                    return AuthContext {
                        is_admin: user.is_admin,
                        user_id: Some(user.id),
                        username: Some(user.username),
                        service_account_id: None,
                        credential: CredentialSource::Password,
                        session_id: None,
                        app_password_id: None,
                    };
                }
            }
        }
    }

    if announce {
        record_login_event(
            &state.db,
            LoginAttempt {
                user_id: None,
                service_account_id: None,
                username: Some(username),
                success: false,
                kind: "password",
                ip,
                user_agent,
            },
        )
        .await;
    }
    AuthContext::anonymous()
}

fn basic_credentials(headers: &axum::http::HeaderMap) -> Option<BasicCredentials> {
    let value = headers.get(header::AUTHORIZATION)?.to_str().ok()?;
    let encoded = value
        .strip_prefix("Basic ")
        .or_else(|| value.strip_prefix("basic "))?;
    let decoded = STANDARD
        .decode(encoded)
        .or_else(|_| STANDARD_NO_PAD.decode(encoded))
        .ok()?;
    let text = String::from_utf8(decoded).ok()?;
    let (username, password) = text.split_once(':')?;
    Some(BasicCredentials {
        username: username.to_string(),
        password: password.to_string(),
    })
}

fn bearer_token(headers: &axum::http::HeaderMap) -> Option<String> {
    let value = headers.get(header::AUTHORIZATION)?.to_str().ok()?;
    let token = value
        .strip_prefix("Bearer ")
        .or_else(|| value.strip_prefix("bearer "))?;
    Some(token.to_string())
}

/// Extractor for the resolved identity; anonymous when no credentials were
/// presented.
#[derive(Clone, Debug)]
pub struct Auth(pub AuthContext);

impl FromRequestParts<AppState> for Auth {
    type Rejection = Infallible;

    async fn from_request_parts(
        parts: &mut Parts,
        _state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        Ok(Auth(
            parts
                .extensions
                .get::<AuthContext>()
                .cloned()
                .unwrap_or_default(),
        ))
    }
}

/// Extractor that rejects anonymous requests with `401`.
#[derive(Clone, Debug)]
pub struct Authenticated(pub AuthContext);

impl FromRequestParts<AppState> for Authenticated {
    type Rejection = ApiError;

    async fn from_request_parts(
        parts: &mut Parts,
        _state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        let ctx = parts
            .extensions
            .get::<AuthContext>()
            .cloned()
            .unwrap_or_default();
        if ctx.is_authenticated() && !ctx.is_registry_token() {
            Ok(Authenticated(ctx))
        } else {
            Err(ApiError::unauthorized("authentication required"))
        }
    }
}

/// Extractor that rejects non-administrators with `403`.
#[derive(Clone, Debug)]
pub struct Admin(pub AuthContext);

impl FromRequestParts<AppState> for Admin {
    type Rejection = ApiError;

    async fn from_request_parts(
        parts: &mut Parts,
        _state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        let ctx = parts
            .extensions
            .get::<AuthContext>()
            .cloned()
            .unwrap_or_default();
        if ctx.is_admin && !ctx.is_registry_token() {
            Ok(Admin(ctx))
        } else {
            Err(ApiError::forbidden("administrator access required"))
        }
    }
}

/// Empty router following the submodule convention.
pub fn router() -> Router<AppState> {
    Router::new()
}
