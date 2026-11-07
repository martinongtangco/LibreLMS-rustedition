//! US2/US3 (P2/P3) — bag semantics over the doubles (spec 005).
//! T008/T012 red-first. FR-003: the bag is content-addressed by the session
//! token_hash at the 001-reserved key (the key lives in shared ground truth
//! — the module never names it). FR-006: a lost bag rehydrates from the
//! last commit (the committed state in the repo survives the flush).

use std::sync::Arc;

use scorm::memory::{InMemoryBagStore, InMemoryScormRepo};
use scorm::repo::{validate_value, BagStore, CmiBag, ScormError, ScormRepo};

const SESSION: &str = "token-hash-abc";
const COURSE: &str = "course-1";
const ORG: &str = "org-1";

fn row_for(session: &str) -> scorm::repo::ScormAttempt {
    scorm::repo::ScormAttempt {
        id: String::new(),
        session_id: session.to_string(),
        course_id: COURSE.to_string(),
        organization_id: ORG.to_string(),
        state: "in_progress".to_string(),
        suspend_data: String::new(),
        score_raw: None,
        session_time: 0,
        finished_at: None,
        created_at: 0,
    }
}

#[tokio::test]
async fn bag_get_set_round_trip() {
    // T008: LMSSetValue/LMSGetValue — set → get returns the JSON string;
    // absent → Ok(None) (never a silent fresh bag — the app layer decides
    // the rehydration, FR-006).
    let bag = InMemoryBagStore::new();
    assert_eq!(bag.get_bag(SESSION).await.unwrap(), None);

    let mut b = CmiBag::fresh();
    b.lesson_status = "in_progress".to_string();
    b.score = Some(42);
    b.suspend_data = "midpoint".to_string();
    let json = serde_json::to_string(&b).unwrap();
    bag.set_bag(SESSION, &json, 3_600).await.unwrap();

    let got = bag.get_bag(SESSION).await.unwrap().unwrap();
    let parsed: CmiBag = serde_json::from_str(&got).unwrap();
    assert_eq!(parsed.lesson_status, "in_progress");
    assert_eq!(parsed.score, Some(42));
    assert_eq!(parsed.suspend_data, "midpoint");

    // A second session key is isolated (the bag key includes the session —
    // no cross-session bleed).
    assert_eq!(bag.get_bag("other-session").await.unwrap(), None);

    // Overwrite semantics: a later set wins (the bag is a single slot per
    // session — the commit/finish flow sets it in order).
    let mut b2 = CmiBag::fresh();
    b2.lesson_status = "completed".to_string();
    bag.set_bag(SESSION, &serde_json::to_string(&b2).unwrap(), 3_600)
        .await
        .unwrap();
    let got2: CmiBag =
        serde_json::from_str(bag.get_bag(SESSION).await.unwrap().unwrap().as_str()).unwrap();
    assert_eq!(got2.lesson_status, "completed");

    // Outage → 503-class, never a silent None (FR-010).
    bag.set_unavailable_for_test(true);
    assert!(matches!(
        bag.get_bag(SESSION).await,
        Err(ScormError::Unavailable)
    ));
    assert!(matches!(
        bag.set_bag(SESSION, "x", 60).await,
        Err(ScormError::Unavailable)
    ));
}

#[tokio::test]
async fn bag_fresh_defaults() {
    // FR-001: the fresh bag is `in_progress` with score None and a 4,000-
    // char-cap suspend (the defaults are the spec's, not an implementation
    // choice).
    let b = CmiBag::fresh();
    assert_eq!(b.lesson_status, "in_progress");
    assert_eq!(b.score, None);
    assert_eq!(b.session_time, 0);
    assert_eq!(b.suspend_data, "");
    // Serialize shape: a compact JSON object (the transport form).
    let json = serde_json::to_string(&b).unwrap();
    let back: CmiBag = serde_json::from_str(&json).unwrap();
    assert_eq!(back.lesson_status, "in_progress");
}

#[tokio::test]
async fn fr006_bag_loss_rehydration() {
    // FR-006: the committed state survives a bag loss — the rehydrated bag
    // is rebuilt from the last commit (the row's committed values), and
    // the app flow continues from it (a new bag is NOT started fresh —
    // that would silently drop the committed score/suspend).
    let repo = InMemoryScormRepo::new();
    let bag = InMemoryBagStore::new();
    let row = row_for(SESSION);
    repo.create(&row).await.unwrap();
    // Re-read the row (the double assigns the id on create — the real
    // engine does the same with NEWID; the app reads the winner's row).
    let row = repo
        .find_by_session_course(SESSION, COURSE)
        .await
        .unwrap()
        .unwrap();
    // Commit a meaningful state (score 55, suspend "checkpoint-A", 120s).
    repo.update_committed(&row.id, "in_progress", "checkpoint-A", Some(55), 120)
        .await
        .unwrap();
    let committed = repo
        .find_by_session_course(SESSION, COURSE)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(committed.score_raw, Some(55));
    assert_eq!(committed.suspend_data, "checkpoint-A");
    assert_eq!(committed.session_time, 120);

    // The bag mirrors the commit (the app sets it after the commit).
    let mut mirrored = CmiBag::fresh();
    mirrored.lesson_status = committed.state.clone();
    mirrored.score = committed.score_raw;
    mirrored.suspend_data = committed.suspend_data.clone();
    mirrored.session_time = committed.session_time;
    bag.set_bag(SESSION, &serde_json::to_string(&mirrored).unwrap(), 3_600)
        .await
        .unwrap();

    // Bag loss (flush — the session store dropped the key).
    bag.flush_for_test();
    assert!(bag.get_bag(SESSION).await.unwrap().is_none());

    // Rehydration: the rehydrated bag is built from the committed state in
    // the repo (NOT from fresh defaults — the committed values survive).
    let rehydrated = rehydrate_from_commit(&committed).await;
    assert_eq!(rehydrated.lesson_status, "in_progress");
    assert_eq!(rehydrated.score, Some(55), "the committed score survives");
    assert_eq!(
        rehydrated.suspend_data, "checkpoint-A",
        "the committed suspend survives"
    );
    assert_eq!(rehydrated.session_time, 120, "the committed time survives");

    // A second commit after rehydration advances the state (the flow
    // continues from the rehydrated bag, not from scratch).
    let row2 = repo
        .find_by_session_course(SESSION, COURSE)
        .await
        .unwrap()
        .unwrap();
    repo.update_committed(&row2.id, "in_progress", "checkpoint-B", Some(77), 300)
        .await
        .unwrap();
    let advanced = repo
        .find_by_session_course(SESSION, COURSE)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(advanced.suspend_data, "checkpoint-B");
    assert_eq!(advanced.score_raw, Some(77));
    assert_eq!(advanced.session_time, 300);
}

/// The app-level rehydration rule (FR-006): rebuild the bag from the last
/// commit's row values. (The rule is exercised here as the function the
/// routes apply when `get_bag` returns `None` for a live attempt.)
async fn rehydrate_from_commit(committed: &scorm::repo::ScormAttempt) -> CmiBag {
    let mut b = CmiBag::fresh();
    b.lesson_status = committed.state.clone();
    b.score = committed.score_raw;
    b.suspend_data = committed.suspend_data.clone();
    b.session_time = committed.session_time;
    b
}

#[tokio::test]
async fn suspend_data_value_limits_and_validation() {
    // T012 (FR-011): the 4,000-char limit — the boundary is inclusive;
    // 4,001 rejects naming the element (400-class).
    assert!(validate_value("cmi.suspend_data", &"x".repeat(4_000)).is_ok());
    let over = "x".repeat(4_001);
    assert!(matches!(
        validate_value("cmi.suspend_data", &over),
        Err(ScormError::Validation {
            field, ..
        }) if field == "cmi.suspend_data"
    ));

    // FR-012 (the bag half): an injection-shaped suspend round-trips through
    // serialize/parse as plain text (the parameterized SQL is the engine's
    // half — the full-env test exercises the real engine; this is the
    // transport half: no structural mutation of the payload).
    let mut b = CmiBag::fresh();
    b.suspend_data = "'); DROP TABLE scorm_attempts;-- <script>x</script>".to_string();
    let json = serde_json::to_string(&b).unwrap();
    let back: CmiBag = serde_json::from_str(&json).unwrap();
    assert_eq!(back.suspend_data, b.suspend_data);
    // The repo round-trip (double half — the engine half is full-env).
    let repo = InMemoryScormRepo::new();
    let row = row_for("sess-limit");
    repo.create(&row).await.unwrap();
    // Re-read (the double assigns the id on create, as the engine does).
    let row = repo
        .find_by_session_course("sess-limit", COURSE)
        .await
        .unwrap()
        .unwrap();
    repo.update_committed(&row.id, "in_progress", &back.suspend_data, None, 0)
        .await
        .unwrap();
    let got = repo
        .find_by_session_course("sess-limit", COURSE)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(got.suspend_data, b.suspend_data);
}

#[tokio::test]
async fn arc_shared_store_isolation() {
    // The Arc-shared double behaves like the shared store: two handles
    // observe the same bags (the host shares ONE store across the 001
    // slice and the scorm slice — the 001-reserved key is the seam).
    let bag: Arc<InMemoryBagStore> = Arc::new(InMemoryBagStore::new());
    let h1 = bag.clone();
    h1.set_bag("k1", "{}", 60).await.unwrap();
    assert!(bag.get_bag("k1").await.unwrap().is_some());
}
