//! Robustness (spec 003 edge cases): last-write-wins under concurrent updates,
//! injection-shaped input treated as plain text.

use catalog::repo::{CourseRepo, InMemoryCourseRepo};
use contracts::Course;

#[tokio::test]
async fn concurrent_updates_last_write_wins_without_corruption() {
    let repo = InMemoryCourseRepo::new();
    repo.create_for_test(Course {
        id: "c1".into(),
        title: "base".into(),
        description: String::new(),
        organization_id: "org".into(),
    })
    .await
    .expect("seed");
    // Two concurrent updates (the double serializes on its mutex — the
    // last-write-wins guarantee is per-value, not per-ordering).
    let r1 = repo.update("c1", "org", Some("writer-one"), None);
    let r2 = repo.update("c1", "org", Some("writer-two"), None);
    let (r1, r2) = tokio::join!(r1, r2);
    assert!(r1.is_ok() && r2.is_ok(), "both updates succeed (no 500)");
    let final_ = repo.get("c1", "org").await.expect("read back");
    assert!(
        final_.title == "writer-one" || final_.title == "writer-two",
        "final value is one of the two writers: {}",
        final_.title
    );
}

#[tokio::test]
async fn injection_shaped_search_terms_are_plain_text() {
    let repo = InMemoryCourseRepo::new();
    repo.create_for_test(Course {
        id: "c1".into(),
        title: "Rust for LMS".into(),
        description: String::new(),
        organization_id: "org".into(),
    })
    .await
    .expect("seed");
    for term in ["'; DROP TABLE courses;--", "../../etc/passwd", "%\u{1}%00"] {
        let res = repo.list_for("org", Some(term)).await;
        assert!(res.is_ok(), "no error for injection-shaped term {term:?}");
        assert!(res.unwrap().is_empty(), "no match, no error for {term:?}");
    }
}

#[tokio::test]
async fn like_meta_characters_are_literal_in_the_double() {
    // CR-2b semantics pin: % and _ are NOT wildcards (the real engine is
    // aligned to this via the ESCAPE clause — see TiberiusCourseRepo::list_for).
    let repo = InMemoryCourseRepo::new();
    repo.create_for_test(Course {
        id: "c1".into(),
        title: "aXb".into(),
        description: String::new(),
        organization_id: "org".into(),
    })
    .await
    .expect("seed");
    let r = repo.list_for("org", Some("a_b")).await.expect("no error");
    assert!(r.is_empty(), "_ must not wildcard-match: {r:?}");
    let r = repo.list_for("org", Some("a%b")).await.expect("no error");
    assert!(r.is_empty(), "% must not wildcard-match: {r:?}");
}
