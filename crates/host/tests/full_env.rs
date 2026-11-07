//! Full-environment integration (Principle XIII/XVI): runs ONLY when
//! DATABASE_URL and VALKEY_URL point at live services. When absent the test
//! is SKIPPED (reported, not faked — a step that cannot fail is not evidence).

use shared::session::{MemorySessionStore, SessionStore, ValkeySessionStore};
use shared::storage::{MemoryStorage, SchemaStatus, Storage, TiberiusStorage};
use std::sync::Arc;

fn env_or_skip(var: &str) -> Option<String> {
    std::env::var(var).ok().filter(|v| !v.trim().is_empty())
}

#[tokio::test]
async fn full_environment_migrations_health_and_bag() {
    let Some(db) = env_or_skip("DATABASE_URL") else {
        eprintln!("SKIP: DATABASE_URL not set (deterministic suite covers the logic)");
        return;
    };
    let Some(vk) = env_or_skip("VALKEY_URL") else {
        eprintln!("SKIP: VALKEY_URL not set (deterministic suite covers the logic)");
        return;
    };

    // Storage: connect, verify, schema (may be not-ready on a fresh DB),
    // migrate, verify ready.
    let storage: Arc<dyn Storage> =
        Arc::new(TiberiusStorage::connect(&db).await.expect("mssql connects"));
    storage.verify().await.expect("mssql verify");
    let before = storage.schema_status().await.expect("schema status");
    let applied = storage
        .run_migrations(shared::migrate::MIGRATIONS)
        .await
        .expect("migrate");
    eprintln!("applied: {applied:?}");
    let after = storage.schema_status().await.expect("schema status after");
    assert_eq!(
        after,
        SchemaStatus::Ready,
        "schema ready after migrations (was {before:?})"
    );
    // Idempotency on the real engine too (spec US2.3).
    let again = storage
        .run_migrations(shared::migrate::MIGRATIONS)
        .await
        .expect("migrate again");
    assert!(again.is_empty(), "real engine: second run is a no-op");

    // Session store: verify, bag set/get/refresh.
    let store: Arc<dyn SessionStore> = Arc::new(
        ValkeySessionStore::connect(&vk)
            .await
            .expect("valkey connects"),
    );
    store.verify().await.expect("valkey verify");
    store
        .set_bag("full-env-test", "{\"ok\":true}", 60)
        .await
        .expect("set bag");
    let got = store.get_bag("full-env-test").await.expect("get bag");
    assert_eq!(got.as_deref(), Some("{\"ok\":true}"));
    store
        .refresh_ttl("full-env-test", 60)
        .await
        .expect("refresh");

    // Memory doubles behave identically at the trait level (contract check).
    let mem: Arc<dyn Storage> = Arc::new(MemoryStorage::new());
    mem.run_migrations(shared::migrate::MIGRATIONS)
        .await
        .expect("memory migrate");
    let mem_store = MemorySessionStore::new();
    mem_store.set_bag("h", "{}", 60).await.expect("memory bag");
    assert_eq!(mem_store.get_bag("h").await.unwrap().as_deref(), Some("{}"));
}
