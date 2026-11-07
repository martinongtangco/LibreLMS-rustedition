//! US1+US2 (P1/P2) + error paths — the catalog HTTP surface (spec 003).
//! T007 (red-first) / T008 (routes wired). Envelope mapping under test:
//! 401 (no session) / 400 (field-named) / 403 (scope + role) / 404 / 503
//! (outage, never a silent empty list).

use catalog::repo::InMemoryCourseRepo;
use contracts::User;
use enrollment::memory::{InMemoryCourseSource, InMemoryEnrollmentRepo};
use rand::SeedableRng;
use shared::session::MemorySessionStore;
use shared::storage::{MemoryStorage, Storage as _};
use std::sync::Arc;

mod common;
use common::{wait_health_ok, AppConfig};

const ORG: &str = "dev-org";

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
    if body.is_empty() {
        // Content-Length: 0 already set; no body header needed.
    } else {
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
async fn boot_with_session(port: u16, repo: Arc<InMemoryCourseRepo>, user_id: &str) -> String {
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
            repo,
            Arc::new(InMemoryEnrollmentRepo::new()),
            Arc::new(InMemoryCourseSource::new()),
            None,
        )
        .await
    });
    assert!(wait_health_ok(port).await, "serving");
    token
}

#[tokio::test]
async fn unauthenticated_list_is_401() {
    let repo = Arc::new(InMemoryCourseRepo::new());
    boot_with_session(17501, repo, "dev-user").await;
    let (code, body) = call(17501, "GET", "/courses", None, None)
        .await
        .expect("connected");
    assert_eq!(code, 401, "no cookie → 401 (US1.4)");
    assert!(body.contains("\"error\""), "envelope: {body}");
}

#[tokio::test]
async fn list_and_filter_through_http() {
    let repo = Arc::new(InMemoryCourseRepo::new());
    let token = boot_with_session(17502, repo.clone(), "dev-user").await;
    for (id, title) in [("c1", "Rust for LMS"), ("c3", "Go Fundamentals")] {
        repo.create_for_test(contracts::Course {
            id: id.into(),
            title: title.into(),
            description: String::new(),
            organization_id: ORG.into(),
        })
        .await
        .expect("seed");
    }
    let (code, body) = call(17502, "GET", "/courses", None, Some(&token))
        .await
        .expect("connected");
    assert_eq!(code, 200, "in-scope list → 200");
    assert!(
        body.contains("Rust for LMS") && body.contains("Go Fundamentals"),
        "list: {body}"
    );
    let (code, body) = call(17502, "GET", "/courses?title=go", None, Some(&token))
        .await
        .expect("connected");
    assert_eq!(code, 200);
    assert!(body.contains("Go Fundamentals"), "filter hit: {body}");
    assert!(
        !body.contains("Rust for LMS"),
        "filter excludes non-matches: {body}"
    );
}

#[tokio::test]
async fn admin_create_and_validation() {
    let repo = Arc::new(InMemoryCourseRepo::new());
    let token = boot_with_session(17503, repo, "dev-user").await; // admin (001 seeder)
    let (code, resp) = call(
        17503,
        "POST",
        "/courses",
        Some(r#"{"title":"New course","description":"d"}"#),
        Some(&token),
    )
    .await
    .expect("connected");
    assert_eq!(code, 201, "admin create → 201");
    assert!(resp.contains("New course"), "201 body: {resp}");
    let (code, resp) = call(
        17503,
        "POST",
        "/courses",
        Some(r#"{"title":""}"#),
        Some(&token),
    )
    .await
    .expect("connected");
    assert_eq!(code, 400, "empty title → 400");
    assert!(resp.contains("title"), "names the field: {resp}");
}

#[tokio::test]
async fn non_admin_write_is_403() {
    let repo = Arc::new(InMemoryCourseRepo::new());
    let token = boot_with_session(17504, repo, "member-user").await; // member role
    let (code, resp) = call(
        17504,
        "POST",
        "/courses",
        Some(r#"{"title":"x"}"#),
        Some(&token),
    )
    .await
    .expect("connected");
    assert_eq!(code, 403, "member write → 403 (US3.1)");
    assert!(resp.contains("\"error\""), "envelope: {resp}");
}

#[tokio::test]
async fn update_delete_and_404() {
    let repo = Arc::new(InMemoryCourseRepo::new());
    let token = boot_with_session(17505, repo.clone(), "dev-user").await;
    repo.create_for_test(contracts::Course {
        id: "c9".into(),
        title: "t".into(),
        description: String::new(),
        organization_id: ORG.into(),
    })
    .await
    .expect("seed");
    let (code, _b) = call(
        17505,
        "PUT",
        "/courses/c9",
        Some(r#"{"title":"t2"}"#),
        Some(&token),
    )
    .await
    .expect("connected");
    assert_eq!(code, 200, "update → 200");
    let (code, _b) = call(17505, "DELETE", "/courses/c9", None, Some(&token))
        .await
        .expect("connected");
    assert_eq!(code, 204, "delete → 204");
    let (code, b) = call(17505, "GET", "/courses/does-not-exist", None, Some(&token))
        .await
        .expect("connected");
    assert_eq!(code, 404, "unknown id → 404 (US3.3)");
    assert!(b.contains("\"error\""), "envelope, no internals: {b}");
}

#[tokio::test]
async fn outage_is_503_never_silent_empty() {
    let repo = Arc::new(InMemoryCourseRepo::new());
    let token = boot_with_session(17506, repo.clone(), "dev-user").await;
    repo.set_unavailable_for_test(true);
    let (code, resp) = call(17506, "GET", "/courses", None, Some(&token))
        .await
        .expect("connected");
    assert_eq!(code, 503, "outage → 503 (FR-008)");
    assert!(
        resp.contains("\"error\""),
        "envelope, not a silent empty list: {resp}"
    );
}
