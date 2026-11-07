//! US3 (P3): the FR-012 validation battery (T011, red-first).
//!
//! App-layer validation FIRST (the engine CHECK/UNIQUE is the belt — CR-1/
//! CR-2); every rejection names the field; a 503 never masquerades as a
//! silent empty list (FR-011).

use contracts::Role;
use management::repo::{
    InMemoryManagementRepo, ManagementError, ManagementRepo, parse_role,
};

async fn create(
    repo: &InMemoryManagementRepo,
    email: &str,
    name: &str,
) -> Result<management::repo::ManagedUser, ManagementError> {
    repo.create_user("org-v", email, name, &Role::Member).await
}

// ---------- field boundaries (FR-012) ----------

#[tokio::test]
async fn email_boundary_320_ok_321_rejected_naming_the_field() {
    let repo = InMemoryManagementRepo::new();
    let tail = "@e.com"; // 6 chars
    let body = "a".repeat(320 - tail.len());
    let email320 = format!("{body}{tail}");
    assert_eq!(email320.len(), 320);
    create(&repo, &email320, "Boundary")
        .await
        .expect("320-char email is valid");
    let body321 = "a".repeat(321 - tail.len());
    let email321 = format!("{body321}{tail}");
    let d = create(&repo, &email321, "Boundary").await;
    assert!(
        matches!(
            d,
            Err(ManagementError::Validation { ref field, .. }) if &**field == "email"
        ),
        "321-char email → Validation{{field: email}}, got {d:?}"
    );
}

#[tokio::test]
async fn empty_email_rejected_naming_the_field() {
    let repo = InMemoryManagementRepo::new();
    let d = create(&repo, "", "N").await;
    assert!(
        matches!(
            d,
            Err(ManagementError::Validation { ref field, .. }) if &**field == "email"
        ),
        "empty email → Validation{{field: email}}, got {d:?}"
    );
}

#[tokio::test]
async fn name_boundary_200_ok_201_rejected_naming_the_field() {
    let repo = InMemoryManagementRepo::new();
    create(&repo, "nb@e.com", &"n".repeat(200))
        .await
        .expect("200-char name is valid");
    let d = create(&repo, "nb2@e.com", &"n".repeat(201)).await;
    assert!(
        matches!(
            d,
            Err(ManagementError::Validation { ref field, .. }) if &**field == "name"
        ),
        "201-char name → Validation{{field: name}}, got {d:?}"
    );
}

#[tokio::test]
async fn empty_name_rejected_naming_the_field() {
    let repo = InMemoryManagementRepo::new();
    let d = create(&repo, "en@e.com", "").await;
    assert!(
        matches!(
            d,
            Err(ManagementError::Validation { ref field, .. }) if &**field == "name"
        ),
        "empty name → Validation{{field: name}}, got {d:?}"
    );
}

#[tokio::test]
async fn org_name_boundary_200_ok_201_rejected_naming_the_field() {
    let repo = InMemoryManagementRepo::new();
    repo.create_org(&"o".repeat(200))
        .await
        .expect("200-char org name is valid");
    let d = repo.create_org(&"o".repeat(201)).await;
    assert!(
        matches!(
            d,
            Err(ManagementError::Validation { ref field, .. }) if &**field == "name"
        ),
        "201-char org name → Validation{{field: name}}, got {d:?}"
    );
}

#[tokio::test]
async fn org_name_empty_rejected_naming_the_field() {
    let repo = InMemoryManagementRepo::new();
    let d = repo.create_org("").await;
    assert!(
        matches!(
            d,
            Err(ManagementError::Validation { ref field, .. }) if &**field == "name"
        ),
        "empty org name → Validation{{field: name}}, got {d:?}"
    );
}

// ---------- role (CR-1) ----------

#[tokio::test]
async fn role_enum_is_closed_junk_never_silently_coerced() {
    // CR-1: role ∈ {admin, member}. The wire edge (parse_role) rejects
    // everything else; the repo never sees a third value (the enum is
    // closed), so the engine CHECK is pure belt-and-braces.
    assert!(parse_role("admin").is_ok());
    assert!(parse_role("member").is_ok());
    for junk in ["superadmin", "ADMIN", "Admin", "", "admin ", "root", "x"] {
        assert!(
            parse_role(junk).is_err(),
            "junk role {junk:?} must not parse (CR-1)"
        );
    }
}

// ---------- injection-shaped values are plain text (FR-012) ----------

#[tokio::test]
async fn injection_shaped_email_stored_and_compared_as_plain_text() {
    // The app layer stores/compares email as plain text (the engine's
    // parameterized SQL is the belt — T013(d) proves it on the real
    // engine). Injection shapes round-trip byte-identical.
    let repo = InMemoryManagementRepo::new();
    let evil = "robert'); DROP TABLE users;--@e.com";
    let u = create(&repo, evil, "Robert").await.expect("created");
    assert_eq!(u.user.email, evil, "stored byte-identical");
    let listed = repo.list_users("org-v").await.unwrap();
    assert!(
        listed.iter().any(|m| m.user.email == evil),
        "compared as plain text in the list"
    );
    // The duplicate check is on the PLAIN-TEXT equality: the same string is
    // the same user (idempotent upsert), never a SQL injection.
    let again = create(&repo, evil, "Robert").await.expect("idempotent");
    assert_eq!(again.user.id, u.user.id, "same plain-text email → same user");
}

#[tokio::test]
async fn injection_shaped_name_and_org_name_stored_as_plain_text() {
    let repo = InMemoryManagementRepo::new();
    let evil_name = "Robert \"Robert\"; EXEC sp_executesql N'delete';";
    create(&repo, "inj@e.com", evil_name)
        .await
        .expect("name stored");
    let evil_org = "O'; DELETE FROM dbo.organizations WHERE 1=1--;";
    let org = repo.create_org(evil_org).await.expect("org stored");
    assert_eq!(org.name, evil_org, "org name byte-identical");
    let listed = repo.list_orgs().await.unwrap();
    assert!(
        listed.iter().any(|(o, _)| o.name == evil_org),
        "org name compared as plain text in the list"
    );
}

// ---------- 503 never a silent empty list (FR-011) ----------

#[tokio::test]
async fn fault_injection_is_unavailable_never_silent_empty() {
    let repo = InMemoryManagementRepo::new();
    repo.create_user("org-f", "f@e.com", "F", &Role::Member)
        .await
        .expect("seeded while up");
    repo.set_unavailable_for_test(true);
    let users = repo.list_users("org-f").await;
    assert!(
        matches!(users, Err(ManagementError::Unavailable)),
        "outage → Unavailable (never a silent empty list), got {users:?}"
    );
    let orgs = repo.list_orgs().await;
    assert!(
        matches!(orgs, Err(ManagementError::Unavailable)),
        "outage → Unavailable (never a silent empty list), got {orgs:?}"
    );
    let d = repo.create_user("org-f", "g@e.com", "G", &Role::Member).await;
    assert!(
        matches!(d, Err(ManagementError::Unavailable)),
        "outage → Unavailable (never a silent write), got {d:?}"
    );
    let r = repo.rename_org("some-org", "New").await;
    assert!(
        matches!(r, Err(ManagementError::Unavailable)),
        "outage → Unavailable (never a silent success), got {r:?}"
    );
}
