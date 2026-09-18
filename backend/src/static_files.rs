//! Serves the built frontend (`frontend/dist`) with single-page-application
//! routing. API and registry prefixes are never served from here.

#![allow(dead_code)]

use std::path::PathBuf;

use axum::Router;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::routing::any;
use tower_http::services::{ServeDir, ServeFile};

use crate::error::ApiError;
use crate::state::AppState;

/// Builds the fallback router that serves static assets and the SPA shell.
///
/// Requests under `/api` and `/v2` are answered with the matching error
/// envelope so that unmatched API paths never render `index.html`.
pub fn router() -> Router<AppState> {
    build(resolve_dist_dir())
}

fn build<S>(dist: Option<PathBuf>) -> Router<S>
where
    S: Clone + Send + Sync + 'static,
{
    let router = Router::new().route("/api/{*rest}", any(api_not_found));

    match dist {
        Some(dir) => {
            let index = dir.join("index.html");
            router.fallback_service(ServeDir::new(dir).fallback(ServeFile::new(index)))
        }
        None => router.fallback(not_built),
    }
}

/// Resolves the directory holding the built frontend.
pub fn resolve_dist_dir() -> Option<PathBuf> {
    resolve_dist_dir_from(std::env::var("FRONTEND_DIR").ok().as_deref())
}

fn resolve_dist_dir_from(configured: Option<&str>) -> Option<PathBuf> {
    if let Some(raw) = configured.filter(|raw| !raw.is_empty()) {
        let path = PathBuf::from(raw);
        if path.is_dir() {
            return Some(path);
        }
        tracing::warn!(path = %path.display(), "FRONTEND_DIR does not exist");
        return None;
    }
    ["/app/dist", "../frontend/dist"]
        .into_iter()
        .map(PathBuf::from)
        .find(|path| path.is_dir())
}

async fn api_not_found() -> ApiError {
    ApiError::not_found("unknown API endpoint")
}

async fn not_built() -> impl IntoResponse {
    (
        StatusCode::NOT_FOUND,
        "frontend assets are not built; set FRONTEND_DIR or run the frontend build",
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::Request;
    use tower::ServiceExt;

    fn temp_dist() -> tempfile::TempDir {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(dir.path().join("index.html"), "<html>lighthouse</html>").expect("index");
        dir
    }

    async fn body_json(response: axum::response::Response) -> serde_json::Value {
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("body");
        serde_json::from_slice(&bytes).expect("json body")
    }

    #[tokio::test]
    async fn v2_paths_are_no_longer_owned_by_static_files() {
        let app = build::<()>(None);
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/v2/darktohka/more/complicated/project2/manifests/latest")
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response");
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn unmatched_api_path_returns_api_envelope() {
        let app = build::<()>(None);
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/does-not-exist")
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response");
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
        let json = body_json(response).await;
        assert_eq!(json["error"]["code"], "not_found");
    }

    #[tokio::test]
    async fn unknown_path_serves_the_spa_shell() {
        let dist = temp_dist();
        let app = build::<()>(Some(dist.path().to_path_buf()));
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/namespaces/darktohka")
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response");
        assert_eq!(response.status(), StatusCode::OK);
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("body");
        let body = String::from_utf8(bytes.to_vec()).expect("utf8");
        assert!(body.contains("lighthouse"));
    }

    #[tokio::test]
    async fn missing_dist_reports_404() {
        let app = build::<()>(None);
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/some/spa/route")
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response");
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }

    #[test]
    fn configured_directory_is_resolved() {
        let dir = tempfile::tempdir().expect("tempdir");
        let resolved = resolve_dist_dir_from(Some(dir.path().to_str().expect("utf8")));
        assert_eq!(resolved, Some(dir.path().to_path_buf()));
    }

    #[test]
    fn missing_configured_directory_resolves_to_none() {
        let dir = tempfile::tempdir().expect("tempdir");
        let missing = dir.path().join("absent");
        assert_eq!(
            resolve_dist_dir_from(Some(missing.to_str().expect("utf8"))),
            None
        );
    }
}
