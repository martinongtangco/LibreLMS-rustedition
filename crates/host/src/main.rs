//! Binary entry point: env config → real backends → host::run (FR-001).

use shared::config::Config;
use shared::session::ValkeySessionStore;
use shared::storage::TiberiusStorage;
use std::sync::Arc;
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() {
    // FR-009: the subscriber never receives config values; log lines never
    // include DATABASE_URL/VALKEY_URL content (the shared error types carry
    // only backend error text).
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();

    let cfg = match Config::from_env() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(1);
        }
    };

    let storage: Arc<dyn shared::storage::Storage> = match TiberiusStorage::connect(&cfg.database_url)
        .await
    {
        Ok(s) => Arc::new(s),
        Err(e) => {
            eprintln!("startup failed: DATABASE_URL backend: {e}");
            std::process::exit(1);
        }
    };
    let store: Arc<dyn shared::session::SessionStore> = match ValkeySessionStore::connect(&cfg.valkey_url)
        .await
    {
        Ok(s) => Arc::new(s),
        Err(e) => {
            eprintln!("startup failed: VALKEY_URL backend: {e}");
            std::process::exit(1);
        }
    };

    if let Err(e) = host::run(cfg, storage, store).await {
        eprintln!("{e}");
        std::process::exit(1);
    }
}
