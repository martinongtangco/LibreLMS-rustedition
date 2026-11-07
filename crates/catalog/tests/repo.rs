//! US1 (P1) — scoped listing + title filter. Red-first (Principle VII): these
//! tests define the `InMemoryCourseRepo` behavior; T004 makes them green.
//! Fixtures: 2 organizations, 5 courses (SC-002/SC-004 battery).

use catalog::repo::{CourseRepo, InMemoryCourseRepo};
use contracts::Course;

const ORG_A: &str = "org-a";
const ORG_B: &str = "org-b";

/// The fixture battery (spec SC-002: precision/recall material).
fn fixture() -> Vec<Course> {
    let c = |id: &str, title: &str, org: &str| Course {
        id: id.to_string(),
        title: title.to_string(),
        description: String::new(),
        organization_id: org.to_string(),
    };
    vec![
        c("c1", "Rust for LMS", ORG_A),
        c("c2", "rust security", ORG_A),
        c("c3", "Go Fundamentals", ORG_A),
        c("c4", "Go Deep Dive", ORG_A),
        c("c5", "Rust for LMS (B org)", ORG_B),
    ]
}

#[tokio::test]
async fn list_returns_exactly_the_session_orgs_courses() {
    let repo = InMemoryCourseRepo::new();
    for c in &fixture() {
        repo.create_for_test(c.clone()).await.expect("seed");
    }
    let a = repo.list_for(ORG_A, None).await.expect("list A");
    assert_eq!(a.len(), 4, "org A sees exactly its 4 courses (SC-004)");
    let b = repo.list_for(ORG_B, None).await.expect("list B");
    assert_eq!(b.len(), 1, "org B sees exactly its 1 course (SC-004)");
    assert!(
        a.iter().all(|c| c.organization_id == ORG_A),
        "no cross-org leakage in A's list"
    );
}

#[tokio::test]
async fn filter_is_case_insensitive_substring_with_full_precision_and_recall() {
    let repo = InMemoryCourseRepo::new();
    for c in &fixture() {
        repo.create_for_test(c.clone()).await.expect("seed");
    }
    // "go" → the two Go courses; "Rust for LMS (B org)" is in org B and "rust
    // security" contains "rust", not "go" — precision.
    let hits = repo.list_for(ORG_A, Some("go")).await.expect("filter go");
    assert_eq!(hits.len(), 2, "recall: both Go courses match");
    assert!(
        hits.iter().all(|c| c.title.to_lowercase().contains("go")),
        "precision: nothing non-matching appears"
    );
    // Case-insensitivity: "RUST" matches both org-A rust titles.
    let hits = repo
        .list_for(ORG_A, Some("RUST"))
        .await
        .expect("filter RUST");
    assert_eq!(hits.len(), 2, "case-insensitive: RUST matches 2 (SC-002)");
}

#[tokio::test]
async fn non_matching_filter_is_an_empty_result_not_an_error() {
    let repo = InMemoryCourseRepo::new();
    for c in &fixture() {
        repo.create_for_test(c.clone()).await.expect("seed");
    }
    let hits = repo
        .list_for(ORG_A, Some("nonexistent-course-xyz"))
        .await
        .expect("no error on empty match");
    assert!(hits.is_empty(), "empty result, not an error (US1.3)");
}

#[tokio::test]
async fn empty_org_and_empty_filter() {
    let repo = InMemoryCourseRepo::new();
    // Empty org: empty list, never an error.
    let none = repo.list_for("org-unknown", None).await.expect("empty org");
    assert!(none.is_empty(), "empty org → empty list, not an error");
    // Empty filter = full list.
    for c in &fixture() {
        repo.create_for_test(c.clone()).await.expect("seed");
    }
    let all = repo.list_for(ORG_A, Some("")).await.expect("empty filter");
    assert_eq!(all.len(), 4, "empty search term = full list");
}
