//! Registry catalog (`/v2/_catalog`).

use axum::extract::{RawQuery, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::Response;

use crate::auth::middleware::Auth;
use crate::error::RegistryError;
use crate::state::{AppState, AuthContext};

pub async fn get(
    State(state): State<AppState>,
    auth: Auth,
    _headers: HeaderMap,
    RawQuery(raw): RawQuery,
) -> Response {
    match list(&state, &auth.0, raw.as_deref()).await {
        Ok(response) => response,
        Err(err) => super::error_response(&state.config, err, Some("registry:catalog:*")),
    }
}

async fn list(
    state: &AppState,
    actor: &AuthContext,
    raw: Option<&str>,
) -> Result<Response, RegistryError> {
    if !actor.is_authenticated() {
        return if actor.is_registry_token() {
            Err(RegistryError::denied(
                "requested access to the resource is denied",
            ))
        } else {
            Err(RegistryError::unauthorized("authentication required"))
        };
    }
    let query = super::query_pairs(raw);
    let page = super::parse_page_size(&query)?;
    let last = super::query_first(&query, "last");

    let mut repositories = state
        .registry
        .list_repository_names(page + 1, last)
        .await?;

    let mut link = None;
    if repositories.len() > page {
        repositories.truncate(page);
        if let Some(last_repo) = repositories.last() {
            link = Some(format!(
                "</v2/_catalog?n={page}&last={last_repo}>; rel=\"next\""
            ));
        }
    }

    let body = serde_json::json!({ "repositories": repositories });
    let mut response = super::json_response(StatusCode::OK, body);
    if let Some(link) = link {
        response = super::set_header(response, "link", link);
    }
    Ok(response)
}
