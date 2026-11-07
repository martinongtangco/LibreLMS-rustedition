//! US1 (P1) — the learner-management HTTP surface (spec 006, T003 red-first).
//! RED at commit time: the /admin/users routes land in T006; every assertion
//! below therefore fails (route miss → 404) until then.
//!
//! Envelope mapping under test (spec 006 FR-011, the stable-code table):
//! 401 `unauthenticated` (no session) / 403 `forbidden` (non-admin, the
//! role-gate — BEFORE any id check) / 403 `scope` (cross-org user ops,
//! content-identical to any other 403 — no existence oracle) /
//! 409 `duplicate-email` (engine UNIQUE is the final arbiter) /
//! 400 `validation` (field named in the body — incl. role, CR-1) /
//! 503 `storage-unavailable` (outage; never a 500, never a silent empty list).

use contracts::User;
use management::repo::InMemoryManagementRepo;
use rand::SeedableRng;
use shared::session::MemorySessionStore;
use shared::storage::{MemoryStorage, Storage as _};
use std::sync::Arc;

mod common;
use common::{wait_health_ok, AppConfig};

const ORG: &str = "dev-org";
const FOREIGN_ORG: &str = "org-b";

/// One raw-socket HTTP call (001 pattern — no extra client). `token=None`
/// sends no cookie. `org` is the X-Organization header (None → omitted).
async fn call(
    port: u16,
    method: &str,
    path: &str,
    body: Option<&str>,
    token: Option<&str>,
    org: Option<&str>,
) -> Option<(u16, String)> {
    let mut stream = tokio::net::TcpStream::connect(("127.0.0.1", port))
        .await
        .ok()?;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let body = body.unwrap_or("");
    let mut req = format!(
        "{method} {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nContent-Length: {}\r\nConnection: close\r\n",
        body.len()
    );
    if !body.is_empty() {
        req.push_str("Content-Type: application/json\r\n");
    }
    if let Some(o) = org {
        req.push_str(&format!("X-Organization: {o}\r\n"));
    }
    if let Some(t) = token {
        req.push_str(&format!("Cookie: session={t}\r\n"));
    }
    req.push_str("\r\n");
    req.push_str(body);
    stream.write_all(req.as_bytes()).await.ok()?;
    let mut buf = Vec::new();
    stream.read_to_end(&mut buf).await.ok()?;
    let text = String::from_utf8_lossy(&buf).to_string();
    let head = text.lines().next()?.to_string();
    let code: u16 = head.split_whitespace().nth(1)?.parse().ok()?;
    Some((
        code,
        text.split_once("\r\n\r\n")
            .map(|(_, b)| b.to_string())
            .unwrap_or_default(),
    ))
}

fn error_code(body: &str) -> Option<String> {
    let v: serde_json::Value = serde_json::from_str(body).ok()?;
    v.get("error")?.as_str()?.to_string().into()
}

/// Boot the host with a session established IN the host's own storage (001
/// pattern). `role` is the session user's role in ORG (the 006 role-gate
/// under test: admin vs member). RED: the management repo is not yet mounted
/// (T006); the parameter lands with the one-line-mount convention.
async fn boot(
    port: u16,
    _mgmt: Arc<InMemoryManagementRepo>,
    role: &contracts::Role,
    user_id: &str,
) -> String {
    let s = MemoryStorage::new();
    s.run_migrations(shared::migrate::MIGRATIONS)
        .await
        .expect("migrate");
    if user_id != "dev-user" {
        s.create_user_for_test(User {
            id: user_id.into(),
            email: format!("{user_id}@example.com"),
            name: user_id.into(),
            theme: "default".into(),
        });
    }
    s.set_org_role_for_test(user_id, ORG, *role);
    let storage: Arc<dyn shared::storage::Storage> = Arc::new(s);
    let store: Arc<dyn shared::session::SessionStore> = Arc::new(MemorySessionStore::new());
    let mut rng = rand::rngs::StdRng::seed_from_u64(port as u64);
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64;
    let (token, _row) =
        shared::auth::establish(&*storage, &mut rng, user_id, ORG, now, 3_600, 72_000)
            .await
            .expect("establish");
    tokio::spawn(async move {
        host::run_with_stop(
            AppConfig::valid(port),
            storage,
            store,
            Arc::new(catalog::repo::InMemoryCourseRepo::new()),
            Arc::new(enrollment::memory::InMemoryEnrollmentRepo::new()),
            Arc::new(enrollment::memory::InMemoryCourseSource::new()),
            Arc::new(scorm::memory::InMemoryScormRepo::new()),
            Arc::new(scorm::memory::InMemoryBagStore::new()),
            Arc::new(scorm::memory::DoubleEnrollmentPort::new()),
            _mgmt,
            None,
        )
        .await
    });
    assert!(wait_health_ok(port).await, "serving");
    token
}

// ---------- US1 (P1): the /admin/users envelope matrix ----------

#[tokio::test]
async fn unauthenticated_user_create_is_401() {
    let mgmt = Arc::new(InMemoryManagementRepo::new());
    boot(17701, mgmt, &contracts::Role::Admin, "dev-user").await;
    let (code, body) = call(
        17701,
        "POST",
        "/admin/users",
        Some(r#"{"email":"a@e.com","name":"A","role":"member"}"#),
        None,
        Some(ORG),
    )
    .await
    .expect("connected");
    assert_eq!(code, 401, "no cookie → 401, got {code}");
    assert_eq!(error_code(&body).as_deref(), Some("unauthenticated"));
}

#[tokio::test]
async fn admin_create_user_in_scope_is_201() {
    let mgmt = Arc::new(InMemoryManagementRepo::new());
    let token = boot(17702, mgmt, &contracts::Role::Admin, "admin-1").await;
    let (code, body) = call(
        17702,
        "POST",
        "/admin/users",
        Some(r#"{"email":"newlearner@e.com","name":"New Learner","role":"member"}"#),
        Some(&token),
        Some(ORG),
    )
    .await
    .expect("connected");
    assert_eq!(
        code, 201,
        "in-scope admin create → 201, got {code} ({body})"
    );
}

#[tokio::test]
async fn member_create_user_is_403_forbidden_role_gate() {
    let mgmt = Arc::new(InMemoryManagementRepo::new());
    let token = boot(17703, mgmt, &contracts::Role::Member, "member-1").await;
    let (code, body) = call(
        17703,
        "POST",
        "/admin/users",
        Some(r#"{"email":"x@e.com","name":"X","role":"member"}"#),
        Some(&token),
        Some(ORG),
    )
    .await
    .expect("connected");
    assert_eq!(
        code, 403,
        "member → 403 role-gate (BEFORE any id check), got {code}"
    );
    assert_eq!(error_code(&body).as_deref(), Some("forbidden"));
}

#[tokio::test]
async fn cross_org_user_operation_is_403_scope() {
    let mgmt = Arc::new(InMemoryManagementRepo::new());
    let token = boot(17704, mgmt, &contracts::Role::Admin, "admin-2").await;
    let (code, body) = call(
        17704,
        "POST",
        "/admin/users",
        Some(r#"{"email":"y@e.com","name":"Y","role":"member"}"#),
        Some(&token),
        Some(FOREIGN_ORG),
    )
    .await
    .expect("connected");
    assert_eq!(code, 403, "cross-org → 403 scope, got {code}");
    assert_eq!(error_code(&body).as_deref(), Some("scope"));
}

#[tokio::test]
async fn duplicate_email_is_409() {
    let mgmt = Arc::new(InMemoryManagementRepo::new());
    let token = boot(17705, mgmt, &contracts::Role::Admin, "admin-3").await;
    let create = r#"{"email":"dup@e.com","name":"D","role":"member"}"#;
    let (first, _) = call(
        17705,
        "POST",
        "/admin/users",
        Some(create),
        Some(&token),
        Some(ORG),
    )
    .await
    .expect("connected");
    assert_eq!(first, 201, "first create → 201, got {first}");
    // NOTE-2: same email AND same name → idempotent upsert (201, the user row
    // is not re-created, the membership is upserted) — NOT a 409. The 409
    // is the identity conflict: same email, DIFFERENT name.
    let conflict = r#"{"email":"dup@e.com","name":"CONFLICT","role":"member"}"#;
    let (code, body) = call(
        17705,
        "POST",
        "/admin/users",
        Some(conflict),
        Some(&token),
        Some(ORG),
    )
    .await
    .expect("connected");
    assert_eq!(code, 409, "duplicate email → 409, got {code} ({body})");
    assert_eq!(error_code(&body).as_deref(), Some("duplicate-email"));
}

#[tokio::test]
async fn junk_role_is_400_naming_the_field_cr1() {
    // CR-1: role ∈ {admin, member} — the CHECK constraint is the belt; the
    // 400 names the field in the body (FR-012).
    let mgmt = Arc::new(InMemoryManagementRepo::new());
    let token = boot(17706, mgmt, &contracts::Role::Admin, "admin-4").await;
    let (code, body) = call(
        17706,
        "POST",
        "/admin/users",
        Some(r#"{"email":"z@e.com","name":"Z","role":"superadmin"}"#),
        Some(&token),
        Some(ORG),
    )
    .await
    .expect("connected");
    assert_eq!(code, 400, "junk role → 400, got {code} ({body})");
    assert_eq!(error_code(&body).as_deref(), Some("validation"));
    assert!(
        body.contains("role"),
        "400 body names the field, got {body}"
    );
}

#[tokio::test]
async fn re_role_persists_and_list_reflects_it() {
    let mgmt = Arc::new(InMemoryManagementRepo::new());
    let token = boot(17707, mgmt, &contracts::Role::Admin, "admin-5").await;
    let (c, b) = call(
        17707,
        "POST",
        "/admin/users",
        Some(r#"{"email":"rr@e.com","name":"RR","role":"admin"}"#),
        Some(&token),
        Some(ORG),
    )
    .await
    .expect("connected");
    assert_eq!(c, 201, "seed user created");
    // The user id is the 201 body's (T006 shape; T009: real extraction).
    // PUT /admin/users/{id}/role → 200 and GET /admin/users reflects it.
    let seed: serde_json::Value = serde_json::from_str(&b).ok().unwrap();
    let uid = seed.get("id").unwrap().as_str().unwrap().to_string();
    let (p, pb) = call(
        17707,
        "PUT",
        &format!("/admin/users/{uid}/role"),
        Some(r#"{"role":"member"}"#),
        Some(&token),
        Some(ORG),
    )
    .await
    .expect("connected");
    assert_eq!(p, 200, "re-role → 200, got {p} ({pb})");
    let (g, gb) = call(17707, "GET", "/admin/users", None, Some(&token), Some(ORG))
        .await
        .expect("connected");
    assert_eq!(g, 200, "list → 200, got {g}");
    assert!(
        gb.contains("\"member\""),
        "list reflects the re-role, got {gb}"
    );
}

#[tokio::test]
async fn remove_user_is_204_membership_gone_user_survives() {
    let mgmt = Arc::new(InMemoryManagementRepo::new());
    let token = boot(17708, mgmt, &contracts::Role::Admin, "admin-6").await;
    let (c, b) = call(
        17708,
        "POST",
        "/admin/users",
        Some(r#"{"email":"rm@e.com","name":"RM","role":"member"}"#),
        Some(&token),
        Some(ORG),
    )
    .await
    .expect("connected");
    assert_eq!(c, 201, "seed user created");
    let seed: serde_json::Value = serde_json::from_str(&b).ok().unwrap();
    let uid = seed.get("id").unwrap().as_str().unwrap().to_string();
    let (d, db) = call(
        17708,
        "DELETE",
        &format!("/admin/users/{uid}"),
        None,
        Some(&token),
        Some(ORG),
    )
    .await
    .expect("connected");
    assert_eq!(d, 204, "remove membership → 204, got {d} ({db})");
    // FR-004: the membership is gone — the user no longer lists in ORG.
    // (The user-row survival is the repo-battery assertion, T002.)
    let (g, gb) = call(17708, "GET", "/admin/users", None, Some(&token), Some(ORG))
        .await
        .expect("connected");
    assert_eq!(g, 200);
    assert!(
        !gb.contains("rm@e.com"),
        "removed membership not listed, got {gb}"
    );
}

#[tokio::test]
async fn storage_outage_is_503_never_500_never_silent() {
    let mgmt = Arc::new(InMemoryManagementRepo::new());
    let token = boot(17709, mgmt.clone(), &contracts::Role::Admin, "admin-7").await;
    mgmt.set_unavailable_for_test(true); // T006 mounts this repo
    let (code, body) = call(17709, "GET", "/admin/users", None, Some(&token), Some(ORG))
        .await
        .expect("connected");
    assert_eq!(
        code, 503,
        "outage → 503 (never 500, never a silent list), got {code} ({body})"
    );
    assert_eq!(error_code(&body).as_deref(), Some("storage-unavailable"));
}

// ---------- US2 (P2): the organization HTTP envelope matrix (T008) ----------

#[tokio::test]
async fn org_create_unauthenticated_is_401() {
    let mgmt = Arc::new(InMemoryManagementRepo::new());
    boot(17720, mgmt, &contracts::Role::Admin, "admin-401").await;
    let (code, _) = call(
        17720,
        "POST",
        "/admin/organizations",
        Some(r#"{"name":"X"}"#),
        None,
        None,
    )
    .await
    .expect("connected");
    assert_eq!(code, 401, "no cookie → 401, got {code}");
}

#[tokio::test]
async fn org_endpoints_member_is_403_forbidden_role_gate() {
    // The role gate fires BEFORE any id check (the 003 pattern): a member
    // with a VALID org id still gets 403 forbidden — never a 200/404.
    let mgmt = Arc::new(InMemoryManagementRepo::new());
    let token = boot(17721, mgmt, &contracts::Role::Member, "mem-org").await;
    let (code, _) = call(
        17721,
        "POST",
        "/admin/organizations",
        Some(r#"{"name":"X"}"#),
        Some(&token),
        Some(ORG),
    )
    .await
    .expect("connected");
    assert_eq!(code, 403, "member → 403 (role gate), got {code}");
}

#[tokio::test]
async fn org_create_is_201_shape_id_and_name_only() {
    let mgmt = Arc::new(InMemoryManagementRepo::new());
    let token = boot(17722, mgmt, &contracts::Role::Admin, "admin-org").await;
    let (code, body) = call(
        17722,
        "POST",
        "/admin/organizations",
        Some(r#"{"name":"Acme"}"#),
        Some(&token),
        Some(ORG),
    )
    .await
    .expect("connected");
    assert_eq!(code, 201, "org create → 201, got {code} ({body})");
    let v: serde_json::Value = serde_json::from_str(&body).ok().unwrap();
    assert!(v.get("id").is_some(), "201 body carries the id");
    assert_eq!(v.get("name").and_then(|x| x.as_str()), Some("Acme"));
}

#[tokio::test]
async fn org_list_includes_counts() {
    let mgmt = Arc::new(InMemoryManagementRepo::new());
    let token = boot(17723, mgmt, &contracts::Role::Admin, "admin-cnt").await;
    let (code, _) = call(
        17723,
        "POST",
        "/admin/organizations",
        Some(r#"{"name":"C1"}"#),
        Some(&token),
        Some(ORG),
    )
    .await
    .expect("connected");
    assert_eq!(code, 201);
    let (code, body) = call(
        17723,
        "GET",
        "/admin/organizations",
        None,
        Some(&token),
        Some(ORG),
    )
    .await
    .expect("connected");
    assert_eq!(code, 200, "list → 200, got {code}");
    let v: serde_json::Value = serde_json::from_str(&body).ok().unwrap();
    assert_eq!(v.as_array().unwrap().len(), 1, "one org listed");
}

#[tokio::test]
async fn org_rename_persists_and_unknown_is_404() {
    let mgmt = Arc::new(InMemoryManagementRepo::new());
    let token = boot(17724, mgmt, &contracts::Role::Admin, "admin-ren").await;
    let (_, cb) = call(
        17724,
        "POST",
        "/admin/organizations",
        Some(r#"{"name":"R1"}"#),
        Some(&token),
        Some(ORG),
    )
    .await
    .expect("connected");
    let id: serde_json::Value = serde_json::from_str(&cb).ok().unwrap();
    let id = id.get("id").unwrap().as_str().unwrap().to_string();
    let (code, _) = call(
        17724,
        "PUT",
        &format!("/admin/organizations/{id}"),
        Some(r#"{"name":"R2"}"#),
        Some(&token),
        Some(ORG),
    )
    .await
    .expect("connected");
    assert_eq!(code, 200, "rename → 200, got {code}");
    let (code, body) = call(
        17724,
        "PUT",
        "/admin/organizations/no-such-id",
        Some(r#"{"name":"R3"}"#),
        Some(&token),
        Some(ORG),
    )
    .await
    .expect("connected");
    assert_eq!(
        code, 404,
        "unknown org rename → 404 (CR-3 org-ids-only), got {code}"
    );
    assert_eq!(error_code(&body).as_deref(), Some("not-found"));
}

#[tokio::test]
async fn org_delete_nonempty_is_409_not_empty() {
    // FR-005: user ops target the SESSION's org only (a created org can only
    // be filled by its own admin), so the 409 proof targets the session's
    // org: the org row is ensured (test-only helper — 001's
    // create_user_for_test pattern), a membership is added via the API, and
    // the delete is refused with a stable code.
    let mgmt = Arc::new(InMemoryManagementRepo::new());
    let token = boot(17725, mgmt.clone(), &contracts::Role::Admin, "admin-del").await;
    mgmt.ensure_org_for_test(ORG, "SessionOrg");
    let (code, _) = call(
        17725,
        "POST",
        "/admin/users",
        Some(r#"{"email":"d@e.com","name":"D","role":"member"}"#),
        Some(&token),
        Some(ORG),
    )
    .await
    .expect("connected");
    assert_eq!(code, 201, "user create (fills the session org) → 201");
    let (code, body) = call(
        17725,
        "DELETE",
        &format!("/admin/organizations/{ORG}"),
        None,
        Some(&token),
        Some(ORG),
    )
    .await
    .expect("connected");
    assert_eq!(code, 409, "non-empty delete → 409, got {code}");
    assert_eq!(error_code(&body).as_deref(), Some("not-empty"));
}

#[tokio::test]
async fn org_delete_empty_is_204_then_404() {
    let mgmt = Arc::new(InMemoryManagementRepo::new());
    let token = boot(17726, mgmt, &contracts::Role::Admin, "admin-dele").await;
    let (_, cb) = call(
        17726,
        "POST",
        "/admin/organizations",
        Some(r#"{"name":"E1"}"#),
        Some(&token),
        Some(ORG),
    )
    .await
    .expect("connected");
    let id: serde_json::Value = serde_json::from_str(&cb).ok().unwrap();
    let id = id.get("id").unwrap().as_str().unwrap().to_string();
    let (code, _) = call(
        17726,
        "DELETE",
        &format!("/admin/organizations/{id}"),
        None,
        Some(&token),
        Some(ORG),
    )
    .await
    .expect("connected");
    assert_eq!(code, 204, "empty org delete → 204, got {code}");
    let (code, body) = call(
        17726,
        "DELETE",
        &format!("/admin/organizations/{id}"),
        None,
        Some(&token),
        Some(ORG),
    )
    .await
    .expect("connected");
    assert_eq!(code, 404, "deleted org → 404, got {code}");
    assert_eq!(error_code(&body).as_deref(), Some("not-found"));
}

#[tokio::test]
async fn org_endpoints_unavailable_is_503_never_500() {
    // Fault injection (the 003 pattern): a storage outage is a 503 with a
    // stable code — never a 500, never a silent empty list.
    let mgmt = Arc::new(InMemoryManagementRepo::new());
    mgmt.set_unavailable_for_test(true);
    let token = boot(17727, mgmt, &contracts::Role::Admin, "admin-down").await;
    let (code, body) = call(
        17727,
        "GET",
        "/admin/organizations",
        None,
        Some(&token),
        Some(ORG),
    )
    .await
    .expect("connected");
    assert_eq!(code, 503, "outage → 503, got {code}");
    assert_eq!(error_code(&body).as_deref(), Some("storage-unavailable"));
}
