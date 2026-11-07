//! Deterministic double battery (spec 004 US1/US2/US3 logic layer + the
//! CourseSource fixture contract). Red-first: this file exists before the
//! full double implementation (T004). The SEQUENTIAL duplicate test here is
//! LOGIC-LAYER — the race proof is the full-env test with N independent
//! connections (plan PQ-1).

use enrollment::memory::{InMemoryCourseSource, InMemoryEnrollmentRepo};
use enrollment::repo::{CourseSource, EnrollmentError, EnrollmentRepo, Status};

const ORG_A: &str = "org-a";
const ORG_B: &str = "org-b";
const USER_A: &str = "11111111-1111-1111-1111-111111111111";
const USER_B: &str = "22222222-2222-2222-2222-222222222222";

#[tokio::test]
async fn create_in_scope_persists_with_shape() {
    let repo = InMemoryEnrollmentRepo::new();
    let e = repo.create(USER_A, "c1", ORG_A).await.expect("create");
    assert_eq!(e.learner_id, USER_A);
    assert_eq!(e.course_id, "c1");
    assert_eq!(e.organization_id, ORG_A);
    assert_eq!(e.status, Status::Enrolled);
    assert!(e.created_at > 0, "timestamp set");
    let listed = repo.list_for_learner(USER_A, ORG_A).await.expect("list");
    assert_eq!(listed.len(), 1, "durable in the double");
    assert_eq!(listed[0].id, e.id);
}

#[tokio::test]
async fn duplicate_create_is_rejected_single_row_logic_layer() {
    // LOGIC-LAYER: the sequential duplicate rule. The RACE proof (SC-003) is
    // the full-env test with N independent connections (plan PQ-1) — a
    // mutex-serialized double cannot prove a race.
    let repo = InMemoryEnrollmentRepo::new();
    repo.create(USER_A, "c1", ORG_A).await.expect("first");
    let dup = repo.create(USER_A, "c1", ORG_A).await;
    assert!(
        matches!(dup, Err(EnrollmentError::AlreadyEnrolled)),
        "sequential duplicate → AlreadyEnrolled, got {dup:?}"
    );
    let listed = repo.list_for_learner(USER_A, ORG_A).await.expect("list");
    assert_eq!(listed.len(), 1, "single row");
}

#[tokio::test]
async fn list_is_org_scoped_both_views() {
    let repo = InMemoryEnrollmentRepo::new();
    repo.create(USER_A, "c1", ORG_A).await.expect("a1");
    repo.create(USER_B, "c1", ORG_B).await.expect("b1");
    // Own view: only the actor's rows.
    let own = repo.list_for_learner(USER_A, ORG_A).await.expect("own");
    assert_eq!(own.len(), 1);
    assert_eq!(own[0].learner_id, USER_A);
    // Org-wide view: all org rows — never other-org rows (SC-002).
    let org_a = repo.list_for_org(ORG_A).await.expect("org a");
    assert_eq!(org_a.len(), 1);
    let org_b = repo.list_for_org(ORG_B).await.expect("org b");
    assert_eq!(org_b.len(), 1);
    assert_eq!(org_b[0].organization_id, ORG_B);
    // Cross-org own-listing: nothing.
    let cross = repo.list_for_learner(USER_A, ORG_B).await.expect("cross");
    assert!(cross.is_empty(), "org B never sees user A's rows");
    // Empty list is Ok(empty), not an error.
    let empty = repo
        .list_for_learner("99999999-9999-9999-9999-999999999999", ORG_A)
        .await
        .expect("empty");
    assert!(empty.is_empty());
}

#[tokio::test]
async fn complete_transitions_first_wins() {
    let repo = InMemoryEnrollmentRepo::new();
    let e = repo.create(USER_A, "c1", ORG_A).await.expect("create");
    let done = repo.complete(USER_A, &e.id, ORG_A).await.expect("complete");
    assert_eq!(done.status, Status::Completed, "transition");
    assert!(done.completed_at.is_some(), "completion record with timestamp");
    let first_ts = done.completed_at.expect("ts");
    // Second completion → rejected, original record unchanged (US3.2).
    let again = repo.complete(USER_A, &e.id, ORG_A).await;
    assert!(
        matches!(again, Err(EnrollmentError::AlreadyCompleted)),
        "double completion → AlreadyCompleted, got {again:?}"
    );
    let after = repo.list_for_learner(USER_A, ORG_A).await.expect("list");
    assert_eq!(after[0].completed_at, Some(first_ts), "original timestamp preserved");
    // Exactly one completion exists per enrollment (the PK's double analog).
    let org = repo.list_for_org(ORG_A).await.expect("org");
    assert_eq!(org.len(), 1);
}

#[tokio::test]
async fn complete_gates_notfound_scope_forbidden() {
    let repo = InMemoryEnrollmentRepo::new();
    let e = repo.create(USER_A, "c1", ORG_A).await.expect("create");
    // Unknown id → NotFound (US3.3).
    let nf = repo.complete(USER_A, "does-not-exist", ORG_A).await;
    assert!(matches!(nf, Err(EnrollmentError::NotFound)), "{nf:?}");
    // Out-of-scope completion → Scope, nothing persisted (US3.3, FR-003 both
    // directions).
    let sc = repo.complete(USER_A, &e.id, ORG_B).await;
    assert!(matches!(sc, Err(EnrollmentError::Scope)), "{sc:?}");
    let still = repo.list_for_learner(USER_A, ORG_A).await.expect("list");
    assert_eq!(still[0].status, Status::Enrolled, "nothing persisted on scope rejection");
    // Cross-learner → Forbidden (the learner's own completion only).
    let fb = repo.complete(USER_B, &e.id, ORG_A).await;
    assert!(matches!(fb, Err(EnrollmentError::Forbidden)), "{fb:?}");
}

#[tokio::test]
async fn outage_is_unavailable_never_silent() {
    let repo = InMemoryEnrollmentRepo::new();
    repo.set_unavailable_for_test(true);
    assert!(
        matches!(
            repo.create(USER_A, "c1", ORG_A).await,
            Err(EnrollmentError::Unavailable)
        ),
        "outage → Unavailable (never 500, never a silent empty list)"
    );
    assert!(matches!(
        repo.list_for_learner(USER_A, ORG_A).await,
        Err(EnrollmentError::Unavailable)
    ));
    let src = InMemoryCourseSource::new();
    src.set_unavailable_for_test(true);
    assert!(
        matches!(src.course_title("c1").await, Err(EnrollmentError::Unavailable)),
        "port outage → Err (N1: never Ok(None) — an outage must never be \
         conflated with absence)"
    );
}

#[tokio::test]
async fn course_source_fixture_contract() {
    let src = InMemoryCourseSource::new();
    src.seed("c1", ORG_A, "Rust for LMS").await;
    src.seed("c2", ORG_B, "Go Fundamentals").await;
    // 404/403 decision procedure inputs (plan).
    assert_eq!(
        src.course_title("c1").await.expect("title"),
        Some("Rust for LMS".to_string())
    );
    assert_eq!(
        src.course_title("missing").await.expect("absent"),
        None,
        "absent → None (404-class)"
    );
    assert!(src.course_exists_in_org("c1", ORG_A).await.expect("in org"));
    assert!(
        !src.course_exists_in_org("c1", ORG_B).await.expect("out of org"),
        "foreign org → false (403-class)"
    );
    assert!(
        !src.course_exists_in_org("missing", ORG_A).await.expect("absent"),
        "absent → false (indistinguishable from out-of-scope — no oracle)"
    );
}
