//! App-password management API (control-plane surface).
//!
//! Creation and rotation return the plaintext secret exactly once; listings only
//! ever expose the display prefix/suffix.

use axum::Json;
use axum::Router;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::auth::app_passwords as credentials;
use crate::auth::middleware::Authenticated;
use crate::error::{ApiError, ApiResult};
use crate::models::AppPassword;
use crate::state::AppState;

#[derive(Debug, Deserialize)]
struct CreateAppPassword {
    name: String,
}

#[derive(Debug, Serialize)]
struct AppPasswordView {
    id: i64,
    name: String,
    token_prefix: String,
    token_suffix: String,
    created_at: DateTime<Utc>,
    last_used_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Serialize)]
struct CreatedAppPassword {
    app_password: AppPasswordView,
    token: String,
}

impl From<&AppPassword> for AppPasswordView {
    fn from(account: &AppPassword) -> Self {
        Self {
            id: account.id,
            name: account.name.clone(),
            token_prefix: account.token_prefix.clone(),
            token_suffix: account.token_suffix.clone(),
            created_at: account.created_at,
            last_used_at: account.last_used_at,
        }
    }
}

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/app-passwords", get(list).post(create))
        .route("/api/app-passwords/{id}", axum::routing::delete(remove))
        .route("/api/app-passwords/{id}/token", post(rotate))
}

fn actor_user_id(ctx: &crate::state::AuthContext) -> ApiResult<i64> {
    ctx.user_id
        .ok_or_else(|| ApiError::unauthorized("user session required"))
}

async fn list(
    State(state): State<AppState>,
    Authenticated(ctx): Authenticated,
) -> ApiResult<Response> {
    let user_id = actor_user_id(&ctx)?;
    let accounts = credentials::list(&state.db, user_id).await?;
    let views: Vec<AppPasswordView> = accounts.iter().map(AppPasswordView::from).collect();
    Ok(Json(views).into_response())
}

async fn create(
    State(state): State<AppState>,
    Authenticated(ctx): Authenticated,
    Json(request): Json<CreateAppPassword>,
) -> ApiResult<Response> {
    let user_id = actor_user_id(&ctx)?;
    let (account, token) = credentials::create(&state, user_id, &request.name).await?;
    Ok((
        StatusCode::CREATED,
        Json(CreatedAppPassword {
            app_password: AppPasswordView::from(&account),
            token,
        }),
    )
        .into_response())
}

async fn rotate(
    State(state): State<AppState>,
    Authenticated(ctx): Authenticated,
    Path(id): Path<i64>,
) -> ApiResult<Response> {
    let user_id = actor_user_id(&ctx)?;
    let (account, token) = credentials::rotate(&state, user_id, id)
        .await?
        .ok_or_else(|| ApiError::not_found("app password not found"))?;
    Ok(Json(CreatedAppPassword {
        app_password: AppPasswordView::from(&account),
        token,
    })
    .into_response())
}

async fn remove(
    State(state): State<AppState>,
    Authenticated(ctx): Authenticated,
    Path(id): Path<i64>,
) -> ApiResult<Response> {
    let user_id = actor_user_id(&ctx)?;
    if !credentials::delete(&state, user_id, id).await? {
        return Err(ApiError::not_found("app password not found"));
    }
    Ok(StatusCode::NO_CONTENT.into_response())
}

#[cfg(test)]
mod tests {
    use axum::Router;
    use axum::body::Body;
    use axum::http::{Request as HttpRequest, StatusCode};
    use serde_json::{Value, json};
    use tower::ServiceExt;

    use crate::auth::middleware::resolve_identity;
    use crate::auth::test_support::{body_json, cookie_pair, create_user, test_state};
    use crate::state::AppState;

    async fn login(app: &Router, identifier: &str) -> String {
        let response = app
            .clone()
            .oneshot(
                HttpRequest::builder()
                    .method("POST")
                    .uri("/api/auth/login")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        json!({ "identifier": identifier, "password": "correct-horse-battery" })
                            .to_string(),
                    ))
                    .expect("request"),
            )
            .await
            .expect("response");
        assert_eq!(response.status(), StatusCode::OK);
        cookie_pair(&response, "lighthouse_token").expect("cookie")
    }

    async fn send(
        app: &Router,
        method: &str,
        uri: &str,
        cookie: Option<&str>,
        body: Option<Value>,
    ) -> axum::response::Response {
        let mut builder = HttpRequest::builder().method(method).uri(uri);
        if let Some(cookie) = cookie {
            builder = builder.header("cookie", cookie);
        }
        let body = match body {
            Some(value) => {
                builder = builder.header("content-type", "application/json");
                Body::from(value.to_string())
            }
            None => Body::empty(),
        };
        app.clone()
            .oneshot(builder.body(body).expect("request"))
            .await
            .expect("response")
    }

    async fn harness(state: &AppState) -> Router {
        Router::new()
            .merge(crate::auth::router())
            .merge(super::router())
            .layer(axum::middleware::from_fn_with_state(
                state.clone(),
                resolve_identity,
            ))
            .with_state(state.clone())
    }

    #[tokio::test]
    async fn create_list_rotate_delete_flow() {
        let (_dir, state) = test_state().await;
        create_user(&state, "alice").await;
        let app = harness(&state).await;
        let cookie = login(&app, "alice").await;

        let response = send(
            &app,
            "POST",
            "/api/app-passwords",
            Some(&cookie),
            Some(json!({ "name": "laptop" })),
        )
        .await;
        assert_eq!(response.status(), StatusCode::CREATED);
        let body = body_json(response).await;
        let id = body["app_password"]["id"].as_i64().expect("id");
        let secret = body["token"].as_str().expect("token").to_string();
        assert!(secret.starts_with("lhp_"));
        assert!(body["app_password"].get("token_hash").is_none());

        let response = send(&app, "GET", "/api/app-passwords", Some(&cookie), None).await;
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(body_json(response).await.as_array().expect("list").len(), 1);

        let response = send(
            &app,
            "POST",
            &format!("/api/app-passwords/{id}/token"),
            Some(&cookie),
            None,
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);
        let rotated = body_json(response).await["token"]
            .as_str()
            .expect("token")
            .to_string();
        assert_ne!(rotated, secret);

        let response = send(
            &app,
            "DELETE",
            &format!("/api/app-passwords/{id}"),
            Some(&cookie),
            None,
        )
        .await;
        assert_eq!(response.status(), StatusCode::NO_CONTENT);
    }

    #[tokio::test]
    async fn anonymous_callers_are_rejected() {
        let (_dir, state) = test_state().await;
        let app = harness(&state).await;
        let response = send(&app, "GET", "/api/app-passwords", None, None).await;
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }
}
