//! Disk usage, reachability and pull statistics.

use std::collections::{HashMap, HashSet};

use axum::Json;
use axum::Router;
use axum::extract::{Query, State};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};

use crate::auth::middleware::Auth;
use crate::error::{ApiError, ApiResult};
use crate::models::Repository;
use crate::permissions;
use crate::state::AppState;

use super::{
    TagBlobRow, TagSizeEntry, load_namespace, namespace_visible, repository_size_map, tag_size_map,
};

#[derive(Debug, Deserialize)]
struct OverviewQuery {
    #[serde(default)]
    namespace: Option<String>,
}

#[derive(Debug, Serialize)]
struct DiskUsageEntry {
    repository: String,
    size: i64,
    unique_size: i64,
}

#[derive(Debug, Serialize)]
struct PullsOverTime {
    date: String,
    pulls: i64,
}

#[derive(Debug, Serialize)]
struct TopRepository {
    repository: String,
    pulls: i64,
}

pub fn router() -> Router<AppState> {
    Router::new().route("/api/analytics/overview", get(overview))
}

#[derive(sqlx::FromRow)]
struct TagJoinRow {
    tag_id: i64,
    tag_name: String,
    tag_updated_at: DateTime<Utc>,
    repository_id: i64,
    repository_name: String,
    namespace_name: String,
    manifest_id: i64,
    digest: String,
    media_type: String,
    manifest_size: i64,
}

async fn overview(
    State(state): State<AppState>,
    auth: Auth,
    Query(query): Query<OverviewQuery>,
) -> ApiResult<Response> {
    let actor = auth.0;

    let namespace_filter = match query.namespace.as_deref() {
        Some(raw) if !raw.trim().is_empty() => {
            let namespace = load_namespace(&state.db, raw.trim())
                .await?
                .ok_or_else(|| ApiError::not_found("namespace not found"))?;
            if !namespace_visible(&state, &actor, &namespace).await? {
                return Err(ApiError::not_found("namespace not found"));
            }
            Some(namespace.id)
        }
        _ => None,
    };

    let all_repositories =
        sqlx::query_as::<_, Repository>("SELECT * FROM repositories ORDER BY name")
            .fetch_all(&state.db)
            .await?;

    let mut scoped: Vec<Repository> = Vec::new();
    for repository in &all_repositories {
        if let Some(namespace_id) = namespace_filter {
            if repository.namespace_id != namespace_id {
                continue;
            }
        }
        if permissions::repository_visible(&state, &actor, repository).await? {
            scoped.push(repository.clone());
        }
    }
    let scope_ids: HashSet<i64> = scoped.iter().map(|repository| repository.id).collect();
    let name_by_id: HashMap<i64, String> = scoped
        .iter()
        .map(|repository| (repository.id, repository.name.clone()))
        .collect();

    let all_rows = super::load_tag_blob_rows(&state).await?;
    let rows: Vec<TagBlobRow> = all_rows
        .into_iter()
        .filter(|row| scope_ids.contains(&row.repository_id))
        .collect();

    let tag_sizes = tag_size_map(&rows);
    let repository_sizes = repository_size_map(&rows);

    let total_size: i64 = repository_sizes.values().map(|(total, _)| *total).sum();
    let unique_size: i64 = repository_sizes.values().map(|(_, unique)| *unique).sum();
    let shared_size = total_size - unique_size;
    let shared_percentage = if total_size == 0 {
        0.0
    } else {
        shared_size as f64 / total_size as f64 * 100.0
    };

    let blob_ids: HashSet<i64> = rows.iter().map(|row| row.blob_id).collect();
    let blob_count = blob_ids.len() as i64;

    let tag_counts = sqlx::query_as::<_, (i64, i64)>(
        "SELECT repository_id, COUNT(*) FROM tags GROUP BY repository_id",
    )
    .fetch_all(&state.db)
    .await?;
    let tag_count: i64 = tag_counts
        .iter()
        .filter(|(repository_id, _)| scope_ids.contains(repository_id))
        .map(|(_, count)| *count)
        .sum();

    let mut manifest_ids: HashSet<i64> = HashSet::new();
    for repository in &scoped {
        let ids = sqlx::query_scalar::<_, i64>(
            "WITH RECURSIVE reach(manifest_id) AS ( \
                 SELECT t.manifest_id FROM tags t WHERE t.repository_id = ? \
                 UNION \
                 SELECT mc.child_manifest_id FROM reach r \
                 JOIN manifest_children mc ON mc.parent_manifest_id = r.manifest_id \
             ) SELECT DISTINCT manifest_id FROM reach",
        )
        .bind(repository.id)
        .fetch_all(&state.db)
        .await?;
        manifest_ids.extend(ids);
    }

    let pull_rows =
        sqlx::query_as::<_, (i64, String, i64)>("SELECT repository_id, day, pulls FROM pull_stats")
            .fetch_all(&state.db)
            .await?;
    let pull_count: i64 = pull_rows
        .iter()
        .filter(|(repository_id, _, _)| scope_ids.contains(repository_id))
        .map(|(_, _, pulls)| *pulls)
        .sum();

    let cutoff = (Utc::now() - Duration::days(30))
        .format("%Y-%m-%d")
        .to_string();
    let mut by_day: HashMap<String, i64> = HashMap::new();
    let mut by_repo: HashMap<i64, i64> = HashMap::new();
    let mut pull_count_30d = 0i64;
    for (repository_id, day, pulls) in &pull_rows {
        if !scope_ids.contains(repository_id) {
            continue;
        }
        *by_repo.entry(*repository_id).or_insert(0) += pulls;
        if day.as_str() >= cutoff.as_str() {
            pull_count_30d += pulls;
            *by_day.entry(day.clone()).or_insert(0) += pulls;
        }
    }

    let mut pulls_over_time: Vec<PullsOverTime> = by_day
        .into_iter()
        .map(|(date, pulls)| PullsOverTime { date, pulls })
        .collect();
    pulls_over_time.sort_by(|a, b| a.date.cmp(&b.date));

    let mut top_repositories: Vec<TopRepository> = by_repo
        .into_iter()
        .filter_map(|(repository_id, pulls)| {
            name_by_id.get(&repository_id).map(|name| TopRepository {
                repository: name.clone(),
                pulls,
            })
        })
        .collect();
    top_repositories.sort_by(|a, b| b.pulls.cmp(&a.pulls).then(a.repository.cmp(&b.repository)));
    top_repositories.truncate(10);

    let mut disk_usage_by_repository: Vec<DiskUsageEntry> = scoped
        .iter()
        .map(|repository| {
            let (size, unique) = repository_sizes
                .get(&repository.id)
                .copied()
                .unwrap_or((0, 0));
            DiskUsageEntry {
                repository: repository.name.clone(),
                size,
                unique_size: unique,
            }
        })
        .collect();
    disk_usage_by_repository
        .sort_by(|a, b| b.size.cmp(&a.size).then(a.repository.cmp(&b.repository)));

    let tag_join = sqlx::query_as::<_, TagJoinRow>(
        "SELECT t.id AS tag_id, t.name AS tag_name, t.updated_at AS tag_updated_at, \
                r.id AS repository_id, r.name AS repository_name, n.name AS namespace_name, \
                m.id AS manifest_id, m.digest AS digest, m.media_type AS media_type, \
                m.size AS manifest_size \
         FROM tags t \
         JOIN repositories r ON r.id = t.repository_id \
         JOIN namespaces n ON n.id = r.namespace_id \
         JOIN manifests m ON m.id = t.manifest_id",
    )
    .fetch_all(&state.db)
    .await?;

    struct LargestTag {
        row: TagJoinRow,
        total: i64,
        unique: i64,
    }
    let mut largest: Vec<LargestTag> = tag_join
        .into_iter()
        .filter(|row| scope_ids.contains(&row.repository_id))
        .map(|row| {
            let (total, unique) = tag_sizes.get(&row.tag_id).copied().unwrap_or((0, 0));
            LargestTag { row, total, unique }
        })
        .collect();
    largest.sort_by(|a, b| {
        b.total
            .cmp(&a.total)
            .then(a.row.repository_name.cmp(&b.row.repository_name))
    });
    largest.truncate(10);

    let mut largest_tags = Vec::with_capacity(largest.len());
    for entry in largest {
        let platforms = super::repositories::platforms_for(
            &state,
            entry.row.manifest_id,
            &entry.row.media_type,
            &entry.row.digest,
            entry.row.manifest_size,
        )
        .await?;
        largest_tags.push(TagSizeEntry {
            repository: entry.row.repository_name,
            namespace: entry.row.namespace_name,
            tag: entry.row.tag_name,
            total_size: entry.total,
            unique_size: entry.unique,
            shared_size: entry.total - entry.unique,
            platforms,
            updated_at: entry.row.tag_updated_at,
        });
    }

    let body = serde_json::json!({
        "total_size": total_size,
        "unique_size": unique_size,
        "shared_size": shared_size,
        "shared_percentage": shared_percentage,
        "blob_count": blob_count,
        "manifest_count": manifest_ids.len() as i64,
        "repository_count": scoped.len() as i64,
        "tag_count": tag_count,
        "pull_count": pull_count,
        "pull_count_30d": pull_count_30d,
        "largest_tags": largest_tags,
        "disk_usage_by_repository": disk_usage_by_repository,
        "pulls_over_time": pulls_over_time,
        "top_repositories": top_repositories,
    });
    Ok(Json(body).into_response())
}
