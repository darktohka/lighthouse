//! Tag listing (`/v2/<name>/tags/list`).

use axum::http::{HeaderMap, StatusCode};
use axum::response::Response;

use crate::error::RegistryError;
use crate::permissions::Action;
use crate::state::{AppState, AuthContext};

use super::EventInfo;

pub async fn list(
    state: &AppState,
    actor: &AuthContext,
    _headers: &HeaderMap,
    _info: &EventInfo,
    name: &str,
    query: &[(String, String)],
) -> Result<Response, RegistryError> {
    super::validate_name(name)?;
    let repository = super::find_repo(state, actor, name, Action::Pull).await?;
    let page = super::parse_page_size(query)?;
    let last = super::query_first(query, "last");

    let mut tags = state
        .registry
        .list_tags(repository.id, page + 1, last)
        .await?;

    let mut link = None;
    if tags.len() > page {
        tags.truncate(page);
        if let Some(last_tag) = tags.last() {
            link = Some(format!(
                "</v2/{name}/tags/list?n={page}&last={last_tag}>; rel=\"next\""
            ));
        }
    }

    let body = serde_json::json!({ "name": name, "tags": tags });
    let mut response = super::json_response(StatusCode::OK, body);
    if let Some(link) = link {
        response = super::set_header(response, "link", link);
    }
    Ok(response)
}
