//! US3 — auth mechanism gates (spec US3: cookie session, org scope, sliding
//! expiry). Deterministic: `now` is injected (Principle XVII).

use rand::SeedableRng;
use shared::auth::{establish, token_hash, verify, AuthError};
use shared::session::MemorySessionStore;
use shared::storage::{MemoryStorage, Storage};
use std::sync::Arc;

const TTL: i64 = 3_600;
const MAX_TTL: i64 = 72_000;

fn rng() -> rand::rngs::StdRng {
    rand::rngs::StdRng::seed_from_u64(42)
}

async fn setup() -> (Arc<dyn Storage>, Arc<MemorySessionStore>) {
    let s = MemoryStorage::new();
    s.run_migrations().await.expect("migrate");
    (
        Arc::new(s) as Arc<dyn Storage>,
        Arc::new(MemorySessionStore::new()),
    )
}

#[tokio::test]
async fn establish_then_verify_same_org_succeeds() {
    let (storage, _store) = setup().await;
    let mut rng = rng();
    let (token, _row) = establish(
        &*storage, &mut rng, "dev-user", "dev-org", 1_000, TTL, MAX_TTL,
    )
    .await
    .expect("establish");
    assert_eq!(token.len(), 43, "32 bytes base64url");
    let ctx = verify(&*storage, &token, "dev-org", 1_100, TTL, MAX_TTL)
        .await
        .expect("verify same org");
    assert_eq!(ctx.user.id, "dev-user");
    assert_eq!(ctx.organization_id, "dev-org");
}

#[tokio::test]
async fn cross_org_request_is_a_scope_error_403() {
    let (storage, _store) = setup().await;
    let mut rng = rng();
    let (token, _row) = establish(
        &*storage, &mut rng, "dev-user", "dev-org", 1_000, TTL, MAX_TTL,
    )
    .await
    .expect("establish");
    // A second org the user is NOT a member of.
    let err = verify(&*storage, &token, "other-org", 1_100, TTL, MAX_TTL)
        .await
        .unwrap_err();
    assert_eq!(err, AuthError::Scope, "scope mismatch → 403 semantics");
}

#[tokio::test]
async fn expired_without_activity_is_401() {
    let (storage, _store) = setup().await;
    let mut rng = rng();
    let (token, _row) = establish(
        &*storage, &mut rng, "dev-user", "dev-org", 1_000, TTL, MAX_TTL,
    )
    .await
    .expect("establish");
    let err = verify(&*storage, &token, "dev-org", 1_000 + TTL, TTL, MAX_TTL)
        .await
        .unwrap_err();
    assert_eq!(
        err,
        AuthError::Unauthenticated,
        "expired, no activity → 401"
    );
}

#[tokio::test]
async fn sliding_expiry_extends_past_original_expiry() {
    let (storage, _store) = setup().await;
    let mut rng = rng();
    let (token, _row) = establish(
        &*storage, &mut rng, "dev-user", "dev-org", 1_000, TTL, MAX_TTL,
    )
    .await
    .expect("establish");
    // Activity near the edge slides the expiry forward.
    let active_at = 1_000 + TTL - 10;
    verify(&*storage, &token, "dev-org", active_at, TTL, MAX_TTL)
        .await
        .expect("alive near the edge");
    // Past the ORIGINAL expiry — alive only because of the slide.
    verify(&*storage, &token, "dev-org", 1_000 + TTL, TTL, MAX_TTL)
        .await
        .expect("alive after slide (past original expiry)");
    // The previous check (1_000+TTL) slid again to 1_000+2TTL. Past THAT it
    // dies.
    let err = verify(
        &*storage,
        &token,
        "dev-org",
        1_000 + 2 * TTL + 1,
        TTL,
        MAX_TTL,
    )
    .await
    .unwrap_err();
    assert_eq!(err, AuthError::Unauthenticated, "slid expiry enforced");
}

#[tokio::test]
async fn absolute_max_bound_wins_over_sliding() {
    let (storage, _store) = setup().await;
    let mut rng = rng();
    let (token, _row) = establish(
        &*storage, &mut rng, "dev-user", "dev-org", 1_000, TTL, MAX_TTL,
    )
    .await
    .expect("establish");
    // Chain of slides, each just inside the current expiry; the last one
    // clamps the expiry to the absolute bound (created + MAX_TTL).
    let mut t = 1_000 + TTL - 10;
    for _ in 0..20 {
        verify(&*storage, &token, "dev-org", t, TTL, MAX_TTL)
            .await
            .expect("alive during the slide chain");
        t += TTL - 10; // next slide just inside the (slid) expiry
        if t > 1_000 + MAX_TTL - 10 {
            t = 1_000 + MAX_TTL - 10;
        }
    }
    let err = verify(
        &*storage,
        &token,
        "dev-org",
        1_000 + MAX_TTL + 1,
        TTL,
        MAX_TTL,
    )
    .await
    .unwrap_err();
    assert_eq!(
        err,
        AuthError::Unauthenticated,
        "absolute max bound enforced"
    );
}

#[tokio::test]
async fn unknown_or_garbage_token_is_401_never_500() {
    let (storage, _store) = setup().await;
    let err = verify(
        &*storage,
        "not-a-real-token",
        "dev-org",
        1_000,
        TTL,
        MAX_TTL,
    )
    .await
    .unwrap_err();
    assert_eq!(err, AuthError::Unauthenticated);
}

#[tokio::test]
async fn tokens_are_unique_and_hashed() {
    let mut a = rand::rngs::StdRng::seed_from_u64(42);
    let mut b = rand::rngs::StdRng::seed_from_u64(43);
    let t1 = shared::auth::generate_token(&mut a);
    let t2 = shared::auth::generate_token(&mut b);
    assert_ne!(t1, t2, "tokens are unique (same seed, sequential draws)");
    let h = token_hash(&t1);
    assert_eq!(h.len(), 64, "sha256 hex");
    assert!(!h.contains(&t1), "hash never leaks the token");
}
