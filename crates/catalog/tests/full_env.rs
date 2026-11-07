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
    assert_eq!(
        applied, applied2,
        "idempotent: second run applies nothing new (applied={applied:?} applied2={applied2:?})"
    );

    // 2. Repo against the real engine.
    let repo = TiberiusCourseRepo::connect(&url)
        .await
        .expect("connect repo");
    repo.verify().await.expect("verify");

    // 3. CRUD (FR-002/003).
    let created = repo
        .create("org-full-env", "Full env course", Some("desc"))
        .await
        .expect("create");
    assert!(!created.id.is_empty(), "server-generated id");
    assert_eq!(created.organization_id, "org-full-env");

    let fetched = repo.get(&created.id, "org-full-env").await.expect("get");
    assert_eq!(fetched.title, "Full env course");
    assert_eq!(fetched.description, "desc");

    let filtered = repo
        .list_for("org-full-env", Some("full env"))
        .await
        .expect("list");
    assert!(
        filtered.iter().any(|c| c.id == created.id),
        "filter (LIKE) finds the course on the real engine"
    );

    let updated = repo
        .update(&created.id, "org-full-env", Some("Renamed"), None)
        .await
        .expect("update");
    assert_eq!(updated.title, "Renamed");
    assert_eq!(updated.description, "desc", "COALESCE keeps description");

    // 4. Scope discipline on the real engine (US3.2 — 403-oracle rule).
    let scope = repo.get(&created.id, "other-org").await;
    assert!(
        matches!(scope, Err(CatalogError::Scope)),
        "foreign-org get → Scope, got {scope:?}"
    );
    let missing = repo.get("does-not-exist", "org-full-env").await;
    assert!(
        matches!(missing, Err(CatalogError::NotFound)),
        "absent id → NotFound, got {missing:?}"
    );

    // 5b. Unscoped port lookup (PQ-2 seam: course_title provider).
    let found = repo.find_by_id(&created.id).await;
    assert!(found.is_some(), "find_by_id on the real engine");
    assert_eq!(found.unwrap().title, "Full env course");
    assert!(repo.find_by_id("does-not-exist").await.is_none());

    // 5. Validation on the real engine (FR-007).
    let bad = repo.create("org-full-env", "", None).await;
    assert!(
        matches!(bad, Err(CatalogError::Validation { .. })),
        "{bad:?}"
    );

    // 6. Delete + confirm absent (FR-005).
    repo.delete(&created.id, "org-full-env")
        .await
        .expect("delete");
    let after = repo.get(&created.id, "org-full-env").await;
    assert!(
        matches!(after, Err(CatalogError::NotFound)),
        "deleted course is NotFound, got {after:?}"
    );

    println!("FULL-ENV CATALOG: OK (real MSSQL)");
}
