//! Full-environment, REAL-ENGINE enrollment tests (spec 004 SC-003 + US3.2 +
//! FR-008). The Module Persistence Seam proven against real MSSQL.
//! GATED: runs only when DATABASE_URL is set (Principle XVII: disposable
//! env). A SKIP here is NOT a pass — the fresh-container run is the
//! deciding evidence.
//!
//! SC-003 race proof (plan PQ-1 + CR-1): N INDEPENDENT `TiberiusEnrollmentRepo`
//! instances (one connection each — a single-client repo would serialize the
//! joins on its own mutex and the UNIQUE constraint would never see a real
//! contest), `tokio::join!`-ed.

use std::sync::Arc;

use enrollment::migrations::MIGRATIONS as ENROLLMENT_MIGRATIONS;
use enrollment::repo::{Enrollment, EnrollmentError, EnrollmentRepo, Status};
use enrollment::tiberius::TiberiusEnrollmentRepo;
use shared::storage::{Storage as _, TiberiusStorage};

const RACE_LEARNER: &str = "00000000-aaaa-0000-0000-000000000001";
const RACE_ORG: &str = "full-env-enrollment";

fn env(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|v| !v.is_empty())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn real_engine_full_battery() {
    let url = match env("DATABASE_URL") {
        Some(u) => u,
        None => {
            eprintln!(
                "SKIP: DATABASE_URL not set — real-engine enrollment tests need a \
                 disposable environment (Principle XVII). The fresh-container \
                 run is the deciding evidence."
            );
            return;
        }
    };

    // 1. Migrations (foundation + enrollment) — idempotent on the real engine.
    let storage = TiberiusStorage::connect(&url)
        .await
        .expect("connect storage");
    let mut manifest: Vec<(&str, &str)> = shared::migrate::MIGRATIONS
        .iter()
        .map(|(n, sql)| (*n, *sql))
        .collect();
    manifest.extend(ENROLLMENT_MIGRATIONS.iter().map(|(n, sql)| (*n, *sql)));
    manifest.sort_by(|a, b| a.0.cmp(b.0));
    let applied = storage
        .run_migrations(&manifest)
        .await
        .expect("migration 1");
    eprintln!("applied on run 1: {applied:?}");
    let applied2 = storage
        .run_migrations(&manifest)
        .await
        .expect("migration 2");
    assert!(
        applied2.is_empty(),
        "idempotent: second run is a no-op (applied2={applied2:?})"
    );

    // 2. Repo against the real engine.
    let repo = TiberiusEnrollmentRepo::connect(&url)
        .await
        .expect("connect repo");
    repo.verify().await.expect("verify");

    // 3. The SC-003 RACE PROOF (plan PQ-1 + CR-1): N=8 independent repos
    // (one connection each) — a genuine engine-side contest on the UNIQUE
    // (learner_id, course_id) constraint. A unique run suffix keeps repeat
    // runs from colliding with their own previous rows.
    let suffix = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let race_course = format!("race-{suffix}");
    let mut repos: Vec<Arc<TiberiusEnrollmentRepo>> = Vec::with_capacity(8);
    for _ in 0..8 {
        repos.push(Arc::new(
            TiberiusEnrollmentRepo::connect(&url)
                .await
                .expect("race repo connect"),
        ));
    }
    // 8 independent repo instances, one connection each — a single-client
    // repo would serialize the contest. (Spawn ≠ immediate execution; the
    // proof stands on the independent connections + engine arbitration +
    // the row-count/code assertions below.)
    // before any is awaited — a genuine engine-side contest.
    let handles: Vec<_> = repos
        .iter()
        .map(|r| {
            let r = Arc::clone(r);
            let course = race_course.clone();
            tokio::spawn(async move { r.create(RACE_LEARNER, &course, RACE_ORG).await })
        })
        .collect();
    let mut results: Vec<Result<Enrollment, EnrollmentError>> = Vec::with_capacity(8);
    for h in handles {
        results.push(h.await.expect("race join"));
    }
    let wins = results.iter().filter(|r| r.is_ok()).count();
    let losses = results
        .iter()
        .filter(|r| matches!(r, Err(EnrollmentError::AlreadyEnrolled)))
        .count();
    assert_eq!(wins, 1, "exactly one winner; results: {results:?}");
    assert_eq!(
        losses, 7,
        "the losers ALL got the stable code (2601/2627 mapping fired — never a \
         500, never a second row): {results:?}"
    );
    let listed = repo
        .list_for_learner(RACE_LEARNER, RACE_ORG)
        .await
        .expect("race list");
    let race_rows = listed.iter().filter(|e| e.course_id == race_course).count();
    assert_eq!(race_rows, 1, "exactly one row in the engine: {listed:?}");

    // 4. First-completion-wins on the real engine (US3.2 — the PK is the
    // engine anchor; no UPDATE path).
    let enrolled = listed
        .iter()
        .find(|e| e.course_id == race_course)
        .expect("the one row");
    let done = repo
        .complete(RACE_LEARNER, &enrolled.id, RACE_ORG)
        .await
        .expect("complete");
    assert_eq!(
        done.status,
        Status::Completed,
        "transition on the real engine"
    );
    assert!(done.completed_at.is_some(), "timestamped completion record");
    let first_ts = done.completed_at.expect("ts");
    let again = repo.complete(RACE_LEARNER, &enrolled.id, RACE_ORG).await;
    assert!(
        matches!(again, Err(EnrollmentError::AlreadyCompleted)),
        "second completion rejected on the real engine: {again:?}"
    );
    let after = repo
        .list_for_learner(RACE_LEARNER, RACE_ORG)
        .await
        .expect("list after");
    let after_row = after
        .iter()
        .find(|e| e.course_id == race_course)
        .expect("row");
    assert_eq!(
        after_row.completed_at,
        Some(first_ts),
        "original completion record unchanged"
    );

    // 5. Durability (US3.4): a FRESH connection sees the same state.
    let fresh = TiberiusEnrollmentRepo::connect(&url)
        .await
        .expect("reconnect");
    let durable = fresh
        .list_for_learner(RACE_LEARNER, RACE_ORG)
        .await
        .expect("durable list");
    let durable_row = durable
        .iter()
        .find(|e| e.course_id == race_course)
        .expect("durable row");
    assert_eq!(
        durable_row.completed_at,
        Some(first_ts),
        "rows survive a fresh client reconnect"
    );

    println!("FULL-ENV ENROLLMENT: OK (real MSSQL; race proof + idempotency + durability)");
}
