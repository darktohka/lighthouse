use axum::middleware;
use axum::routing::get;
use axum::{Json, Router};
use serde_json::json;

use crate::logging;
use crate::state::AppState;
use crate::{api, auth, captcha, oci, static_files};

/// Builds the complete application router.
///
/// `/v2/*` and `/api/*` are owned by their modules; the static-file router is
/// the single fallback and answers unmatched `/v2` and `/api` paths with the
/// appropriate error envelope so they never render the SPA.
///
/// Layer order: `resolve_identity` is added last, which makes it the outermost
/// layer, so identity resolution runs before the access log and both `/v2/*`
/// and `/api/*` see the `AuthContext` extension.
pub fn build(state: AppState) -> Router {
    let fallback = static_files::router().with_state(state.clone());
    captcha::install(&state.db, &state.config);

    Router::new()
        .route("/healthz", get(health))
        .route("/api/health", get(health))
        .merge(oci::router())
        .merge(api::router())
        .merge(auth::router())
        .fallback_service(fallback)
        .layer(middleware::from_fn_with_state(
            state.clone(),
            logging::request_log,
        ))
        .layer(middleware::from_fn_with_state(
            state.clone(),
            auth::middleware::resolve_identity,
        ))
        .with_state(state)
}

async fn health() -> Json<serde_json::Value> {
    Json(json!({ "status": "ok", "service": "lighthouse" }))
}
