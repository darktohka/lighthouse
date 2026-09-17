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
//! Only Basic is honoured here. A cookie is deliberately ignored so a browser
//! session cannot be turned into a bearer token (a CSRF mint surface), and the
//! response never carries `identity_token`, which would make the CLI store a
//! token instead of the credentials needed to refresh it.

use axum::Router;
use axum::body::Bytes;
use axum::extract::{RawQuery, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::Json;
use chrono::Utc;

use crate::auth::middleware::Auth;
use crate::auth::registry;
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
    headers: HeaderMap,
    RawQuery(raw): RawQuery,
    body: Bytes,
) -> Response {
    let mut params = query_pairs(raw.as_deref());
    if !body.is_empty() {
        params.extend(query_pairs(std::str::from_utf8(&body).ok()));
    }
    issue(&state, &auth.0, &headers, &params).await
}

async fn issue(
    state: &AppState,
    actor: &AuthContext,
    headers: &HeaderMap,
    params: &[(String, String)],
) -> Response {
    let has_basic = headers
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value.starts_with("Basic ") || value.starts_with("basic "));

    let authenticated = has_basic
        && actor.is_authenticated()
        && actor.credential != CredentialSource::RegistryToken;

    if has_basic && !authenticated {
        return unauthorized_with_basic();
    }

    let scopes = registry::parse_scopes(&scope_parameters(params));
    let mut access: Vec<RegistryAccess> = Vec::new();
    if authenticated {
        for scope in &scopes {
            let actions = registry::granted_actions(state, actor, scope).await;
            if !actions.is_empty() {
                access.push(RegistryAccess {
                    kind: scope.kind.clone(),
                    name: scope.name.clone(),
                    actions,
                });
            }
        }
    }

    let sub = if authenticated {
        actor.user_id.unwrap_or(0)
    } else {
        0
    };
    let token = match tokens::issue_registry_token(
        &state.config,
        sub,
        actor.username.as_deref(),
        actor.service_account_id,
        access,
    ) {
        Ok(token) => token,
        Err(err) => {
            tracing::error!(error = %err, "failed to issue registry token");
            return ApiError::internal("failed to issue registry token").into_response();
        }
    };

    Json(serde_json::json!({
        "token": token,
        "access_token": token,
        "expires_in": state.config.registry_token_ttl_secs,
        "issued_at": Utc::now().to_rfc3339(),
    }))
    .into_response()
}

fn scope_parameters(params: &[(String, String)]) -> Vec<String> {
    params
        .iter()
        .filter(|(key, _)| key == "scope")
        .map(|(_, value)| value.clone())
        .collect()
}

fn query_pairs(raw: Option<&str>) -> Vec<(String, String)> {
    match raw {
        Some(raw) => url::form_urlencoded::parse(raw.as_bytes())
            .map(|(key, value)| (key.into_owned(), value.into_owned()))
            .collect(),
        None => Vec::new(),
    }
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
}
