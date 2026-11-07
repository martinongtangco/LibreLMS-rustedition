//! US3 — ephemeral bag: a session-store flush loses only the bag; the session
//! row (system of record) survives (spec US3.4, Principle VI).

use rand::SeedableRng;
use shared::auth::{establish, verify};
use shared::session::{MemorySessionStore, SessionStore};
use shared::storage::{MemoryStorage, Storage};

#[tokio::test]
async fn store_flush_loses_only_the_bag() {
    let s = MemoryStorage::new();
    s.run_migrations(shared::migrate::MIGRATIONS)
        .await
        .expect("migrate");
    let mut rng = rand::rngs::StdRng::seed_from_u64(7);
    let (token, _row) = establish(
        &s as &dyn Storage,
        &mut rng,
        "dev-user",
        "dev-org",
        1_000,
        3_600,
        72_000,
    )
    .await
    .expect("establish");
    let h = shared::auth::token_hash(&token);

    // Bag lives in the (ephemeral) session store.
    let live = MemorySessionStore::new();
    live.set_bag(&h, "{\"cmi.core.lesson_status\":\"incomplete\"}", 3_600)
        .await
        .expect("set bag");
    assert!(live.get_bag(&h).await.unwrap().is_some(), "bag present");

    // "Flush" the store: a fresh instance stands in for the flushed store.
    let flushed = MemorySessionStore::new();
    assert!(
        flushed.get_bag(&h).await.unwrap().is_none(),
        "flush loses the bag"
    );

    // The session row (DB) survives the flush — login state is not lost.
    verify(&s as &dyn Storage, &token, "dev-org", 1_100, 3_600, 72_000)
        .await
        .expect("session row survives the store flush");
}

#[tokio::test]
async fn bag_ttl_expires_with_the_sliding_clock() {
    let store = MemorySessionStore::new();
    store.set_bag("h1", "{}", 100).await.expect("set");
    store.set_now(90);
    assert!(store.get_bag("h1").await.unwrap().is_some());
    store.set_now(150);
    assert!(store.get_bag("h1").await.unwrap().is_none(), "TTL elapsed");
}
