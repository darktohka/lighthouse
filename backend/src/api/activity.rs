//! Activity timeline feed and dashboard.

use std::collections::{HashMap, HashSet};

use axum::Json;
use axum::Router;
use axum::extract::{Query, State};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use chrono::{Duration, Utc};

use crate::auth::middleware::{Auth, Authenticated};
use crate::error::ApiResult;
use crate::models::{Activity, Namespace};
use crate::state::{AppState, AuthContext};

use super::{
    ActivityEntry, PageQuery, Pagination, RepositorySummary, namespace_visible,
    parse_metadata, user_summaries,
};

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/activity", get(feed))
        .route("/api/activity/me", get(my_activity))
        .route("/api/dashboard", get(dashboard))
}

async fn name_map(state: &AppState, table: &str, ids: &[i64]) -> ApiResult<HashMap<i64, String>> {
    let mut map = HashMap::new();
    if ids.is_empty() {
        return Ok(map);
    }
    let mut unique: Vec<i64> = ids.to_vec();
    unique.sort_unstable();
    unique.dedup();
    let placeholders = vec!["?"; unique.len()].join(",");
    let sql = format!("SELECT id, name FROM {table} WHERE id IN ({placeholders})");
    let mut query = sqlx::query_as::<_, (i64, String)>(sqlx::AssertSqlSafe(sql));
    for id in &unique {
        query = query.bind(*id);
    }
    for (id, name) in query.fetch_all(&state.db).await? {
        map.insert(id, name);
    }
    Ok(map)
}

async fn build_entries(state: &AppState, rows: Vec<Activity>) -> ApiResult<Vec<ActivityEntry>> {
    let actor_ids: Vec<i64> = rows.iter().filter_map(|row| row.actor_user_id).collect();
    let namespace_ids: Vec<i64> = rows.iter().filter_map(|row| row.namespace_id).collect();
    let repository_ids: Vec<i64> = rows.iter().filter_map(|row| row.repository_id).collect();

    let actors = user_summaries(&state.db, &actor_ids).await?;
    let namespaces = name_map(state, "namespaces", &namespace_ids).await?;
    let repositories = name_map(state, "repositories", &repository_ids).await?;

    Ok(rows
        .into_iter()
        .map(|row| {
            let actor = row
                .actor_user_id
                .and_then(|actor_id| actors.get(&actor_id).cloned());
            ActivityEntry {
                id: row.id,
                kind: row.kind,
                summary: row.summary,
                actor,
                namespace: row
                    .namespace_id
                    .and_then(|namespace_id| namespaces.get(&namespace_id).cloned()),
                repository: row
                    .repository_id
                    .and_then(|repository_id| repositories.get(&repository_id).cloned()),
                metadata: parse_metadata(row.metadata.as_deref()),
                created_at: row.created_at,
            }
        })
        .collect())
}

async fn all_activity(state: &AppState) -> ApiResult<Vec<Activity>> {
    let rows = sqlx::query_as::<_, Activity>("SELECT * FROM activity ORDER BY created_at DESC, id DESC")
        .fetch_all(&state.db)
        .await?;
    Ok(rows)
}

async fn feed(
    State(state): State<AppState>,
    auth: Auth,
    Query(query): Query<PageQuery>,
) -> ApiResult<Response> {
    let actor = auth.0;
    let pagination = Pagination::from_query(&query);

    let rows = all_activity(&state).await?;
    let visible_namespaces = visible_namespace_ids(&state, &actor).await?;
    let visible_repositories = super::visible_repository_ids(&state, &actor).await?;

    let filtered: Vec<Activity> = rows
        .into_iter()
        .filter(|row| {
            row.is_public
                || actor.user_id == row.actor_user_id
                || row
                    .repository_id
                    .is_some_and(|id| visible_repositories.contains(&id))
                || row
                    .namespace_id
                    .is_some_and(|id| visible_namespaces.contains(&id))
        })
        .collect();

    let total = filtered.len() as i64;
    let windowed = pagination.window(filtered);
    let items = build_entries(&state, windowed).await?;
    Ok(Json(pagination.envelope(items, total)).into_response())
}

async fn my_activity(
    State(state): State<AppState>,
    Authenticated(ctx): Authenticated,
    Query(query): Query<PageQuery>,
) -> ApiResult<Response> {
    let pagination = Pagination::from_query(&query);
    let user_id = ctx.user_id;

    let rows = sqlx::query_as::<_, Activity>(
        "SELECT * FROM activity WHERE actor_user_id = ? ORDER BY created_at DESC, id DESC",
    )
    .bind(user_id)
    .fetch_all(&state.db)
    .await?;

    let total = rows.len() as i64;
    let windowed = pagination.window(rows);
    let items = build_entries(&state, windowed).await?;
    Ok(Json(pagination.envelope(items, total)).into_response())
}

async fn visible_namespace_ids(
    state: &AppState,
    actor: &AuthContext,
) -> ApiResult<HashSet<i64>> {
    let namespaces = sqlx::query_as::<_, Namespace>("SELECT * FROM namespaces")
        .fetch_all(&state.db)
        .await?;
    let mut visible = HashSet::new();
    for namespace in namespaces {
        if namespace_visible(state, actor, &namespace).await? {
            visible.insert(namespace.id);
        } else if super::is_namespace_owner(actor, &namespace) {
            visible.insert(namespace.id);
        }
    }
    Ok(visible)
}

async fn dashboard(
    State(state): State<AppState>,
    Authenticated(ctx): Authenticated,
) -> ApiResult<Response> {
    let user_id = ctx
        .user_id
        .ok_or_else(|| crate::error::ApiError::unauthorized("user session required"))?;

    let namespaces = sqlx::query_as::<_, Namespace>(
        "SELECT * FROM namespaces \
         WHERE owner_user_id = ? \
            OR id IN (SELECT namespace_id FROM namespace_members WHERE user_id = ?) \
         ORDER BY name COLLATE NOCASE",
    )
    .bind(user_id)
    .bind(user_id)
    .fetch_all(&state.db)
    .await?;

    let mut repositories: Vec<RepositorySummary> = Vec::new();
    for namespace in &namespaces {
        repositories.extend(
            super::repositories::namespace_repository_summaries(&state, &ctx, namespace.id).await?,
        );
    }
    repositories.sort_by(|a, b| b.updated_at.cmp(&a.updated_at).then(a.name.cmp(&b.name)));
    let repository_ids: Vec<i64> = repositories.iter().map(|repository| repository.id).collect();

    let tag_count: i64 = repositories.iter().map(|repository| repository.tag_count).sum();
    let total_size: i64 = repositories.iter().map(|repository| repository.size).sum();

    let mut pull_count_30d = 0i64;
    if !repository_ids.is_empty() {
        let cutoff = (Utc::now() - Duration::days(30))
            .format("%Y-%m-%d")
            .to_string();
        let rows = sqlx::query_as::<_, (i64, i64)>(
            "SELECT repository_id, COALESCE(SUM(pulls), 0) FROM pull_stats \
             WHERE day >= ? GROUP BY repository_id",
        )
        .bind(&cutoff)
        .fetch_all(&state.db)
        .await?;
        for (repository_id, pulls) in rows {
            if repository_ids.contains(&repository_id) {
                pull_count_30d += pulls;
            }
        }
    }

    let activity_rows = sqlx::query_as::<_, Activity>(
        "SELECT * FROM activity WHERE actor_user_id = ? ORDER BY created_at DESC, id DESC LIMIT 20",
    )
    .bind(user_id)
    .fetch_all(&state.db)
    .await?;
    let activity = build_entries(&state, activity_rows).await?;

    let body = serde_json::json!({
        "repositories": repositories,
        "activity": activity,
        "stats": {
            "repository_count": repository_ids.len() as i64,
            "tag_count": tag_count,
            "total_size": total_size,
            "pull_count_30d": pull_count_30d,
        },
    });
    Ok(Json(body).into_response())
}
