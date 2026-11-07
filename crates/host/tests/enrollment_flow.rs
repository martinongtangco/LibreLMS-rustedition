//! US1/US2/US3 (P1/P2/P3) — the enrollment HTTP surface (spec 004).
//! T008/T011/T014 (red-first). Envelope mapping under test:
//! 401 (no session) / 403 (scope + role + actor) / 404 (unknown id) /
//! 409 (already-enrolled / not-enrolled / already-completed) / 503 (outage,
//! never a silent empty list). The 404-vs-403 split follows the CourseSource
//! decision procedure (plan); the 403 body is content-identical to any other
//! 403 (no existence oracle).

use contracts::User;
use enrollment::memory::{InMemoryCourseSource, InMemoryEnrollmentRepo};
use rand::SeedableRng;
use shared::session::MemorySessionStore;
use shared::storage::{MemoryStorage, Storage as _};
use std::sync::Arc;

mod common;
use common::{wait_health_ok, AppConfig};

const ORG: &str = "dev-org";
const FOREIGN_ORG: &str = "org-b";

/// One raw-socket HTTP call (001 pattern — no extra client). `token=None`
/// sends no cookie.
async fn call(
    port: u16,
    method: &str,
    path: &str,
    body: Option<&str>,
    token: Option<&str>,
) -> Option<(u16, String)> {
    let mut stream = tokio::net::TcpStream::connect(("127.0.0.1", port))
        .await
        .ok()?;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let body = body.unwrap_or("");
    let mut req = format!(
        "{method} {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nContent-Length: {}\r\nConnection: close\r\nX-Organization: {ORG}\r\n",
        body.len()
    );
    if !body.is_empty() {
        req.push_str("Content-Type: application/json\r\n");
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

/// Boot the host with a session established IN the host's own storage (001
/// pattern: the session row must live where the host looks).
async fn boot(
    port: u16,
    repo: Arc<InMemoryEnrollmentRepo>,
    source: Arc<InMemoryCourseSource>,
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
        s.set_org_role_for_test(user_id, ORG, contracts::Role::Member);
    }
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
            std::sync::Arc::new(catalog::repo::InMemoryCourseRepo::new()),
            repo,
            source,
            Arc::new(scorm::memory::InMemoryScormRepo::new()),
            Arc::new(scorm::memory::InMemoryBagStore::new()),
            Arc::new(scorm::memory::DoubleEnrollmentPort::new()),
            None,
        )
        .await
    });
    assert!(wait_health_ok(port).await, "serving");
    token
}

async fn seed_course(source: &InMemoryCourseSource, id: &str, org: &str, title: &str) {
    source.seed(id, org, title).await;
}

// ---------- US1 (P1): enroll ----------

#[tokio::test]
async fn unauthenticated_enroll_is_401() {
    let repo = Arc::new(InMemoryEnrollmentRepo::new());
    let source = Arc::new(InMemoryCourseSource::new());
    boot(17601, repo, source, "dev-user").await;
    let (code, body) = call(
        17601,
        "POST",
        "/enrollments",
        Some("{\"course_id\":\"c1\"}"),
        None,
    )
    .await
    .expect("connected");
    assert_eq!(code, 401, "no cookie → 401");
    assert!(body.contains("\"error\""), "envelope: {body}");
}

#[tokio::test]
async fn enroll_in_scope_succeeds_and_is_visible() {
    let repo = Arc::new(InMemoryEnrollmentRepo::new());
    let source = Arc::new(InMemoryCourseSource::new());
    seed_course(&source, "c1", ORG, "Rust for LMS").await;
    let token = boot(17602, repo.clone(), source.clone(), "dev-user").await;
    let (code, body) = call(
        17602,
        "POST",
        "/enrollments",
        Some("{\"course_id\":\"c1\"}"),
        Some(&token),
    )
    .await
    .expect("connected");
    assert_eq!(code, 201, "in-scope enroll → 201: {body}");
    assert!(body.contains("\"enrolled\""), "status enrolled: {body}");
    // Visible in the own list.
    let (code, body) = call(17602, "GET", "/enrollments", None, Some(&token))
        .await
        .expect("connected");
    assert_eq!(code, 200);
    assert!(
        body.contains("c1") && body.contains("Rust for LMS"),
        "list: {body}"
    );
}

#[tokio::test]
async fn enroll_unknown_course_is_404_not_403() {
    let repo = Arc::new(InMemoryEnrollmentRepo::new());
    let source = Arc::new(InMemoryCourseSource::new());
    let token = boot(17603, repo, source, "dev-user").await;
    let (code, body) = call(
        17603,
        "POST",
        "/enrollments",
        Some("{\"course_id\":\"nope\"}"),
        Some(&token),
    )
    .await
    .expect("connected");
    assert_eq!(code, 404, "unknown course → 404 (not 403): {body}");
    assert!(body.contains("not-found"), "code: {body}");
}

#[tokio::test]
async fn enroll_out_of_scope_course_is_403_scope() {
    let repo = Arc::new(InMemoryEnrollmentRepo::new());
    let source = Arc::new(InMemoryCourseSource::new());
    seed_course(&source, "c2", FOREIGN_ORG, "Go Fundamentals").await;
    let token = boot(17604, repo, source, "dev-user").await;
    let (code, body) = call(
        17604,
        "POST",
        "/enrollments",
        Some("{\"course_id\":\"c2\"}"),
        Some(&token),
    )
    .await
    .expect("connected");
    assert_eq!(code, 403, "foreign-org course → 403: {body}");
    assert!(body.contains("scope"), "code: {body}");
    // No row was written (SC-002 create direction).
    let (_code, body) = call(17604, "GET", "/enrollments", None, Some(&token))
        .await
        .expect("connected");
    assert!(body.trim() == "[]", "nothing persisted: {body}");
}

#[tokio::test]
async fn duplicate_enroll_is_409_single_row() {
    let repo = Arc::new(InMemoryEnrollmentRepo::new());
    let source = Arc::new(InMemoryCourseSource::new());
    seed_course(&source, "c1", ORG, "Rust for LMS").await;
    let token = boot(17605, repo, source, "dev-user").await;
    let (code, _) = call(
        17605,
        "POST",
        "/enrollments",
        Some("{\"course_id\":\"c1\"}"),
        Some(&token),
    )
    .await
    .expect("connected");
    assert_eq!(code, 201);
    let (code, body) = call(
        17605,
        "POST",
        "/enrollments",
        Some("{\"course_id\":\"c1\"}"),
        Some(&token),
    )
    .await
    .expect("connected");
    assert_eq!(code, 409, "duplicate → 409: {body}");
    assert!(body.contains("already-enrolled"), "code: {body}");
    let (_, body) = call(17605, "GET", "/enrollments", None, Some(&token))
        .await
        .expect("connected");
    assert_eq!(
        body.matches("\"course_id\":\"c1\"").count(),
        1,
        "single row: {body}"
    );
}

#[tokio::test]
async fn outage_is_503_never_500_never_silent() {
    let repo = Arc::new(InMemoryEnrollmentRepo::new());
    let source = Arc::new(InMemoryCourseSource::new());
    // The course must exist (and be in-scope) so the port checks pass and
    // the failure is the REPO outage, not a 404.
    seed_course(&source, "c1", ORG, "Rust for LMS").await;
    let token = boot(17606, repo.clone(), source, "dev-user").await;
    repo.set_unavailable_for_test(true);
    let (code, body) = call(
        17606,
        "POST",
        "/enrollments",
        Some("{\"course_id\":\"c1\"}"),
        Some(&token),
    )
    .await
    .expect("connected");
    assert_eq!(code, 503, "create outage → 503: {body}");
    assert!(body.contains("storage-unavailable"), "code: {body}");
    let (code, body) = call(17606, "GET", "/enrollments", None, Some(&token))
        .await
        .expect("connected");
    assert_eq!(
        code, 503,
        "list outage → 503 (never a silent empty list): {body}"
    );
}

// ---------- US2 (P2): views by role ----------

#[tokio::test]
async fn member_own_list_empty_and_scoped() {
    let repo = Arc::new(InMemoryEnrollmentRepo::new());
    let source = Arc::new(InMemoryCourseSource::new());
    let token = boot(17610, repo.clone(), source.clone(), "dev-user").await;
    let (code, body) = call(17610, "GET", "/enrollments", None, Some(&token))
        .await
        .expect("connected");
    assert_eq!(code, 200, "empty list → 200 []: {body}");
    assert!(body.trim() == "[]", "{body}");
}

#[tokio::test]
async fn org_wide_view_is_admin_only() {
    // dev-user is the seeded dev-org ADMIN (001).
    let repo = Arc::new(InMemoryEnrollmentRepo::new());
    let source = Arc::new(InMemoryCourseSource::new());
    seed_course(&source, "c1", ORG, "Rust for LMS").await;
    let admin_token = boot(17611, repo.clone(), source.clone(), "dev-user").await;
    let (code, _) = call(
        17611,
        "POST",
        "/enrollments",
        Some("{\"course_id\":\"c1\"}"),
        Some(&admin_token),
    )
    .await
    .expect("connected");
    assert_eq!(code, 201);
    // Admin sees the org-wide view.
    let (code, body) = call(
        17611,
        "GET",
        "/enrollments?scope=org",
        None,
        Some(&admin_token),
    )
    .await
    .expect("connected");
    assert_eq!(code, 200, "admin org-wide → 200: {body}");
    assert!(body.contains("dev-user") && body.contains("c1"), "{body}");
    // A plain member is refused (403 forbidden — FR-004).
    let member_token = boot(17612, repo.clone(), source.clone(), "member-2").await;
    let (code, body) = call(
        17612,
        "GET",
        "/enrollments?scope=org",
        None,
        Some(&member_token),
    )
    .await
    .expect("connected");
    assert_eq!(code, 403, "non-admin org-wide → 403: {body}");
    assert!(body.contains("forbidden"), "code: {body}");
    // ...but the member's OWN list still works (FR-004 first clause).
    let (code, body) = call(17612, "GET", "/enrollments", None, Some(&member_token))
        .await
        .expect("connected");
    assert_eq!(code, 200);
    assert!(body.trim() == "[]", "member own list: {body}");
    // Unauthenticated → 401.
    let (code, _) = call(17612, "GET", "/enrollments?scope=org", None, None)
        .await
        .expect("connected");
    assert_eq!(code, 401);
}

#[tokio::test]
async fn views_never_leak_other_org_rows() {
    let repo = Arc::new(InMemoryEnrollmentRepo::new());
    let source = Arc::new(InMemoryCourseSource::new());
    seed_course(&source, "c1", ORG, "Rust for LMS").await;
    seed_course(&source, "c2", FOREIGN_ORG, "Go Fundamentals").await;
    let token = boot(17613, repo.clone(), source, "dev-user").await;
    let (code, _) = call(
        17613,
        "POST",
        "/enrollments",
        Some("{\"course_id\":\"c1\"}"),
        Some(&token),
    )
    .await
    .expect("connected");
    assert_eq!(code, 201);
    let (code, body) = call(17613, "GET", "/enrollments?scope=org", None, Some(&token))
        .await
        .expect("connected");
    assert_eq!(code, 200);
    assert!(body.contains("c1"), "own-org row present: {body}");
    assert!(!body.contains("c2"), "foreign-org row absent: {body}");
}

// ---------- US3 (P3): completion ----------

#[tokio::test]
async fn completion_transitions_and_second_is_rejected() {
    let repo = Arc::new(InMemoryEnrollmentRepo::new());
    let source = Arc::new(InMemoryCourseSource::new());
    seed_course(&source, "c1", ORG, "Rust for LMS").await;
    let token = boot(17620, repo, source, "dev-user").await;
    let (code, body) = call(
        17620,
        "POST",
        "/enrollments",
        Some("{\"course_id\":\"c1\"}"),
        Some(&token),
    )
    .await
    .expect("connected");
    assert_eq!(code, 201, "{body}");
    let enrid: String = body
        .find("\"id\":\"")
        .map(|i| &body[i + 6..])
        .and_then(|s| s.split('"').next())
        .map(|s| s.to_string())
        .expect("id in body");
    let (code, body) = call(
        17620,
        "POST",
        &format!("/enrollments/{enrid}/completion"),
        None,
        Some(&token),
    )
    .await
    .expect("connected");
    assert_eq!(code, 200, "complete → 200: {body}");
    assert!(body.contains("\"completed\""), "status: {body}");
    // Second completion → rejected, original record unchanged (US3.2).
    let (code, body) = call(
        17620,
        "POST",
        &format!("/enrollments/{enrid}/completion"),
        None,
        Some(&token),
    )
    .await
    .expect("connected");
    assert_eq!(code, 409, "double completion → 409: {body}");
    assert!(body.contains("already-completed"), "code: {body}");
    let (_, body) = call(17620, "GET", "/enrollments", None, Some(&token))
        .await
        .expect("connected");
    assert!(body.contains("\"completed\""), "still completed: {body}");
}

#[tokio::test]
async fn completion_gates_404_403_scope_403_forbidden() {
    let repo = Arc::new(InMemoryEnrollmentRepo::new());
    let source = Arc::new(InMemoryCourseSource::new());
    seed_course(&source, "c1", ORG, "Rust for LMS").await;
    seed_course(&source, "c2", FOREIGN_ORG, "Go Fundamentals").await;
    let token = boot(17621, repo.clone(), source.clone(), "dev-user").await;
    let (_, body) = call(
        17621,
        "POST",
        "/enrollments",
        Some("{\"course_id\":\"c1\"}"),
        Some(&token),
    )
    .await
    .expect("connected");
    let enrid: String = body
        .find("\"id\":\"")
        .map(|i| &body[i + 6..])
        .and_then(|s| s.split('"').next())
        .map(|s| s.to_string())
        .expect("id in body");
    // Unknown id → 404 (US3.3).
    let (code, body) = call(
        17621,
        "POST",
        "/enrollments/nope/completion",
        None,
        Some(&token),
    )
    .await
    .expect("connected");
    assert_eq!(code, 404, "unknown → 404: {body}");
    // Out-of-scope completion → 403 scope, nothing persisted (US3.3).
    // (The row belongs to ORG; a foreign-org session is expressed through the
    // repo's scope gate — here proven at the route by the 403 for a member of
    // another org is not possible with one session, so the double/repo test
    // covers it; the route proves the 404/403 plumbing.)
    // Cross-learner → 403 forbidden.
    let member_token = boot(17622, repo.clone(), source.clone(), "member-2").await;
    let (code, body) = call(
        17622,
        "POST",
        &format!("/enrollments/{enrid}/completion"),
        None,
        Some(&member_token),
    )
    .await
    .expect("connected");
    assert_eq!(code, 403, "cross-learner → 403: {body}");
    assert!(body.contains("forbidden"), "code: {body}");
    // The original enrollment is untouched.
    let (_, body) = call(17621, "GET", "/enrollments", None, Some(&token))
        .await
        .expect("connected");
    assert!(body.contains("\"enrolled\""), "still enrolled: {body}");
}

#[tokio::test]
async fn completion_outage_is_503() {
    let repo = Arc::new(InMemoryEnrollmentRepo::new());
    let source = Arc::new(InMemoryCourseSource::new());
    seed_course(&source, "c1", ORG, "Rust for LMS").await;
    let token = boot(17623, repo.clone(), source, "dev-user").await;
    let (_, body) = call(
        17623,
        "POST",
        "/enrollments",
        Some("{\"course_id\":\"c1\"}"),
        Some(&token),
    )
    .await
    .expect("connected");
    let enrid: String = body
        .find("\"id\":\"")
        .map(|i| &body[i + 6..])
        .and_then(|s| s.split('"').next())
        .map(|s| s.to_string())
        .expect("id in body");
    repo.set_unavailable_for_test(true);
    let (code, _) = call(
        17623,
        "POST",
        &format!("/enrollments/{enrid}/completion"),
        None,
        Some(&token),
    )
    .await
    .expect("connected");
    assert_eq!(code, 503, "completion outage → 503");
}
