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
use crate::models::{AppPassword, AppPasswordCategory};
use crate::state::AppState;

#[derive(Debug, Deserialize)]
struct CreateAppPassword {
    name: String,
    #[serde(default)]
    category_id: Option<i64>,
}

#[derive(Debug, Deserialize)]
struct UpdateAppPassword {
    /// Mandatory key: `null` clears the category, a number assigns one. A
    /// request without the key is rejected rather than treated as "no change",
    /// hence the double option (missing vs. explicit `null`).
    #[serde(default, deserialize_with = "deserialize_present_option")]
    category_id: Option<Option<i64>>,
}

/// Distinguishes a missing key (`None`) from an explicit `null`
/// (`Some(None)`), which plain `Option` collapses.
fn deserialize_present_option<'de, D>(deserializer: D) -> Result<Option<Option<i64>>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    Option::<i64>::deserialize(deserializer).map(Some)
}

#[derive(Debug, Deserialize)]
struct CreateAppPasswordCategory {
    name: String,
}

#[derive(Debug, Serialize)]
struct AppPasswordView {
    id: i64,
    name: String,
    category_id: Option<i64>,
    token_prefix: String,
    token_suffix: String,
    created_at: DateTime<Utc>,
    last_used_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Serialize)]
struct AppPasswordCategoryView {
    id: i64,
    name: String,
    password_count: i64,
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
            category_id: account.category_id,
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
        .route(
            "/api/app-passwords/{id}",
            axum::routing::delete(remove).patch(update),
        )
        .route("/api/app-passwords/{id}/token", post(rotate))
        .route(
            "/api/app-password-categories",
            get(list_categories).post(create_category),
        )
        .route(
            "/api/app-password-categories/{id}",
            axum::routing::delete(remove_category),
        )
}

fn actor_user_id(ctx: &crate::state::AuthContext) -> ApiResult<i64> {
    ctx.user_id
        .ok_or_else(|| ApiError::unauthorized("user session required"))
}

async fn account_for_owner(state: &AppState, user_id: i64, id: i64) -> ApiResult<AppPassword> {
    credentials::find_for_user(&state.db, user_id, id)
        .await?
        .ok_or_else(|| ApiError::not_found("app password not found"))
}

const CATEGORY_NAME_MAX_LEN: usize = 40;

async fn category_for_owner(
    state: &AppState,
    user_id: i64,
    id: i64,
) -> ApiResult<AppPasswordCategory> {
    let category = sqlx::query_as::<_, AppPasswordCategory>(
        "SELECT * FROM app_password_categories WHERE id = ? LIMIT 1",
    )
    .bind(id)
    .fetch_optional(&state.db)
    .await?
    .ok_or_else(|| ApiError::not_found("category not found"))?;
    if category.owner_user_id != user_id {
        return Err(ApiError::not_found("category not found"));
    }
    Ok(category)
}

#[derive(sqlx::FromRow)]
struct CategoryRow {
    id: i64,
    name: String,
    password_count: i64,
}

async fn category_views(state: &AppState, user_id: i64) -> ApiResult<Vec<AppPasswordCategoryView>> {
    let rows = sqlx::query_as::<_, CategoryRow>(
        "SELECT c.id AS id, c.name AS name, COUNT(p.id) AS password_count \
         FROM app_password_categories c \
         LEFT JOIN app_passwords p ON p.category_id = c.id AND p.user_id = c.owner_user_id \
         WHERE c.owner_user_id = ? \
         GROUP BY c.id ORDER BY c.name COLLATE NOCASE",
    )
    .bind(user_id)
    .fetch_all(&state.db)
    .await?;
    Ok(rows
        .into_iter()
        .map(|row| AppPasswordCategoryView {
            id: row.id,
            name: row.name,
            password_count: row.password_count,
        })
        .collect())
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
    if let Some(category_id) = request.category_id {
        category_for_owner(&state, user_id, category_id).await?;
    }
    let (account, token) = credentials::create(&state, user_id, &request.name).await?;

    // A partially configured credential would be usable with a token the caller
    // already holds, so a failed assignment deletes the row again.
    if let Err(err) = sqlx::query("UPDATE app_passwords SET category_id = ? WHERE id = ?")
        .bind(request.category_id)
        .bind(account.id)
        .execute(&state.db)
        .await
    {
        rollback_account(&state, account.id).await;
        return Err(err.into());
    }

    let account = sqlx::query_as::<_, AppPassword>("SELECT * FROM app_passwords WHERE id = ?")
        .bind(account.id)
        .fetch_one(&state.db)
        .await?;
    Ok((
        StatusCode::CREATED,
        Json(CreatedAppPassword {
            app_password: AppPasswordView::from(&account),
            token,
        }),
    )
        .into_response())
}

async fn update(
    State(state): State<AppState>,
    Authenticated(ctx): Authenticated,
    Path(id): Path<i64>,
    Json(request): Json<UpdateAppPassword>,
) -> ApiResult<Response> {
    let user_id = actor_user_id(&ctx)?;
    let account = account_for_owner(&state, user_id, id).await?;
    let category_id = request
        .category_id
        .ok_or_else(|| ApiError::bad_request("category_id is required"))?;
    if let Some(category_id) = category_id {
        category_for_owner(&state, user_id, category_id).await?;
    }
    sqlx::query("UPDATE app_passwords SET category_id = ? WHERE id = ?")
        .bind(category_id)
        .bind(account.id)
        .execute(&state.db)
        .await?;
    let account =
        sqlx::query_as::<_, AppPassword>("SELECT * FROM app_passwords WHERE id = ? LIMIT 1")
            .bind(id)
            .fetch_one(&state.db)
            .await?;
    Ok(Json(AppPasswordView::from(&account)).into_response())
}

async fn list_categories(
    State(state): State<AppState>,
    Authenticated(ctx): Authenticated,
) -> ApiResult<Response> {
    let user_id = actor_user_id(&ctx)?;
    let views = category_views(&state, user_id).await?;
    Ok(Json(views).into_response())
}

async fn create_category(
    State(state): State<AppState>,
    Authenticated(ctx): Authenticated,
    Json(request): Json<CreateAppPasswordCategory>,
) -> ApiResult<Response> {
    let user_id = actor_user_id(&ctx)?;
    let name = request.name.trim().to_string();
    if name.is_empty() {
        return Err(ApiError::bad_request("name is required"));
    }
    if name.chars().count() > CATEGORY_NAME_MAX_LEN {
        return Err(ApiError::bad_request(format!(
            "name must be at most {CATEGORY_NAME_MAX_LEN} characters"
        )));
    }
    let existing: Option<i64> = sqlx::query_scalar(
        "SELECT 1 FROM app_password_categories \
         WHERE owner_user_id = ? AND name = ? COLLATE NOCASE LIMIT 1",
    )
    .bind(user_id)
    .bind(&name)
    .fetch_optional(&state.db)
    .await?;
    if existing.is_some() {
        return Err(ApiError::conflict("category name is taken"));
    }
    let id = sqlx::query(
        "INSERT INTO app_password_categories (owner_user_id, name, created_at) VALUES (?, ?, ?)",
    )
    .bind(user_id)
    .bind(&name)
    .bind(Utc::now())
    .execute(&state.db)
    .await?
    .last_insert_rowid();
    let view = AppPasswordCategoryView {
        id,
        name,
        password_count: 0,
    };
    Ok((StatusCode::CREATED, Json(view)).into_response())
}

async fn remove_category(
    State(state): State<AppState>,
    Authenticated(ctx): Authenticated,
    Path(id): Path<i64>,
) -> ApiResult<Response> {
    let user_id = actor_user_id(&ctx)?;
    let category = category_for_owner(&state, user_id, id).await?;
    let in_use: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM app_passwords WHERE category_id = ?")
            .bind(category.id)
            .fetch_one(&state.db)
            .await?;
    if in_use > 0 {
        return Err(ApiError::conflict("category is not empty"));
    }
    sqlx::query("DELETE FROM app_password_categories WHERE id = ? AND owner_user_id = ?")
        .bind(category.id)
        .bind(user_id)
        .execute(&state.db)
        .await?;
    Ok(StatusCode::NO_CONTENT.into_response())
}

/// Deletes a freshly created credential so a failed multi-step creation leaves
/// nothing behind.
async fn rollback_account(state: &AppState, account_id: i64) {
    let _ = sqlx::query("DELETE FROM app_passwords WHERE id = ?")
        .bind(account_id)
        .execute(&state.db)
        .await;
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

    async fn create_category(app: &Router, cookie: &str, name: &str) -> Value {
        let response = send(
            app,
            "POST",
            "/api/app-password-categories",
            Some(cookie),
            Some(json!({ "name": name })),
        )
        .await;
        assert_eq!(response.status(), StatusCode::CREATED);
        body_json(response).await
    }

    async fn create_password(app: &Router, cookie: &str, body: Value) -> Value {
        let response = send(app, "POST", "/api/app-passwords", Some(cookie), Some(body)).await;
        assert_eq!(response.status(), StatusCode::CREATED);
        body_json(response).await
    }

    async fn list_categories(app: &Router, cookie: &str) -> Vec<Value> {
        let response = send(
            app,
            "GET",
            "/api/app-password-categories",
            Some(cookie),
            None,
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);
        body_json(response).await.as_array().expect("list").clone()
    }

    #[tokio::test]
    async fn category_create_list_validates_names() {
        let (_dir, state) = test_state().await;
        create_user(&state, "alice").await;
        let app = harness(&state).await;
        let cookie = login(&app, "alice").await;

        let created = create_category(&app, &cookie, "  Laptops  ").await;
        assert_eq!(created["name"], "Laptops");
        assert_eq!(created["password_count"], 0);

        let categories = list_categories(&app, &cookie).await;
        assert_eq!(categories.len(), 1);
        assert_eq!(categories[0]["name"], "Laptops");
        assert_eq!(categories[0]["password_count"], 0);

        let response = send(
            &app,
            "POST",
            "/api/app-password-categories",
            Some(&cookie),
            Some(json!({ "name": "laptops" })),
        )
        .await;
        assert_eq!(response.status(), StatusCode::CONFLICT);

        for name in [String::new(), "   ".to_string(), "x".repeat(41)] {
            let response = send(
                &app,
                "POST",
                "/api/app-password-categories",
                Some(&cookie),
                Some(json!({ "name": name })),
            )
            .await;
            assert_eq!(response.status(), StatusCode::BAD_REQUEST, "name {name:?}");
        }
    }

    #[tokio::test]
    async fn categories_are_listed_case_insensitively_alphabetically() {
        let (_dir, state) = test_state().await;
        create_user(&state, "alice").await;
        let app = harness(&state).await;
        let cookie = login(&app, "alice").await;

        create_category(&app, &cookie, "zeta").await;
        create_category(&app, &cookie, "Alpha").await;

        let names: Vec<String> = list_categories(&app, &cookie)
            .await
            .iter()
            .map(|category| category["name"].as_str().expect("name").to_string())
            .collect();
        assert_eq!(names, vec!["Alpha".to_string(), "zeta".to_string()]);
    }

    #[tokio::test]
    async fn passwords_carry_and_clear_categories() {
        let (_dir, state) = test_state().await;
        create_user(&state, "alice").await;
        let app = harness(&state).await;
        let cookie = login(&app, "alice").await;

        let laptops = create_category(&app, &cookie, "Laptops").await;
        let category_id = laptops["id"].as_i64().expect("id");

        let created = create_password(
            &app,
            &cookie,
            json!({ "name": "laptop", "category_id": category_id }),
        )
        .await;
        assert_eq!(created["app_password"]["category_id"], category_id);
        let id = created["app_password"]["id"].as_i64().expect("id");

        let categories = list_categories(&app, &cookie).await;
        assert_eq!(categories[0]["password_count"], 1);

        let response = send(&app, "GET", "/api/app-passwords", Some(&cookie), None).await;
        assert_eq!(response.status(), StatusCode::OK);
        let listed = body_json(response).await;
        assert_eq!(listed[0]["category_id"], category_id);

        let response = send(
            &app,
            "PATCH",
            &format!("/api/app-passwords/{id}"),
            Some(&cookie),
            Some(json!({ "category_id": null })),
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(body_json(response).await["category_id"], Value::Null);

        assert_eq!(list_categories(&app, &cookie).await[0]["password_count"], 0);
    }

    #[tokio::test]
    async fn category_updates_require_the_key_and_ownership() {
        let (_dir, state) = test_state().await;
        create_user(&state, "alice").await;
        create_user(&state, "bob").await;
        let app = harness(&state).await;
        let alice = login(&app, "alice").await;
        let bob = login(&app, "bob").await;

        let created = create_password(&app, &alice, json!({ "name": "laptop" })).await;
        assert_eq!(created["app_password"]["category_id"], Value::Null);
        let id = created["app_password"]["id"].as_i64().expect("id");

        let response = send(
            &app,
            "PATCH",
            &format!("/api/app-passwords/{id}"),
            Some(&alice),
            Some(json!({})),
        )
        .await;
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);

        let foreign = create_category(&app, &bob, "Bob only").await;
        let foreign_id = foreign["id"].as_i64().expect("id");
        let response = send(
            &app,
            "PATCH",
            &format!("/api/app-passwords/{id}"),
            Some(&alice),
            Some(json!({ "category_id": foreign_id })),
        )
        .await;
        assert_eq!(response.status(), StatusCode::NOT_FOUND);

        let response = send(
            &app,
            "POST",
            "/api/app-passwords",
            Some(&alice),
            Some(json!({ "name": "other", "category_id": foreign_id })),
        )
        .await;
        assert_eq!(response.status(), StatusCode::NOT_FOUND);

        let response = send(
            &app,
            "PATCH",
            &format!("/api/app-passwords/{id}"),
            Some(&alice),
            Some(json!({ "category_id": 9999 })),
        )
        .await;
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn only_empty_owned_categories_can_be_deleted() {
        let (_dir, state) = test_state().await;
        create_user(&state, "alice").await;
        create_user(&state, "bob").await;
        let app = harness(&state).await;
        let alice = login(&app, "alice").await;
        let bob = login(&app, "bob").await;

        let laptops = create_category(&app, &alice, "Laptops").await;
        let laptops_id = laptops["id"].as_i64().expect("id");
        let servers = create_category(&app, &alice, "Servers").await;
        let servers_id = servers["id"].as_i64().expect("id");

        let created = create_password(
            &app,
            &alice,
            json!({ "name": "laptop", "category_id": laptops_id }),
        )
        .await;
        let id = created["app_password"]["id"].as_i64().expect("id");

        let response = send(
            &app,
            "DELETE",
            &format!("/api/app-password-categories/{laptops_id}"),
            Some(&alice),
            None,
        )
        .await;
        assert_eq!(response.status(), StatusCode::CONFLICT);
        assert_eq!(
            body_json(response).await["error"]["message"],
            "category is not empty"
        );

        let response = send(
            &app,
            "DELETE",
            &format!("/api/app-password-categories/{laptops_id}"),
            Some(&bob),
            None,
        )
        .await;
        assert_eq!(response.status(), StatusCode::NOT_FOUND);

        let response = send(
            &app,
            "PATCH",
            &format!("/api/app-passwords/{id}"),
            Some(&alice),
            Some(json!({ "category_id": servers_id })),
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);

        let response = send(
            &app,
            "DELETE",
            &format!("/api/app-password-categories/{laptops_id}"),
            Some(&alice),
            None,
        )
        .await;
        assert_eq!(response.status(), StatusCode::NO_CONTENT);

        let response = send(
            &app,
            "DELETE",
            &format!("/api/app-password-categories/{laptops_id}"),
            Some(&alice),
            None,
        )
        .await;
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }
}
