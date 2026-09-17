use std::sync::Arc;

use crate::config::Config;
use crate::db::Db;
use crate::ratelimit::Limiters;
use crate::storage::Storage;
use crate::storage::registry::Registry;

/// Identity attached to a request by the authentication middleware. Inserted as
/// a request extension so downstream handlers and the access log can read it.
#[derive(Debug, Clone, Default)]
pub struct AuthContext {
    pub user_id: Option<i64>,
    pub username: Option<String>,
    pub service_account_id: Option<i64>,
    pub is_admin: bool,
}

impl AuthContext {
    pub fn anonymous() -> Self {
        Self::default()
    }

    pub fn is_authenticated(&self) -> bool {
        self.user_id.is_some() || self.service_account_id.is_some()
    }
}

#[derive(Clone)]
pub struct AppState {
    pub config: Arc<Config>,
    pub db: Db,
    pub storage: Arc<Storage>,
    pub registry: Arc<Registry>,
    pub limiters: Arc<Limiters>,
}

impl AppState {
    /// Builds the shared application state, creating the content-store layout.
    pub async fn new(config: Config, db: Db) -> anyhow::Result<Self> {
        let storage = Arc::new(Storage::new(&config)?);
        storage.ensure_layout().await?;

        let registry = Arc::new(Registry::new(db.clone(), Arc::clone(&storage)));
        let limiters = Arc::new(Limiters::new(&config));

        Ok(Self {
            config: Arc::new(config),
            db,
            storage,
            registry,
            limiters,
        })
    }
}
