//! User profile, follow graph, heatmap and search.

use axum::Json;
use axum::Router;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, patch, post};
use chrono::{DateTime, Datelike, Duration, NaiveDate, Utc};
use serde::{Deserialize, Serialize};

use crate::auth::middleware::{Auth, Authenticated};
use crate::error::{ApiError, ApiResult};
use crate::models::User;
use crate::state::{AppState, AuthContext};

use super::{PageQuery, Pagination, UserSummary, load_user};

#[derive(Debug, Serialize)]
struct UserProfile {
    id: i64,
    username: String,
    first_name: Option<String>,
    last_name: Option<String>,
    bio: Option<String>,
    company: Option<String>,
    location: Option<String>,
    website: Option<String>,
    avatar_url: Option<String>,
    created_at: DateTime<Utc>,
    namespace: String,
    repository_count: i64,
    public_repository_count: i64,
    total_pulls: i64,
    follower_count: i64,
    following_count: i64,
    is_following: bool,
    is_self: bool,
}

#[derive(Debug, Deserialize)]
struct UpdateProfile {
    #[serde(default)]
    first_name: Option<String>,
    #[serde(default)]
    last_name: Option<String>,
    #[serde(default)]
    bio: Option<String>,
    #[serde(default)]
    company: Option<String>,
    #[serde(default)]
    location: Option<String>,
    #[serde(default)]
    website: Option<String>,
    #[serde(default)]
    theme: Option<String>,
}

#[derive(Debug, Deserialize)]
struct SearchQuery {
    #[serde(default)]
    q: Option<String>,
    #[serde(default)]
    limit: Option<i64>,
}

#[derive(Debug, Deserialize)]
struct HeatmapQuery {
    #[serde(default)]
    end: Option<String>,
}

#[derive(Debug, Serialize)]
struct HeatmapDay {
    date: String,
    count: i64,
}

#[derive(Debug, Serialize)]
struct Heatmap {
    start: String,
    end: String,
    days: Vec<HeatmapDay>,
    total: i64,
}

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/users/search", get(search))
        .route("/api/users/me", patch(update_me))
        .route("/api/users/{username}", get(profile))
        .route("/api/users/{username}/heatmap", get(heatmap))
        .route("/api/users/{username}/followers", get(followers))
        .route("/api/users/{username}/following", get(following))
        .route(
            "/api/users/{username}/follow",
            post(follow).delete(unfollow),
        )
}

fn normalize(value: Option<String>) -> Option<String> {
    value.and_then(|value| {
        let trimmed = value.trim().to_string();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed)
        }
    })
}

async fn find_user_by_username(state: &AppState, username: &str) -> ApiResult<User> {
    sqlx::query_as::<_, User>("SELECT * FROM users WHERE username = ? COLLATE NOCASE LIMIT 1")
        .bind(username)
        .fetch_optional(&state.db)
        .await?
        .ok_or_else(|| ApiError::not_found("user not found"))
}

async fn build_profile(
    state: &AppState,
    actor: &AuthContext,
    user: &User,
) -> ApiResult<UserProfile> {
    let namespace: Option<String> = sqlx::query_scalar(
        "SELECT name FROM namespaces WHERE owner_user_id = ? AND kind = 'user' LIMIT 1",
    )
    .bind(user.id)
    .fetch_optional(&state.db)
    .await?;
    let namespace = namespace.unwrap_or_else(|| user.username.clone());

    let is_self = actor.user_id == Some(user.id);

    let repository_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM repositories r \
         JOIN namespaces n ON n.id = r.namespace_id \
         WHERE n.owner_user_id = ? AND n.kind = 'user'",
    )
    .bind(user.id)
    .fetch_one(&state.db)
    .await?;

    let public_repository_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM repositories r \
         JOIN namespaces n ON n.id = r.namespace_id \
         WHERE n.owner_user_id = ? AND n.kind = 'user' AND n.is_public = 1 AND r.is_public = 1",
    )
    .bind(user.id)
    .fetch_one(&state.db)
    .await?;

    let repository_count = if is_self {
        repository_count
    } else {
        public_repository_count
    };

    let total_pulls: i64 = if is_self {
        sqlx::query_scalar(
            "SELECT COALESCE(SUM(ps.pulls), 0) FROM pull_stats ps \
             JOIN repositories r ON r.id = ps.repository_id \
             JOIN namespaces n ON n.id = r.namespace_id \
             WHERE n.owner_user_id = ? AND n.kind = 'user'",
        )
        .bind(user.id)
        .fetch_one(&state.db)
        .await?
    } else {
        sqlx::query_scalar(
            "SELECT COALESCE(SUM(ps.pulls), 0) FROM pull_stats ps \
             JOIN repositories r ON r.id = ps.repository_id \
             JOIN namespaces n ON n.id = r.namespace_id \
             WHERE n.owner_user_id = ? AND n.kind = 'user' \
               AND n.is_public = 1 AND r.is_public = 1",
        )
        .bind(user.id)
        .fetch_one(&state.db)
        .await?
    };

    let follower_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM follows WHERE followed_user_id = ?")
            .bind(user.id)
            .fetch_one(&state.db)
            .await?;
    let following_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM follows WHERE follower_user_id = ?")
            .bind(user.id)
            .fetch_one(&state.db)
            .await?;

    let is_following = match actor.user_id {
        Some(actor_id) => sqlx::query_scalar::<_, i64>(
            "SELECT 1 FROM follows WHERE follower_user_id = ? AND followed_user_id = ? LIMIT 1",
        )
        .bind(actor_id)
        .bind(user.id)
        .fetch_optional(&state.db)
        .await?
        .is_some(),
        None => false,
    };

    Ok(UserProfile {
        id: user.id,
        username: user.username.clone(),
        first_name: user.first_name.clone(),
        last_name: user.last_name.clone(),
        bio: user.bio.clone(),
        company: user.company.clone(),
        location: user.location.clone(),
        website: user.website.clone(),
        avatar_url: user.avatar_url.clone(),
        created_at: user.created_at,
        namespace,
        repository_count,
        public_repository_count,
        total_pulls,
        follower_count,
        following_count,
        is_following,
        is_self,
    })
}

async fn profile(
    State(state): State<AppState>,
    auth: Auth,
    Path(username): Path<String>,
) -> ApiResult<Response> {
    let user = find_user_by_username(&state, &username).await?;
    let profile = build_profile(&state, &auth.0, &user).await?;
    Ok(Json(profile).into_response())
}

async fn update_me(
    State(state): State<AppState>,
    Authenticated(ctx): Authenticated,
    Json(request): Json<UpdateProfile>,
) -> ApiResult<Response> {
    let user_id = ctx
        .user_id
        .ok_or_else(|| ApiError::unauthorized("user session required"))?;

    sqlx::query(
        "UPDATE users SET \
             first_name = COALESCE(?, first_name), \
             last_name = COALESCE(?, last_name), \
             bio = COALESCE(?, bio), \
             company = COALESCE(?, company), \
             location = COALESCE(?, location), \
             website = COALESCE(?, website), \
             theme = COALESCE(?, theme), \
             updated_at = ? \
         WHERE id = ?",
    )
    .bind(normalize(request.first_name))
    .bind(normalize(request.last_name))
    .bind(normalize(request.bio))
    .bind(normalize(request.company))
    .bind(normalize(request.location))
    .bind(normalize(request.website))
    .bind(normalize(request.theme))
    .bind(Utc::now())
    .bind(user_id)
    .execute(&state.db)
    .await?;

    let user = load_user(&state.db, user_id).await?;
    let profile = build_profile(&state, &ctx, &user).await?;
    Ok(Json(profile).into_response())
}

async fn search(
    State(state): State<AppState>,
    Query(query): Query<SearchQuery>,
) -> ApiResult<Response> {
    let term = query.q.unwrap_or_default().trim().to_string();
    let limit = query.limit.unwrap_or(10).clamp(1, 10);
    if term.is_empty() {
        return Ok(Json(Vec::<UserSummary>::new()).into_response());
    }
    let pattern = format!("%{}%", escape_like(&term));
    let users = sqlx::query_as::<_, User>(
        "SELECT * FROM users \
         WHERE username LIKE ? ESCAPE '\\' \
            OR first_name LIKE ? ESCAPE '\\' \
            OR last_name LIKE ? ESCAPE '\\' \
         ORDER BY username COLLATE NOCASE LIMIT ?",
    )
    .bind(&pattern)
    .bind(&pattern)
    .bind(&pattern)
    .bind(limit)
    .fetch_all(&state.db)
    .await?;

    let summaries: Vec<UserSummary> = users.iter().map(UserSummary::from).collect();
    Ok(Json(summaries).into_response())
}

fn escape_like(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace('%', "\\%")
        .replace('_', "\\_")
}

async fn heatmap(
    State(state): State<AppState>,
    Path(username): Path<String>,
    Query(query): Query<HeatmapQuery>,
) -> ApiResult<Response> {
    let user = find_user_by_username(&state, &username).await?;

    let end = match &query.end {
        Some(raw) => NaiveDate::parse_from_str(raw, "%Y-%m-%d")
            .map_err(|_| ApiError::bad_request("invalid end date"))?,
        None => Utc::now().date_naive(),
    };

    // Snap `end` forward to the Saturday of its week (0 = Sun .. 6 = Sat).
    let offset = 6 - end.weekday().num_days_from_sunday();
    let window_end = end + Duration::days(offset as i64);
    let window_start = window_end - Duration::days(363);

    let start_str = window_start.format("%Y-%m-%d").to_string();
    let end_str = window_end.format("%Y-%m-%d").to_string();

    let mut counts: std::collections::HashMap<String, i64> = std::collections::HashMap::new();

    let activity_days = sqlx::query_as::<_, (String, i64)>(
        "SELECT substr(created_at, 1, 10) AS day, COUNT(*) \
         FROM activity WHERE actor_user_id = ? \
         AND substr(created_at, 1, 10) BETWEEN ? AND ? \
         GROUP BY day",
    )
    .bind(user.id)
    .bind(&start_str)
    .bind(&end_str)
    .fetch_all(&state.db)
    .await?;
    for (day, count) in activity_days {
        *counts.entry(day).or_insert(0) += count;
    }

    let pull_days = sqlx::query_as::<_, (String, i64)>(
        "SELECT substr(created_at, 1, 10) AS day, COUNT(*) \
         FROM pull_events WHERE user_id = ? \
         AND substr(created_at, 1, 10) BETWEEN ? AND ? \
         GROUP BY day",
    )
    .bind(user.id)
    .bind(&start_str)
    .bind(&end_str)
    .fetch_all(&state.db)
    .await?;
    for (day, count) in pull_days {
        *counts.entry(day).or_insert(0) += count;
    }

    let mut days = Vec::new();
    let mut total = 0i64;
    let mut cursor = window_start;
    loop {
        let date = cursor.format("%Y-%m-%d").to_string();
        let count = counts.get(&date).copied().unwrap_or(0);
        total += count;
        days.push(HeatmapDay { date, count });
        if cursor >= window_end {
            break;
        }
        cursor = cursor.succ_opt().unwrap_or(window_end);
    }

    Ok(Json(Heatmap {
        start: start_str,
        end: end_str,
        days,
        total,
    })
    .into_response())
}

async fn followers(
    State(state): State<AppState>,
    Path(username): Path<String>,
    Query(query): Query<PageQuery>,
) -> ApiResult<Response> {
    let user = find_user_by_username(&state, &username).await?;
    let pagination = Pagination::from_query(&query);

    let total: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM follows WHERE followed_user_id = ?")
        .bind(user.id)
        .fetch_one(&state.db)
        .await?;

    let rows = sqlx::query_as::<_, User>(
        "SELECT u.* FROM follows f JOIN users u ON u.id = f.follower_user_id \
         WHERE f.followed_user_id = ? \
         ORDER BY f.created_at DESC LIMIT ? OFFSET ?",
    )
    .bind(user.id)
    .bind(pagination.per_page)
    .bind(pagination.offset() as i64)
    .fetch_all(&state.db)
    .await?;

    let items: Vec<UserSummary> = rows.iter().map(UserSummary::from).collect();
    Ok(Json(pagination.envelope(items, total)).into_response())
}

async fn following(
    State(state): State<AppState>,
    Path(username): Path<String>,
    Query(query): Query<PageQuery>,
) -> ApiResult<Response> {
    let user = find_user_by_username(&state, &username).await?;
    let pagination = Pagination::from_query(&query);

    let total: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM follows WHERE follower_user_id = ?")
        .bind(user.id)
        .fetch_one(&state.db)
        .await?;

    let rows = sqlx::query_as::<_, User>(
        "SELECT u.* FROM follows f JOIN users u ON u.id = f.followed_user_id \
         WHERE f.follower_user_id = ? \
         ORDER BY f.created_at DESC LIMIT ? OFFSET ?",
    )
    .bind(user.id)
    .bind(pagination.per_page)
    .bind(pagination.offset() as i64)
    .fetch_all(&state.db)
    .await?;

    let items: Vec<UserSummary> = rows.iter().map(UserSummary::from).collect();
    Ok(Json(pagination.envelope(items, total)).into_response())
}

async fn follow(
    State(state): State<AppState>,
    Authenticated(ctx): Authenticated,
    Path(username): Path<String>,
) -> ApiResult<Response> {
    let actor_id = ctx
        .user_id
        .ok_or_else(|| ApiError::unauthorized("user session required"))?;
    let target = find_user_by_username(&state, &username).await?;
    if target.id == actor_id {
        return Err(ApiError::bad_request("cannot follow yourself"));
    }
    sqlx::query(
        "INSERT OR IGNORE INTO follows (follower_user_id, followed_user_id, created_at) \
         VALUES (?, ?, ?)",
    )
    .bind(actor_id)
    .bind(target.id)
    .bind(Utc::now())
    .execute(&state.db)
    .await?;
    Ok(StatusCode::NO_CONTENT.into_response())
}

async fn unfollow(
    State(state): State<AppState>,
    Authenticated(ctx): Authenticated,
    Path(username): Path<String>,
) -> ApiResult<Response> {
    let actor_id = ctx
        .user_id
        .ok_or_else(|| ApiError::unauthorized("user session required"))?;
    let target = find_user_by_username(&state, &username).await?;
    sqlx::query("DELETE FROM follows WHERE follower_user_id = ? AND followed_user_id = ?")
        .bind(actor_id)
        .bind(target.id)
        .execute(&state.db)
        .await?;
    Ok(StatusCode::NO_CONTENT.into_response())
}
