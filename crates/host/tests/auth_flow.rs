//! US3 — cookie session through the HTTP surface (spec US3.1/US3.2,
//! contracts/api.md): no cookie / unknown / expired → 401 envelope; out-of-
//! scope → 403; session store unreachable → 503 (never 500).

use rand::SeedableRng;
use shared::session::MemorySessionStore;
use shared::storage::{MemoryStorage, Storage as _};
use std::sync::Arc;

mod common;
use common::{get_with, wait_health_ok, AppConfig};

/// Create a real session via the shared auth API and return the token.
async fn make_token(user_id: &str, org: &str) -> (Arc<dyn shared::storage::Storage>, String) {
    let s = MemoryStorage::new();
    s.run_migrations(shared::migrate::MIGRATIONS)
        .await
        .expect("migrate");
    let storage: Arc<dyn shared::storage::Storage> = Arc::new(s);
    let mut rng = rand::rngs::StdRng::seed_from_u64(9);
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64;
    let (token, _row) =
        shared::auth::establish(&*storage, &mut rng, user_id, org, now, 3_600, 72_000)
            .await
            .expect("establish");
    (storage, token)
}

#[tokio::test]
async fn missing_cookie_is_401_with_envelope() {
    let (storage, _token) = make_token("dev-user", "dev-org").await;
    let store = Arc::new(MemorySessionStore::new());
    let enrollment_repo = Arc::new(enrollment::memory::InMemoryEnrollmentRepo::new());
    let course_source = Arc::new(enrollment::memory::InMemoryCourseSource::new());
    let handle = tokio::spawn(async move {
        host::run_with_stop(
            AppConfig::valid(17321),
            storage,
            store,
            std::sync::Arc::new(catalog::repo::InMemoryCourseRepo::new()),
            enrollment_repo,
            course_source,
            Arc::new(scorm::memory::InMemoryScormRepo::new()),
            Arc::new(scorm::memory::InMemoryBagStore::new()),
            Arc::new(scorm::memory::DoubleEnrollmentPort::new()),
            None,
        )
        .await
    });
    assert!(wait_health_ok(17321).await, "serving");

    let (code, body) = get_with(17321, "/me", &[("X-Organization", "dev-org")])
        .await
        .expect("connected");
    assert_eq!(code, 401, "missing cookie → 401 (US3.2)");
    assert!(body.contains("\"error\""), "envelope: {body}");
    handle.abort();
}

#[tokio::test]
async fn valid_cookie_in_scope_is_200_identity() {
    let (storage, token) = make_token("dev-user", "dev-org").await;
    let store = Arc::new(MemorySessionStore::new());
    let enrollment_repo = Arc::new(enrollment::memory::InMemoryEnrollmentRepo::new());
    let course_source = Arc::new(enrollment::memory::InMemoryCourseSource::new());
    let handle = tokio::spawn(async move {
        host::run_with_stop(
            AppConfig::valid(17322),
            storage,
            store,
            std::sync::Arc::new(catalog::repo::InMemoryCourseRepo::new()),
            enrollment_repo,
            course_source,
            Arc::new(scorm::memory::InMemoryScormRepo::new()),
            Arc::new(scorm::memory::InMemoryBagStore::new()),
            Arc::new(scorm::memory::DoubleEnrollmentPort::new()),
            None,
        )
        .await
    });
    assert!(wait_health_ok(17322).await, "serving");

    let (code, body) = get_with(
        17322,
        "/me",
        &[
            ("Cookie", &format!("session={token}")),
            ("X-Organization", "dev-org"),
        ],
    )
    .await
    .expect("connected");
    assert_eq!(code, 200, "in-scope session → 200 (US3.1/3.2)");
    assert!(body.contains("\"user_id\""), "identity envelope: {body}");
    assert!(
        body.contains("dev-user"),
        "identity carries the user id: {body}"
    );
    handle.abort();
}

#[tokio::test]
async fn valid_cookie_out_of_scope_is_403() {
    let (storage, token) = make_token("dev-user", "dev-org").await;
    let store = Arc::new(MemorySessionStore::new());
    let enrollment_repo = Arc::new(enrollment::memory::InMemoryEnrollmentRepo::new());
    let course_source = Arc::new(enrollment::memory::InMemoryCourseSource::new());
    let handle = tokio::spawn(async move {
        host::run_with_stop(
            AppConfig::valid(17323),
            storage,
            store,
            std::sync::Arc::new(catalog::repo::InMemoryCourseRepo::new()),
            enrollment_repo,
            course_source,
            Arc::new(scorm::memory::InMemoryScormRepo::new()),
            Arc::new(scorm::memory::InMemoryBagStore::new()),
            Arc::new(scorm::memory::DoubleEnrollmentPort::new()),
            None,
        )
        .await
    });
    assert!(wait_health_ok(17323).await, "serving");

    let (code, body) = get_with(
        17323,
        "/me",
        &[
            ("Cookie", &format!("session={token}")),
            ("X-Organization", "other-org"),
        ],
    )
    .await
    .expect("connected");
    assert_eq!(code, 403, "out-of-scope → 403 (US3.2)");
    assert!(body.contains("\"error\""), "envelope: {body}");
    handle.abort();
}

#[tokio::test]
async fn expired_cookie_is_401_never_500() {
    // A token established in the far past is expired at "now" (real clock).
    let s = MemoryStorage::new();
    s.run_migrations(shared::migrate::MIGRATIONS)
        .await
        .expect("migrate");
    let storage: Arc<dyn shared::storage::Storage> = Arc::new(s);
    // Create a session whose expiry is already in the past.
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64;
    let row = shared::storage::NewSession {
        token_hash: shared::auth::token_hash("expired-token-abc"),
        user_id: "dev-user".into(),
        organization_id: "dev-org".into(),
        created_at: now - 100_000,
        last_activity: now - 100_000,
        expires_at: now - 1_000,
    };
    storage.create_session(&row).await.expect("create");
    let store = Arc::new(MemorySessionStore::new());
    let enrollment_repo = Arc::new(enrollment::memory::InMemoryEnrollmentRepo::new());
    let course_source = Arc::new(enrollment::memory::InMemoryCourseSource::new());
    let handle = tokio::spawn(async move {
        host::run_with_stop(
            AppConfig::valid(17324),
            storage,
            store,
            std::sync::Arc::new(catalog::repo::InMemoryCourseRepo::new()),
            enrollment_repo,
            course_source,
            Arc::new(scorm::memory::InMemoryScormRepo::new()),
            Arc::new(scorm::memory::InMemoryBagStore::new()),
            Arc::new(scorm::memory::DoubleEnrollmentPort::new()),
            None,
        )
        .await
    });
    assert!(wait_health_ok(17324).await, "serving");

    let (code, _body) = get_with(
        17324,
        "/me",
        &[
            ("Cookie", "session=expired-token-abc"),
            ("X-Organization", "dev-org"),
        ],
    )
    .await
    .expect("connected");
    assert_eq!(code, 401, "expired → 401 (US3.2, never 500)");
    handle.abort();
}
