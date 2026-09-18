//! In-memory rate limiters for the login, registration and generic API buckets.

#![allow(dead_code)]

use std::num::NonZeroU32;
use std::time::Duration;

use governor::clock::{Clock, DefaultClock};
use governor::state::keyed::DefaultKeyedStateStore;
use governor::{Quota, RateLimiter};

use crate::config::Config;

/// Sustained request rate for the generic API bucket (requests per second).
const API_PER_SECOND: u32 = 100;
/// Burst allowance for the generic API bucket.
const API_BURST: u32 = 200;

type KeyedLimiter = RateLimiter<String, DefaultKeyedStateStore<String>, DefaultClock>;

/// Result of a rate-limit check.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LimitOutcome {
    /// The request may proceed.
    Allowed,
    /// The bucket is exhausted; retry after the given delay.
    Limited { retry_after: Duration },
}

impl LimitOutcome {
    pub fn is_allowed(self) -> bool {
        matches!(self, Self::Allowed)
    }

    pub fn retry_after(self) -> Option<Duration> {
        match self {
            Self::Limited { retry_after } => Some(retry_after),
            Self::Allowed => None,
        }
    }
}

/// The registry's named rate limiters.
pub struct Limiters {
    login: KeyedLimiter,
    register: KeyedLimiter,
    forgot_password: KeyedLimiter,
    api: KeyedLimiter,
}

impl Limiters {
    /// Builds limiters from the configured login/registration budgets.
    pub fn new(config: &Config) -> Self {
        Self::with_limits(
            config.rate_limit_login_per_minute,
            config.rate_limit_register_per_hour,
        )
    }

    /// Builds limiters from explicit budgets; `0` is clamped to one.
    pub fn with_limits(login_per_minute: u32, register_per_hour: u32) -> Self {
        Self {
            login: RateLimiter::keyed(Quota::per_minute(capacity(login_per_minute))),
            register: RateLimiter::keyed(Quota::per_hour(capacity(register_per_hour))),
            forgot_password: RateLimiter::keyed(Quota::per_hour(capacity(register_per_hour))),
            api: RateLimiter::keyed(
                Quota::per_second(capacity(API_PER_SECOND)).allow_burst(capacity(API_BURST)),
            ),
        }
    }

    /// Checks the login bucket for `key`.
    pub fn check_login(&self, key: &str) -> LimitOutcome {
        check_limiter(&self.login, key)
    }

    /// Checks the registration bucket for `key`.
    pub fn check_register(&self, key: &str) -> LimitOutcome {
        check_limiter(&self.register, key)
    }

    /// Checks the password-reset request bucket for `key`.
    pub fn check_forgot_password(&self, key: &str) -> LimitOutcome {
        check_limiter(&self.forgot_password, key)
    }

    /// Checks the generic API burst bucket for `key`.
    pub fn check_api(&self, key: &str) -> LimitOutcome {
        check_limiter(&self.api, key)
    }

    /// Dispatches to a named bucket, falling back to the generic API bucket.
    pub fn check(&self, bucket: &str, key: &str) -> LimitOutcome {
        match bucket {
            "login" => self.check_login(key),
            "register" => self.check_register(key),
            "forgot-password" => self.check_forgot_password(key),
            _ => self.check_api(key),
        }
    }

    /// Drops bucket state for keys that have been idle long enough to be
    /// replenished, keeping memory bounded on a long-running process.
    pub fn retain_recent(&self) {
        self.login.retain_recent();
        self.register.retain_recent();
        self.forgot_password.retain_recent();
        self.api.retain_recent();
    }

    /// Builds a stable bucket key from a client IP and an optional username.
    pub fn limit_key(ip: Option<&str>, username: Option<&str>) -> String {
        let username = username.map(str::to_ascii_lowercase);
        match (ip, username) {
            (Some(ip), Some(username)) => format!("{ip}:{username}"),
            (Some(ip), None) => ip.to_string(),
            (None, Some(username)) => format!("user:{username}"),
            (None, None) => "unknown".to_string(),
        }
    }
}

fn check_limiter(limiter: &KeyedLimiter, key: &str) -> LimitOutcome {
    match limiter.check_key(&key.to_string()) {
        Ok(()) => LimitOutcome::Allowed,
        Err(not_until) => LimitOutcome::Limited {
            retry_after: not_until.wait_time_from(DefaultClock::default().now()),
        },
    }
}

fn capacity(value: u32) -> NonZeroU32 {
    NonZeroU32::new(value).unwrap_or(NonZeroU32::MIN)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn login_bucket_enforces_limit_per_key() {
        let limiters = Limiters::with_limits(2, 10);
        assert!(limiters.check_login("1.2.3.4:alice").is_allowed());
        assert!(limiters.check_login("1.2.3.4:alice").is_allowed());
        let limited = limiters.check_login("1.2.3.4:alice");
        assert!(!limited.is_allowed());
        assert!(limited.retry_after().is_some());
        assert!(limiters.check_login("5.6.7.8:bob").is_allowed());
    }

    #[test]
    fn register_bucket_is_independent_from_login() {
        let limiters = Limiters::with_limits(1, 1);
        assert!(limiters.check_login("k").is_allowed());
        assert!(limiters.check_register("k").is_allowed());
        assert!(!limiters.check_login("k").is_allowed());
        assert!(!limiters.check_register("k").is_allowed());
    }

    #[test]
    fn generic_check_dispatches_by_bucket_name() {
        let limiters = Limiters::with_limits(1, 1);
        assert!(limiters.check("login", "key").is_allowed());
        assert!(!limiters.check("login", "key").is_allowed());
        assert!(limiters.check("api", "key").is_allowed());
    }

    #[test]
    fn forgot_password_bucket_is_isolated() {
        let limiters = Limiters::with_limits(1, 1);
        assert!(limiters.check_forgot_password("key").is_allowed());
        assert!(!limiters.check_forgot_password("key").is_allowed());
        assert!(limiters.check_login("key").is_allowed());
    }

    #[test]
    fn zero_budget_is_clamped_to_one() {
        let limiters = Limiters::with_limits(0, 0);
        assert!(limiters.check_login("key").is_allowed());
        assert!(!limiters.check_login("key").is_allowed());
    }

    #[test]
    fn limit_key_combines_ip_and_username() {
        assert_eq!(
            Limiters::limit_key(Some("1.2.3.4"), Some("Alice")),
            "1.2.3.4:alice"
        );
        assert_eq!(Limiters::limit_key(Some("1.2.3.4"), None), "1.2.3.4");
        assert_eq!(Limiters::limit_key(None, Some("Bob")), "user:bob");
        assert_eq!(Limiters::limit_key(None, None), "unknown");
    }
}
