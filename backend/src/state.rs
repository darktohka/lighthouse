use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::config::Config;
use crate::db::Db;
use crate::layer_cache::{
    ChangesCache, ComposedLayerCache, LayerCacheConfig, LayerIndexCache, ListingCache,
};
use crate::ratelimit::Limiters;
use crate::storage::Storage;
use crate::storage::registry::Registry;

/// How a request's identity was established. Registry bearer tokens are an
/// identity for `/v2` only: [`CredentialSource::RegistryToken`] is deliberately
/// rejected by the control-plane extractors so a token cached by `docker`
/// cannot act as a web session.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum CredentialSource {
    #[default]
    None,
    AccessToken,
    Password,
    AppPassword,
    ServiceAccount,
    RegistryToken,
}

/// Identity attached to a request by the authentication middleware. Inserted as
/// a request extension so downstream handlers and the access log can read it.
#[derive(Debug, Clone, Default)]
pub struct AuthContext {
    pub user_id: Option<i64>,
    pub username: Option<String>,
    pub service_account_id: Option<i64>,
    /// Set when the caller authenticated with an app password, so registry
    /// refresh tokens can be revoked together with the credential.
    pub app_password_id: Option<i64>,
    pub is_admin: bool,
    pub credential: CredentialSource,
    /// Access-token `jti` when the caller presented a web session, so 2FA
    /// enrolment can revoke every *other* session.
    pub session_id: Option<String>,
}

impl AuthContext {
    pub fn anonymous() -> Self {
        Self::default()
    }

    pub fn is_authenticated(&self) -> bool {
        self.user_id.is_some() || self.service_account_id.is_some()
    }

    /// True when the caller presented a valid registry bearer token, even an
    /// anonymous one. Used to answer `DENIED` instead of re-issuing a `401`
    /// challenge, which would make `docker` fetch tokens in a loop.
    pub fn is_registry_token(&self) -> bool {
        self.credential == CredentialSource::RegistryToken
    }
}

#[derive(Clone)]
pub struct AppState {
    pub config: Arc<Config>,
    pub db: Db,
    pub storage: Arc<Storage>,
    pub registry: Arc<Registry>,
    pub limiters: Arc<Limiters>,
    pub layer_cache: Arc<LayerIndexCache>,
    /// Cumulative overlays keyed by `(manifest_digest, layer_position)`; see
    /// [`crate::layer_cache::ComposedLayerCache`].
    pub composed_cache: Arc<ComposedLayerCache>,
    /// Diff classifications keyed by `(manifest_digest, layer_position)`; see
    /// [`crate::layer_cache::ChangesCache`].
    pub changes_cache: Arc<ChangesCache>,
    /// Rendered directory listings; see [`crate::layer_cache::ListingCache`].
    pub listing_cache: Arc<ListingCache>,
}

impl AppState {
    /// Builds the shared application state, creating the content-store layout.
    pub async fn new(config: Config, db: Db) -> anyhow::Result<Self> {
        let storage = Arc::new(Storage::new(&config)?);
        storage.ensure_layout().await?;

        let registry = Arc::new(Registry::new(db.clone(), Arc::clone(&storage)));
        let limiters = Arc::new(Limiters::new(&config));
        let ttl = Duration::from_secs(config.layer_cache_ttl_secs);
        let layer_cache = Arc::new(LayerIndexCache::new(LayerCacheConfig {
            max_bytes: config.layer_cache_max_bytes,
            ttl,
        }));
        let composed_cache = Arc::new(ComposedLayerCache::new(
            LayerCacheConfig {
                max_bytes: config.composed_cache_bytes,
                ttl,
            },
            layer_cache.generation(),
        ));
        let changes_cache = Arc::new(ChangesCache::new(
            LayerCacheConfig {
                max_bytes: config.changes_cache_bytes,
                ttl,
            },
            layer_cache.generation(),
        ));
        let listing_cache = Arc::new(ListingCache::new(
            LayerCacheConfig {
                max_bytes: config.listing_cache_bytes,
                ttl,
            },
            layer_cache.generation(),
        ));

        Ok(Self {
            config: Arc::new(config),
            db,
            storage,
            registry,
            limiters,
            layer_cache,
            composed_cache,
            changes_cache,
            listing_cache,
        })
    }

    /// Runs periodic upkeep on every cache and the rate limiters: drops TTL-
    /// expired entries, drops entries derived from a stale layer-index
    /// generation, and discards idle rate-limit buckets. Called from a background
    /// task so memory does not stay resident between requests.
    pub fn maintain(&self) {
        self.layer_cache.sweep_expired();
        self.composed_cache.sweep_expired();
        self.composed_cache.sweep_stale_generation();
        self.changes_cache.sweep_expired();
        self.changes_cache.sweep_stale_generation();
        self.listing_cache.sweep_expired();
        self.listing_cache.sweep_stale_generation();
        self.limiters.retain_recent();
    }

    /// The earliest instant any cached entry expires, or `None` when every cache
    /// is empty. The maintenance task sleeps until then.
    pub fn next_cache_deadline(&self) -> Option<Instant> {
        [
            self.layer_cache.next_deadline(),
            self.composed_cache.next_deadline(),
            self.changes_cache.next_deadline(),
            self.listing_cache.next_deadline(),
        ]
        .into_iter()
        .flatten()
        .min()
    }

    /// A handle woken when the layer-index generation advances, so the
    /// maintenance task can drop newly-stale derived entries immediately.
    pub fn caches_changed(&self) -> Arc<tokio::sync::Notify> {
        self.layer_cache.changed()
    }
}
