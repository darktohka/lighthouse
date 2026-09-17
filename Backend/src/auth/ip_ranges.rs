//! Per-service-account CIDR allowlists.
//!
//! A service account with no configured ranges is unrestricted. As soon as one
//! range exists, the account may only authenticate from an address inside it and
//! every failure path denies (fail closed), so a database error or a missing
//! account can never silently widen access.

use chrono::Utc;
use ipnet::IpNet;

use crate::db::{self, Db};
use crate::error::{ApiError, ApiResult};
use crate::models::ServiceAccountIpRange;
use crate::net;

/// Upper bound on the number of ranges per account.
pub const MAX_RANGES: i64 = 64;

/// Lists an account's ranges in insertion order.
pub async fn list(db: &Db, account_id: i64) -> ApiResult<Vec<ServiceAccountIpRange>> {
    let rows = sqlx::query_as::<_, ServiceAccountIpRange>(
        "SELECT * FROM service_account_ip_ranges WHERE service_account_id = ? ORDER BY id",
    )
    .bind(account_id)
    .fetch_all(db)
    .await?;
    Ok(rows)
}

/// Whether `account_id` is allowed to authenticate from `ip`.
///
/// Fail closed: returns `true` only for an existing account with no ranges.
/// A missing account, an unparseable/absent IP when ranges exist, or any
/// database error all deny.
pub async fn allowed(db: &Db, account_id: i64, ip: Option<&str>) -> bool {
    let exists: Result<Option<i64>, sqlx::Error> =
        sqlx::query_scalar("SELECT 1 FROM service_accounts WHERE id = ? LIMIT 1")
            .bind(account_id)
            .fetch_optional(db)
            .await;
    match exists {
        Ok(Some(_)) => {}
        Ok(None) => return false,
        Err(err) => {
            tracing::warn!(error = %err, account_id, "IP allowlist: account lookup failed");
            return false;
        }
    }

    let ranges: Result<Vec<String>, sqlx::Error> = sqlx::query_scalar(
        "SELECT cidr FROM service_account_ip_ranges WHERE service_account_id = ?",
    )
    .bind(account_id)
    .fetch_all(db)
    .await;
    let ranges = match ranges {
        Ok(ranges) => ranges,
        Err(err) => {
            tracing::warn!(error = %err, account_id, "IP allowlist: range lookup failed");
            return false;
        }
    };
    if ranges.is_empty() {
        return true;
    }

    let Some(ip) = ip.and_then(net::normalize_client_ip) else {
        return false;
    };
    ranges.iter().any(|cidr| match cidr.parse::<IpNet>() {
        Ok(network) => network.contains(&ip),
        Err(err) => {
            tracing::warn!(error = %err, cidr, "IP allowlist: stored CIDR is invalid");
            false
        }
    })
}

/// Adds a normalized range, enforcing the cap and rejecting duplicates.
pub async fn add(db: &Db, account_id: i64, cidr: &str) -> ApiResult<ServiceAccountIpRange> {
    let normalized = net::parse_cidr(cidr).map_err(ApiError::bad_request)?;

    let count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM service_account_ip_ranges WHERE service_account_id = ?",
    )
    .bind(account_id)
    .fetch_one(db)
    .await?;
    if count >= MAX_RANGES {
        return Err(ApiError::bad_request(format!(
            "a service account may have at most {MAX_RANGES} IP ranges"
        )));
    }

    let duplicate: Option<i64> = sqlx::query_scalar(
        "SELECT 1 FROM service_account_ip_ranges WHERE service_account_id = ? AND cidr = ? LIMIT 1",
    )
    .bind(account_id)
    .bind(&normalized)
    .fetch_optional(db)
    .await?;
    if duplicate.is_some() {
        return Err(ApiError::conflict("IP range already exists"));
    }

    let now = Utc::now();
    let id = db::with_busy_retry(|| async {
        sqlx::query(
            "INSERT INTO service_account_ip_ranges (service_account_id, cidr, created_at) \
             VALUES (?, ?, ?)",
        )
        .bind(account_id)
        .bind(&normalized)
        .bind(now)
        .execute(db)
        .await
        .map(|result| result.last_insert_rowid())
    })
    .await
    .map_err(ApiError::from)?;

    let row = sqlx::query_as::<_, ServiceAccountIpRange>(
        "SELECT * FROM service_account_ip_ranges WHERE id = ? LIMIT 1",
    )
    .bind(id)
    .fetch_one(db)
    .await?;
    Ok(row)
}

/// Removes one range. `Ok(false)` when it does not belong to the account.
pub async fn remove(db: &Db, account_id: i64, range_id: i64) -> ApiResult<bool> {
    let result = db::with_busy_retry(|| async {
        sqlx::query("DELETE FROM service_account_ip_ranges WHERE id = ? AND service_account_id = ?")
            .bind(range_id)
            .bind(account_id)
            .execute(db)
            .await
    })
    .await
    .map_err(ApiError::from)?;
    Ok(result.rows_affected() > 0)
}

/// Replaces every range for the account inside one transaction.
pub async fn replace_all(db: &Db, account_id: i64, cidrs: &[String]) -> ApiResult<()> {
    let mut tx = db.begin().await?;
    sqlx::query("DELETE FROM service_account_ip_ranges WHERE service_account_id = ?")
        .bind(account_id)
        .execute(&mut *tx)
        .await?;

    let now = Utc::now();
    for cidr in cidrs {
        let normalized = net::parse_cidr(cidr).map_err(ApiError::bad_request)?;
        sqlx::query(
            "INSERT INTO service_account_ip_ranges (service_account_id, cidr, created_at) \
             VALUES (?, ?, ?)",
        )
        .bind(account_id)
        .bind(&normalized)
        .bind(now)
        .execute(&mut *tx)
        .await?;
    }

    tx.commit().await?;
    Ok(())
}

/// Parses, normalizes and de-duplicates a requested range list, preserving order
/// and enforcing [`MAX_RANGES`]. Fails on the first invalid entry.
pub fn normalize_list(raw: &[String]) -> Result<Vec<String>, String> {
    let mut normalized: Vec<String> = Vec::new();
    for entry in raw {
        let cidr = net::parse_cidr(entry)?;
        if !normalized.contains(&cidr) {
            normalized.push(cidr);
        }
    }
    if normalized.len() as i64 > MAX_RANGES {
        return Err(format!(
            "a service account may have at most {MAX_RANGES} IP ranges"
        ));
    }
    Ok(normalized)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::service_accounts::{self, NewServiceAccount};
    use crate::auth::test_support::{create_user, test_state};

    async fn account(state: &crate::state::AppState) -> i64 {
        let user = create_user(state, "owner").await;
        let (account, _token) = service_accounts::create(
            state,
            NewServiceAccount {
                owner_user_id: user.id,
                name: "ci",
                username: "owner-ci",
                description: None,
            },
        )
        .await
        .expect("create");
        account.id
    }

    #[tokio::test]
    async fn allows_only_configured_ranges_and_fails_closed() {
        let (_dir, state) = test_state().await;
        let id = account(&state).await;

        assert!(
            allowed(&state.db, id, Some("1.2.3.4")).await,
            "zero ranges means unrestricted"
        );

        add(&state.db, id, "10.0.0.0/8").await.expect("add");
        assert!(allowed(&state.db, id, Some("10.1.2.3")).await);
        assert!(!allowed(&state.db, id, Some("192.168.1.1")).await);
        assert!(!allowed(&state.db, id, None).await);
        assert!(!allowed(&state.db, id, Some("not-an-ip")).await);

        assert!(
            !allowed(&state.db, id + 9999, Some("10.1.2.3")).await,
            "a missing account is denied"
        );
    }

    #[tokio::test]
    async fn rejects_duplicate_and_invalid_ranges() {
        let (_dir, state) = test_state().await;
        let id = account(&state).await;

        add(&state.db, id, "10.0.0.1/24").await.expect("add");
        assert!(add(&state.db, id, "10.0.0.0/24").await.is_err());
        assert!(add(&state.db, id, "garbage").await.is_err());
    }

    #[test]
    fn normalize_list_dedupes_in_order() {
        let normalized = normalize_list(&["10.0.0.1/24".to_string(), "10.0.0.0/24".to_string()])
            .expect("normalize");
        assert_eq!(normalized, vec!["10.0.0.0/24".to_string()]);
    }
}
