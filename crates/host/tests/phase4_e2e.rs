//! Phase 4 — the cross-slice E2E (the integration proof): enroll via 004
//! (through the 003 catalog adapter) → SCORM launch/finish through the 004
//! repository adapter → the completion visible in the 004 list (with the
//! title resolved through the 003 adapter). Adapters are the REAL
//! `CatalogCourseSource` / `EnrollmentRepoPort` over the module doubles —
//! the full HTTP surface is exercised, the engine half is the full-env
//! battery.
//!
//! The 404/403 decision procedure is proven across the whole stack:
//! unknown course → 404 (adapter `find_by_id` → None); a course of another
//! org → 403 (adapter `exists_in_org` → false); both via the 004 enroll
//! route (the 004 plan's per-call-site procedure) AND via the 005 launch
//! gate (the port's error type carries the same distinction, FR-002).

use std::sync::Arc;

use catalog::repo::CourseRepo;
use contracts::{Role, User};
use enrollment::memory::InMemoryEnrollmentRepo;
use host::adapters::{CatalogCourseSource, EnrollmentRepoPort};
use scorm::memory::{InMemoryBagStore, InMemoryScormRepo};
use scorm::repo::EnrollmentPort;
use shared::session::MemorySessionStore;
use shared::storage::{MemoryStorage, Storage as _};

mod common;
use common::{wait_health_ok, AppConfig};

const ORG: &str = "dev-org";
const ORG_B: &str = "org-b";
const ADMIN: &str = "dev-user";

/// One raw-socket HTTP call (the 001 pattern — no extra client).
async fn call(
    port: u16,
    method: &str,
    path: &str,
    body: Option<&str>,
    token: Option<&str>,
    org_header: &str,
) -> Option<(u16, String)> {
    let mut stream = tokio::net::TcpStream::connect(("127.0.0.1", port))
        .await
        .ok()?;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let body = body.unwrap_or("");
    let mut req = format!(
        "{method} {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nContent-Length: {}\r\nConnection: close\r\nX-Organization: {org_header}\r\n",
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

/// Boot the host with the REAL Phase-4 adapters over the module doubles.
/// Returns (port, admin_token, learner_token, the shared doubles for
/// fault-injection / inspection).
#[allow(clippy::type_complexity)]
async fn boot(
    port: u16,
) -> (
    Arc<catalog::repo::InMemoryCourseRepo>,
    Arc<enrollment::memory::InMemoryEnrollmentRepo>,
    Arc<scorm::memory::InMemoryScormRepo>,
    Arc<scorm::memory::InMemoryBagStore>,
    Arc<host::adapters::EnrollmentRepoPort>,
    String,
    String,
) {
    let s = MemoryStorage::new();
    s.run_migrations(shared::migrate::MIGRATIONS)
        .await
        .expect("migrate");
    // The learner: a member of ORG (the flow) and ORG_B (the scope case).
    let learner = "p4-learner";
    s.create_user_for_test(User {
        id: learner.into(),
        email: format!("{learner}@example.com"),
        name: learner.into(),
        theme: "default".into(),
    });
    s.set_org_role_for_test(learner, ORG, Role::Member);
    s.set_org_role_for_test(learner, ORG_B, Role::Member);

    let catalog = Arc::new(catalog::repo::InMemoryCourseRepo::new());
    let enrollment = Arc::new(InMemoryEnrollmentRepo::new());
    let scorm_repo = Arc::new(InMemoryScormRepo::new());
    let scorm_bag = Arc::new(InMemoryBagStore::new());
    // The Phase-4 adapters (the production wiring shape — main.rs wires
    // the same two structs over the real repos).
    let course_source: Arc<dyn enrollment::repo::CourseSource> =
        Arc::new(CatalogCourseSource::new(catalog.clone()));
    let scorm_port = Arc::new(EnrollmentRepoPort::new(
        enrollment.clone(),
        course_source.clone(),
    ));

    let storage: Arc<dyn shared::storage::Storage> = Arc::new(s);
    let store: Arc<dyn shared::session::SessionStore> = Arc::new(MemorySessionStore::new());
    let mut rng = rand::rngs::StdRng::seed_from_u64(port as u64);
    use rand::SeedableRng;
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64;
    let (admin_token, _) =
        shared::auth::establish(&*storage, &mut rng, ADMIN, ORG, now, 3_600, 72_000)
            .await
            .expect("admin establish");
    let (learner_token, _) =
        shared::auth::establish(&*storage, &mut rng, learner, ORG, now, 3_600, 72_000)
            .await
            .expect("learner establish");

    // Clones for the return (the spawn takes its own moves).
    let (ret_catalog, ret_enrollment, ret_scorm_repo, ret_bag, ret_port) = (
        catalog.clone(),
        enrollment.clone(),
        scorm_repo.clone(),
        scorm_bag.clone(),
        scorm_port.clone(),
    );

    tokio::spawn(async move {
        host::run_with_stop(
            AppConfig::valid(port),
            storage,
            store,
            catalog.clone(), // the SAME double the adapter reads (one system of record)
            enrollment.clone(),
            course_source.clone(),
            scorm_repo,
            scorm_bag,
            scorm_port.clone(),
            Arc::new(management::repo::InMemoryManagementRepo::new()),
            None,
        )
        .await
    });
    assert!(wait_health_ok(port).await, "host is healthy");
    (
        ret_catalog,
        ret_enrollment,
        ret_scorm_repo,
        ret_bag,
        ret_port,
        admin_token,
        learner_token,
    )
}

#[tokio::test]
async fn cross_slice_e2e_enroll_scorm_complete() {
    let (catalog, _enrollment, _scorm_repo, _bag, _port, admin_token, learner_token) =
        boot(18_900).await;

    // 1. Admin creates the course (003 slice).
    let (code, body) = call(
        18_900,
        "POST",
        "/courses",
        Some(r#"{"title":"Phase4 Course"}"#),
        Some(&admin_token),
        ORG,
    )
    .await
    .unwrap();
    assert_eq!(code, 201, "admin course create → 201: {body}");
    let course_id = serde_json::from_str::<serde_json::Value>(&body)
        .expect("json")
        .get("id")
        .and_then(|v| v.as_str())
        .expect("id")
        .to_string();

    // 2. The learner enrolls (004 slice) — through the REAL catalog
    // adapter (the 404/403 decision procedure: title Some + in-org true).
    let (code, body) = call(
        18_900,
        "POST",
        "/enrollments",
        Some(&format!("{{\"course_id\":\"{course_id}\"}}")),
        Some(&learner_token),
        ORG,
    )
    .await
    .unwrap();
    assert_eq!(code, 201, "enroll through the adapter → 201: {body}");

    // 3. SCORM launch (005 slice) — the gate reads the 004 state through
    // the REAL enrollment adapter (course check via the 003 adapter first).
    let (code, body) = call(
        18_900,
        "POST",
        &format!("/scorm/{course_id}/initialize"),
        None,
        Some(&learner_token),
        ORG,
    )
    .await
    .unwrap();
    assert_eq!(code, 200, "scorm launch through the port → 200: {body}");

    // 4. in_progress → commit → passed → finish (the implicit final commit
    // drives record_completion through the port → the 004 transition).
    let (code, _) = call(
        18_900,
        "POST",
        &format!("/scorm/{course_id}/set"),
        Some(r#"{"element":"cmi.core.lesson_status","value":"in_progress"}"#),
        Some(&learner_token),
        ORG,
    )
    .await
    .unwrap();
    assert_eq!(code, 200, "set in_progress → 200");
    let (code, _) = call(
        18_900,
        "POST",
        &format!("/scorm/{course_id}/commit"),
        None,
        Some(&learner_token),
        ORG,
    )
    .await
    .unwrap();
    assert_eq!(code, 200, "commit → 200");
    let (code, _) = call(
        18_900,
        "POST",
        &format!("/scorm/{course_id}/set"),
        Some(r#"{"element":"cmi.core.lesson_status","value":"passed"}"#),
        Some(&learner_token),
        ORG,
    )
    .await
    .unwrap();
    assert_eq!(code, 200, "set passed → 200");
    let (code, body) = call(
        18_900,
        "POST",
        &format!("/scorm/{course_id}/finish"),
        None,
        Some(&learner_token),
        ORG,
    )
    .await
    .unwrap();
    assert_eq!(code, 200, "finish → 200: {body}");

    // 5. THE cross-slice proof: the 004 list shows the course as
    // completed, with the TITLE resolved through the 003 adapter
    // (EnrollmentView.title = Some via CourseSource::course_title).
    let (code, body) = call(
        18_900,
        "GET",
        "/enrollments",
        None,
        Some(&learner_token),
        ORG,
    )
    .await
    .unwrap();
    assert_eq!(code, 200, "004 list → 200: {body}");
    let parsed: serde_json::Value = serde_json::from_str(&body).expect("json");
    let view = parsed
        .as_array()
        .and_then(|a| {
            a.iter()
                .find(|v| v.get("course_id").and_then(|c| c.as_str()) == Some(&course_id))
        })
        .expect("the enrollment row");
    assert_eq!(
        view.get("status").and_then(|s| s.as_str()),
        Some("completed"),
        "the completion is visible in the 004 list (cross-slice): {body}"
    );
    assert_eq!(
        view.get("title").and_then(|t| t.as_str()),
        Some("Phase4 Course"),
        "the title is resolved through the 003 adapter: {body}"
    );
    assert!(
        view.get("completed_at").is_some(),
        "the first-completion timestamp is visible: {body}"
    );

    // 6. Re-launch after the completion → 409 already-completed via the
    // PORT (the adapter reads the 004 Completed state — the 005 gate
    // before resume, never a second attempt).
    let (code, body) = call(
        18_900,
        "POST",
        &format!("/scorm/{course_id}/initialize"),
        None,
        Some(&learner_token),
        ORG,
    )
    .await
    .unwrap();
    assert_eq!(
        code, 409,
        "re-launch after completion → 409 already-completed (port reads 004 state): {body}"
    );

    // The adapter's idempotency (the 004 engine anchor's double analog):
    // a direct second record_completion is a no-op success, never an
    // error, never a second timestamp.
    _port
        .record_completion("p4-learner", &course_id, ORG)
        .await
        .expect("idempotent second completion — never an error");
    let (_code, body) = call(
        18_900,
        "GET",
        "/enrollments",
        None,
        Some(&learner_token),
        ORG,
    )
    .await
    .unwrap();
    let ts: Vec<i64> = serde_json::from_str::<serde_json::Value>(&body)
        .expect("json")
        .as_array()
        .expect("array")
        .iter()
        .filter_map(|v| v.get("completed_at").and_then(|t| t.as_i64()))
        .collect();
    assert_eq!(ts.len(), 1, "one completion record (first-wins): {body}");

    // 7. Decision procedure across the stack (004 enroll route):
    // unknown course → 404 (the adapter's find_by_id → None).
    let (code, _) = call(
        18_900,
        "POST",
        "/enrollments",
        Some(r#"{"course_id":"no-such-course"}"#),
        Some(&learner_token),
        ORG,
    )
    .await
    .unwrap();
    assert_eq!(code, 404, "unknown course → 404 through the adapter");

    // a course of ANOTHER org → 403 (the adapter's exists_in_org →
    // false; content-identical to any other 403, no existence oracle).
    let (code, body) = call(
        18_900,
        "POST",
        "/enrollments",
        Some(&format!("{{\"course_id\":\"{course_id}\"}}")),
        Some(&learner_token),
        ORG_B, // the learner's session is bound to ORG_B here
    )
    .await
    .unwrap();
    assert_eq!(
        code, 403,
        "a course of another org → 403 scope through the adapter: {body}"
    );

    // The 005 gate carries the same distinction (FR-002): launch on the
    // other org's course → 403 Scope via the port; unknown course → 404.
    let (code, body) = call(
        18_900,
        "POST",
        &format!("/scorm/{course_id}/initialize"),
        None,
        Some(&learner_token),
        ORG_B,
    )
    .await
    .unwrap();
    assert_eq!(
        code, 403,
        "scorm launch on another org's course → 403 via the port: {body}"
    );
    let (code, _) = call(
        18_900,
        "POST",
        "/scorm/no-such-course/initialize",
        None,
        Some(&learner_token),
        ORG,
    )
    .await
    .unwrap();
    assert_eq!(
        code, 404,
        "scorm launch on unknown course → 404 via the port"
    );

    // The catalog double holds exactly one course (no cross-slice bleed —
    // the single shared system of record was used by both the 003 route
    // and the adapter).
    let listed = catalog.list_for(ORG, None).await.expect("catalog list");
    assert_eq!(listed.len(), 1, "one course in the shared system of record");
}

#[tokio::test]
async fn adapter_outage_never_fake() {
    // N1 across the adapters: a catalog outage is 503-class — never a
    // fake Ok(None)/Ok(false) (which would read as 404/403).
    let (catalog, _enrollment, _scorm_repo, _bag, _port, _admin, learner_token) =
        boot(18_910).await;

    catalog.set_unavailable_for_test(true);
    // 004 enroll route: the CourseSource outage → 503 (never a fake 404).
    let (code, _) = call(
        18_910,
        "POST",
        "/enrollments",
        Some(r#"{"course_id":"any-course"}"#),
        Some(&learner_token),
        ORG,
    )
    .await
    .unwrap();
    assert_eq!(
        code, 503,
        "catalog outage → 503 through the adapter (never a fake 404)"
    );
    // 005 launch gate: the port's course check hits the outage → 503
    // (never a fake NotEnrolled/NotFound/Scope).
    let (code, body) = call(
        18_910,
        "POST",
        "/scorm/any-course/initialize",
        None,
        Some(&learner_token),
        ORG,
    )
    .await
    .unwrap();
    assert_eq!(
        code, 503,
        "course-check outage → 503 through the port (never a fake gate): {body}"
    );
}
