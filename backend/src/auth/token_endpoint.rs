//! Registry token endpoint (`/api/auth/token`).
//!
//! `docker` discovers this URL through the `WWW-Authenticate: Bearer
//! realm="…/api/auth/token",service="…"` challenge. A call with no credentials
//! receives an anonymous token; valid Basic credentials (service token, app
//! password, or an account password with TOTP disabled) yield a token bound to
//! that identity. The requested scope is resolved against the same permission
//! model as a live request, and a scope that is not granted simply yields an
//! empty `access` list — the registry then answers `DENIED`, which is what gives
//! `docker` its clean "pull access denied … may require 'docker login'" message.
//!
//! Beyond the Basic probe the endpoint implements the OAuth2 grants a registry
//! client uses for long-lived logins: `offline_token=true` (GET) or
//! `access_type=offline` (POST) returns a `refresh_token`, and
//! `grant_type=refresh_token` trades it for a new bearer token — see
//! [`crate::auth::registry_refresh`]. `grant_type=password` accepts credentials
//! in the form body.
//!
//! Only Basic is honoured for the probe. A cookie is deliberately ignored so a
//! browser session cannot be turned into a bearer token (a CSRF mint surface).

use axum::Json;
use axum::Router;
use axum::body::Bytes;
use axum::extract::{RawQuery, State};
use axum::http::{HeaderMap, Method, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use chrono::Utc;

use crate::auth::ip_ranges;
use crate::auth::middleware::{self, Auth};
use crate::auth::registry;
use crate::auth::registry_refresh::{self, Principal};
use crate::auth::tokens::{self, RegistryAccess};
use crate::error::ApiError;
use crate::state::{AppState, AuthContext, CredentialSource};

/// Path advertised as the token endpoint in the `/v2` challenge.
pub const TOKEN_PATH: &str = "/api/auth/token";

pub fn router() -> Router<AppState> {
    Router::new().route(TOKEN_PATH, get(token).post(token))
}

async fn token(
    State(state): State<AppState>,
    auth: Auth,
    method: Method,
    headers: HeaderMap,
    RawQuery(raw): RawQuery,
    client_ip: crate::logging::ClientIp,
    body: Bytes,
) -> Response {
    let mut params = query_pairs(raw.as_deref());
    if !body.is_empty() {
        params.extend(query_pairs(std::str::from_utf8(&body).ok()));
    }
    if method == Method::GET
        && params
            .iter()
            .any(|(key, value)| key == "grant_type" && value == "refresh_token")
    {
        return oauth_error(
            "invalid_request",
            "grant_type=refresh_token must be sent in a POST body",
        );
    }
    issue(&state, &auth.0, &headers, &params, client_ip.0.as_deref()).await
}

async fn issue(
    state: &AppState,
    actor: &AuthContext,
    headers: &HeaderMap,
    params: &[(String, String)],
    client_ip: Option<&str>,
) -> Response {
    let has_basic = has_basic_credentials(headers);
    let requested = scope_parameters(params);
    let scopes = registry::parse_scopes(&requested);
    let client_id = param(params, "client_id");
    let grant_type = param(params, "grant_type");

    if grant_type == Some("refresh_token") {
        let Some(presented) = param(params, "refresh_token") else {
            return oauth_error("invalid_request", "refresh_token is required");
        };
        return match registry_refresh::redeem(state, presented, &requested).await {
            Ok(redeemed) => {
                if let Some(service_account_id) = redeemed.service_account_id
                    && !ip_ranges::allowed(&state.db, service_account_id, client_ip).await
                {
                    return oauth_challenge_error(
                        registry_refresh::INVALID_GRANT,
                        "refresh token is invalid, expired or revoked",
                    );
                }
                mint(
                    state,
                    redeemed.subject,
                    Some(&redeemed.username),
                    redeemed.service_account_id,
                    redeemed.access,
                    None,
                )
            }
            Err(err) if err.code == registry_refresh::INVALID_GRANT => {
                oauth_challenge_error("invalid_grant", &err.message)
            }
            Err(err) => err.into_response(),
        };
    }

    if matches!(grant_type, Some(other) if other != "password") {
        return oauth_error(
            "unsupported_grant_type",
            "only password and refresh_token are supported",
        );
    }

    let (resolved, authenticated) = if grant_type == Some("password") && !has_basic {
        let (Some(username), Some(password)) =
            (param(params, "username"), param(params, "password"))
        else {
            return oauth_error("invalid_request", "username and password are required");
        };
        let resolved =
            middleware::authenticate_basic(state, username, password, client_ip, None, false).await;
        let authenticated = resolved.is_authenticated();
        (resolved, authenticated)
    } else {
        let authenticated = has_basic
            && actor.is_authenticated()
            && actor.credential != CredentialSource::RegistryToken;
        (actor.clone(), authenticated)
    };

    if grant_type == Some("password") && !authenticated {
        return oauth_error("invalid_grant", "invalid credentials");
    }
    if has_basic && !authenticated && grant_type.is_none() {
        return unauthorized_with_basic();
    }

    let mut access: Vec<RegistryAccess> = Vec::new();
    if authenticated {
        for scope in &scopes {
            let actions = registry::granted_actions(state, &resolved, scope).await;
            if !actions.is_empty() {
                access.push(RegistryAccess {
                    kind: scope.kind.clone(),
                    name: scope.name.clone(),
                    actions,
                });
            }
        }
    }

    let refresh_token = if authenticated && offline_requested(params) {
        match Principal::from_auth_context(&resolved) {
            Some(principal) => {
                match registry_refresh::issue(state, &principal, &requested, client_id).await {
                    Ok(token) => Some(token),
                    Err(err) => {
                        tracing::error!(error = %err, "failed to issue registry refresh token");
                        None
                    }
                }
            }
            None => None,
        }
    } else {
        None
    };

    let sub = if authenticated {
        resolved.user_id.unwrap_or(0)
    } else {
        0
    };
    mint(
        state,
        sub,
        resolved.username.as_deref(),
        resolved.service_account_id,
        access,
        refresh_token.as_deref(),
    )
}

fn mint(
    state: &AppState,
    sub: i64,
    username: Option<&str>,
    service_account_id: Option<i64>,
    access: Vec<RegistryAccess>,
    refresh_token: Option<&str>,
) -> Response {
    let token = match tokens::issue_registry_token(
        &state.config,
        sub,
        username,
        service_account_id,
        access,
    ) {
        Ok(token) => token,
        Err(err) => {
            tracing::error!(error = %err, "failed to issue registry token");
            return ApiError::internal("failed to issue registry token").into_response();
        }
    };

    let mut body = serde_json::json!({
        "token": token,
        "access_token": token,
        "expires_in": state.config.registry_token_ttl_secs,
        "issued_at": Utc::now().to_rfc3339(),
    });
    if let Some(refresh_token) = refresh_token {
        body["refresh_token"] = serde_json::Value::String(refresh_token.to_string());
    }
    Json(body).into_response()
}

fn param<'a>(params: &'a [(String, String)], key: &str) -> Option<&'a str> {
    params
        .iter()
        .find(|(name, _)| name == key)
        .map(|(_, value)| value.as_str())
}

fn scope_parameters(params: &[(String, String)]) -> Vec<String> {
    params
        .iter()
        .filter(|(key, _)| key == "scope")
        .map(|(_, value)| value.clone())
        .collect()
}

fn has_basic_credentials(headers: &HeaderMap) -> bool {
    headers
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value.starts_with("Basic ") || value.starts_with("basic "))
}

fn offline_requested(params: &[(String, String)]) -> bool {
    match param(params, "offline_token") {
        Some(value) => value.is_empty() || is_truthy(value),
        None => {
            param(params, "access_type").is_some_and(|value| value.eq_ignore_ascii_case("offline"))
        }
    }
}

fn is_truthy(value: &str) -> bool {
    matches!(
        value.trim().to_ascii_lowercase().as_str(),
        "1" | "true" | "yes"
    )
}

fn query_pairs(raw: Option<&str>) -> Vec<(String, String)> {
    match raw {
        Some(raw) => url::form_urlencoded::parse(raw.as_bytes())
            .map(|(key, value)| (key.into_owned(), value.into_owned()))
            .collect(),
        None => Vec::new(),
    }
}

fn oauth_error(error: &str, description: &str) -> Response {
    (
        StatusCode::BAD_REQUEST,
        Json(serde_json::json!({
            "error": error,
            "error_description": description,
        })),
    )
        .into_response()
}

/// A rejected refresh token: `401` with a Basic challenge, so a client prompts
/// for credentials again rather than printing a body-parse error, carrying the
/// OAuth2 error body.
fn oauth_challenge_error(error: &str, description: &str) -> Response {
    let mut response = (
        StatusCode::UNAUTHORIZED,
        Json(serde_json::json!({
            "error": error,
            "error_description": description,
        })),
    )
        .into_response();
    let (name, value) = registry::basic_challenge();
    response.headers_mut().insert(name, value);
    response
}

fn unauthorized_with_basic() -> Response {
    let mut response = ApiError::new(
        StatusCode::UNAUTHORIZED,
        "unauthorized",
        "authentication required",
    )
    .into_response();
    let (name, value) = registry::basic_challenge();
    response.headers_mut().insert(name, value);
    response
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn collects_repeated_and_joined_scopes() {
        let params = vec![
            ("service".to_string(), "registry.local".to_string()),
            ("scope".to_string(), "repository:a/app:pull".to_string()),
            (
                "scope".to_string(),
                "repository:b/app:pull registry:catalog:*".to_string(),
            ),
        ];
        assert_eq!(scope_parameters(&params).len(), 2);
    }

    #[test]
    fn parses_form_encoded_bodies() {
        let pairs = query_pairs(Some("service=x&scope=repository:a:pull&client_id=docker"));
        assert_eq!(pairs.len(), 3);
        assert_eq!(pairs[1].1, "repository:a:pull");
    }

    #[test]
    fn detects_offline_requests() {
        assert!(offline_requested(&[(
            "offline_token".to_string(),
            "true".to_string(),
        )]));
        assert!(offline_requested(&[(
            "access_type".to_string(),
            "offline".to_string(),
        )]));
        assert!(!offline_requested(&[(
            "offline_token".to_string(),
            "false".to_string(),
        )]));
        assert!(!offline_requested(&[]));
    }
}
