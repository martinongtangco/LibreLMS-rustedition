//! US3 (P3) — scope & permission enforcement (spec 003 story 3, FR-003/FR-006).
//! Red-first: T006 makes these green.

use catalog::repo::{CatalogError, CourseRepo, InMemoryCourseRepo};
use contracts::Course;

const ORG_A: &str = "org-a";
const ORG_B: &str = "org-b";

async fn seeded() -> InMemoryCourseRepo {
    let repo = InMemoryCourseRepo::new();
    repo.create_for_test(Course {
        id: "a1".into(),
        title: "A one".into(),
        description: String::new(),
        organization_id: ORG_A.into(),
    })
    .await
    .expect("seed");
    repo.create_for_test(Course {
        id: "b1".into(),
        title: "B one".into(),
        description: String::new(),
        organization_id: ORG_B.into(),
    })
    .await
    .expect("seed");
    repo
}

#[tokio::test]
async fn out_of_scope_get_is_scope_never_not_found() {
    let repo = seeded().await;
    let e = repo.get("b1", ORG_A).await.unwrap_err();
    assert_eq!(
        e,
        CatalogError::Scope,
        "existing-but-foreign → Scope (403); no existence oracle (story 3.2)"
    );
}

#[tokio::test]
async fn out_of_scope_update_and_delete_are_scope() {
    let repo = seeded().await;
    assert_eq!(
        repo.update("b1", ORG_A, Some("hax"), None)
            .await
            .unwrap_err(),
        CatalogError::Scope,
        "cross-org update rejected"
    );
    assert_eq!(
        repo.delete("b1", ORG_A).await.unwrap_err(),
        CatalogError::Scope,
        "cross-org delete rejected"
    );
    // FR-003 both directions: org B cannot touch A either.
    assert_eq!(
        repo.update("a1", ORG_B, None, Some("hax"))
            .await
            .unwrap_err(),
        CatalogError::Scope
    );
}

#[tokio::test]
async fn unknown_id_is_plain_not_found() {
    let repo = seeded().await;
    assert_eq!(
        repo.get("nope", ORG_A).await.unwrap_err(),
        CatalogError::NotFound,
        "absent id → 404-class, plain"
    );
    assert_eq!(
        repo.delete("nope", ORG_A).await.unwrap_err(),
        CatalogError::NotFound
    );
}

#[tokio::test]
async fn create_lands_in_the_session_org_only() {
    let repo = seeded().await;
    let c = repo
        .create(ORG_B, "New B course", None)
        .await
        .expect("create in B");
    assert_eq!(
        c.organization_id, ORG_B,
        "create is scoped to the session org"
    );
    // ...and is invisible to A.
    let a = repo.list_for(ORG_A, None).await.expect("list A");
    assert!(
        a.iter().all(|c| c.organization_id == ORG_A),
        "B's new course never appears in A's list"
    );
}

#[tokio::test]
async fn storage_outage_is_unavailable_never_silent() {
    let repo = InMemoryCourseRepo::new();
    repo.set_unavailable_for_test(true);
    assert_eq!(
        repo.list_for(ORG_A, None).await.unwrap_err(),
        CatalogError::Unavailable,
        "outage → Unavailable (503); never a silent empty list (FR-008)"
    );
    assert_eq!(
        repo.get("a1", ORG_A).await.unwrap_err(),
        CatalogError::Unavailable
    );
}
