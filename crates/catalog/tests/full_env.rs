//! Full-environment, REAL-ENGINE catalog tests (spec 003, US3.2 + FR-002/003/
//! 005/008) — the Module Persistence Seam proven against real MSSQL.
//! GATED: runs only when DATABASE_URL is set (Principle XVII: disposable
//! env). A SKIP here is NOT a pass — the peer's fresh-container run is the
//! deciding evidence.

use catalog::migrations::MIGRATIONS as CATALOG_MIGRATIONS;
use catalog::repo::{CatalogError, CourseRepo};
use catalog::tiberius::TiberiusCourseRepo;
use shared::storage::{Storage as _, TiberiusStorage};

fn env(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|v| !v.is_empty())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn real_engine_full_crud() {
    let url = match env("DATABASE_URL") {
        Some(u) => u,
        None => {
            eprintln!(
                "SKIP: DATABASE_URL not set — real-engine catalog tests need a disposable \
                 environment (Principle XVII). The peer's fresh-container run is the \
                 deciding evidence."
            );
            return;
        }
    };

    // 1. Migrations (foundation + catalog) — idempotent.
    let storage = TiberiusStorage::connect(&url)
        .await
        .expect("connect storage");
    let mut manifest: Vec<(&str, &str)> = shared::migrate::MIGRATIONS
        .iter()
        .map(|(n, sql)| (*n, *sql))
        .collect();
    manifest.extend(CATALOG_MIGRATIONS.iter().map(|(n, sql)| (*n, *sql)));
    let applied = storage
        .run_migrations(&manifest)
        .await
        .expect("migration 1");
    let applied2 = storage
        .run_migrations(&manifest)
        .await
        .expect("migration 2");
    // 001 US2.3 contract: the SECOND run is a no-op. (CR-6: on a fresh
    // container the first run applies [0001, 0002, 0003] and the second
    // applies [] — `applied == applied2` can never hold fresh.)
    assert!(
        applied2.is_empty(),
        "idempotent: second run applies nothing (applied={applied:?} applied2={applied2:?})"
    );

    // 2. Repo against the real engine.
    let repo = TiberiusCourseRepo::connect(&url)
        .await
        .expect("connect repo");
    repo.verify().await.expect("verify");

    // 3. CRUD (FR-002/003).
    // UUID-shaped orgs: the DDL stores organization_id as UNIQUEIDENTIFIER
    // with CAST at the wire edge (001 convention) — a non-UUID org would
    // fail the conversion, so the battery uses UUID-shaped strings to
    // exercise exactly that path.
    const ORG: &str = "11111111-1111-1111-1111-111111111111";
    const OTHER: &str = "22222222-2222-2222-2222-222222222222";

    let created = repo
        .create(ORG, "Full env course", Some("desc"))
        .await
        .expect("create");
    assert!(!created.id.is_empty(), "server-generated id");
    assert_eq!(created.organization_id, ORG);

    let fetched = repo.get(&created.id, ORG).await.expect("get");
    assert_eq!(fetched.title, "Full env course");
    assert_eq!(fetched.description, "desc");

    let filtered = repo.list_for(ORG, Some("full env")).await.expect("list");
    assert!(
        filtered.iter().any(|c| c.id == created.id),
        "filter (LIKE) finds the course on the real engine"
    );

    let updated = repo
        .update(&created.id, ORG, Some("Renamed"), None)
        .await
        .expect("update");
    assert_eq!(updated.title, "Renamed");
    assert_eq!(updated.description, "desc", "COALESCE keeps description");

    // 4. Scope discipline on the real engine (US3.2 — 403-oracle rule).
    let scope = repo.get(&created.id, OTHER).await;
    assert!(
        matches!(scope, Err(CatalogError::Scope)),
        "foreign-org get → Scope, got {scope:?}"
    );
    let missing = repo.get("does-not-exist", ORG).await;
    assert!(
        matches!(missing, Err(CatalogError::NotFound)),
        "absent id → NotFound, got {missing:?}"
    );

    // 5b. Unscoped port lookup (PQ-2 seam: course_title provider).
    let found = repo.find_by_id(&created.id).await.expect("find");
    assert!(found.is_some(), "find_by_id on the real engine");
    assert_eq!(found.unwrap().title, "Renamed");
    let absent = repo.find_by_id("does-not-exist").await.expect("find");
    assert!(absent.is_none(), "absent → Ok(None) on the real engine");

    // 5. Validation on the real engine (FR-007).
    let bad = repo.create(ORG, "", None).await;
    assert!(
        matches!(bad, Err(CatalogError::Validation { .. })),
        "{bad:?}"
    );

    // 5c. CR-4: 5000-char boundary — proves the NVARCHAR(MAX) column AND the
    // tiberius >4000-char wire path in one assert (Rust strings are uncapped,
    // so no T-SQL REPLICATE cap gotcha applies here).
    let big: String = "a".repeat(5_000);
    let bigc = repo
        .create(ORG, "Boundary course", Some(&big))
        .await
        .expect("create 5000-char");
    let bigr = repo.get(&bigc.id, ORG).await.expect("read back 5000-char");
    assert_eq!(
        bigr.description.len(),
        5_000,
        "5000-char description round-trips (NVARCHAR(MAX) + tiberius wire)"
    );
    repo.delete(&bigc.id, ORG).await.expect("delete boundary");

    // 6. Delete + confirm absent (FR-005).
    repo.delete(&created.id, ORG).await.expect("delete");
    let after = repo.get(&created.id, ORG).await;
    assert!(
        matches!(after, Err(CatalogError::NotFound)),
        "deleted course is NotFound, got {after:?}"
    );

    println!("FULL-ENV CATALOG: OK (real MSSQL)");
}
