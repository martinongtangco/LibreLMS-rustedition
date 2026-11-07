//! US2 — migration idempotency + manifest integrity (spec US2.3, research R1).

use shared::migrate::{checksum, MIGRATIONS};
use shared::storage::{MemoryStorage, Storage};

#[tokio::test]
async fn second_migration_run_is_a_noop_with_bookkeeping() {
    let s = MemoryStorage::new();
    let first = s
        .run_migrations(shared::migrate::MIGRATIONS)
        .await
        .expect("first run");
    assert_eq!(
        first,
        vec!["0001_base", "0002_seed"],
        "all scripts applied once"
    );

    let second = s
        .run_migrations(shared::migrate::MIGRATIONS)
        .await
        .expect("second run");
    assert!(
        second.is_empty(),
        "second run must apply nothing (idempotent)"
    );

    // Third run as well — property, not a one-off.
    let third = s
        .run_migrations(shared::migrate::MIGRATIONS)
        .await
        .expect("third run");
    assert!(
        third.is_empty(),
        "third run must apply nothing (idempotent)"
    );
}

#[tokio::test]
async fn checksums_stable_and_distinct() {
    let mut sums: Vec<String> = Vec::new();
    for (_, sql) in MIGRATIONS {
        let a = checksum(sql);
        let b = checksum(sql);
        assert_eq!(a, b, "checksums are deterministic");
        assert_eq!(a.len(), 64, "sha256 hex length");
        assert!(!sums.contains(&a), "checksums are distinct per script");
        sums.push(a);
    }
}

#[test]
fn manifest_order_matches_migration_files() {
    // The manifest order is the application order (research R1).
    assert_eq!(MIGRATIONS[0].0, "0001_base");
    assert_eq!(MIGRATIONS[1].0, "0002_seed");
    for (name, sql) in MIGRATIONS {
        assert!(!sql.trim().is_empty(), "{name} must not be empty");
        assert!(
            sql.contains(';') || sql.lines().count() > 1,
            "{name} is a real script"
        );
    }
}
