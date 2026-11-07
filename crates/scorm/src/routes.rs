//! SCORM HTTP surface (spec 005). The routes are state-bound to the repo,
//! bag store, and enrollment port (host mount pattern: the host merges
//! this router and inserts the auth-resolved caller as an extension) — the
//! module never depends on host types (Principle III).
//!
//! The thin SCORM subset (FR-004): LMSInitialize / LMSGetValue /
//! LMSSetValue / LMSCommit / LMSFinish over the `cmi` data subset.

use std::sync::Arc;

use axum::extract::{FromRef, Path, State as AxumState};
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use axum::{Extension, Json, Router};
use serde::Deserialize;

use contracts::ErrorEnvelope;

use crate::repo::{
    validate_value, BagStore, CmiBag, EnrollmentPort, LaunchGate, ScormAttempt, ScormError,
    ScormRepo,
};

/// What the host's auth middleware resolves the session to (scorm-owned
/// shape so the module stays host-independent — Principle III). `token_hash`
/// addresses the bag at the 001-reserved key (the host computes it via
/// `shared::auth::token_hash`; the module never names the key).
#[derive(Debug, Clone)]
pub struct ScormCaller {
    pub user_id: String,
    pub organization_id: String,
    pub role: contracts::Role,
    pub token_hash: String,
}

/// State holder (satisfies axum 0.8's `State(S: Clone + Send + Sync)` bound).
#[derive(Clone)]
pub struct RepoState {
    repo: Arc<dyn ScormRepo>,
    bag: Arc<dyn BagStore>,
    port: Arc<dyn EnrollmentPort>,
}

impl RepoState {
    pub fn new(
        repo: Arc<dyn ScormRepo>,
        bag: Arc<dyn BagStore>,
        port: Arc<dyn EnrollmentPort>,
    ) -> Self {
        Self { repo, bag, port }
    }

    fn repo(&self) -> &Arc<dyn ScormRepo> {
        &self.repo
    }
    fn bag(&self) -> &Arc<dyn BagStore> {
        &self.bag
    }
    fn port(&self) -> &Arc<dyn EnrollmentPort> {
        &self.port
    }
}

/// The bag TTL = session lifetime (the shared store's Principle — the 001
/// session sliding expiry is 3,600s in the test fixtures; the production
/// session TTL rides the same value).
const BAG_TTL_SECS: u64 = 3_600;

#[derive(Deserialize)]
struct ElementBody {
    element: Option<String>,
    value: Option<String>,
}

/// Error envelope mapping (plan table — hyphen stable codes, 001 api.md
/// style; never leaks backend internals; never a 500, FR-010).
pub fn error_response(e: &ScormError) -> Response {
    let (status, code, message) = match e {
        ScormError::NotFound => (
            axum::http::StatusCode::NOT_FOUND,
            "not-found".to_string(),
            "course not found".to_string(),
        ),
        ScormError::Scope => (
            axum::http::StatusCode::FORBIDDEN,
            "scope".to_string(),
            "organization scope mismatch".to_string(),
        ),
        ScormError::NotEnrolled => (
            axum::http::StatusCode::CONFLICT,
            "not-enrolled".to_string(),
            "not enrolled in this course".to_string(),
        ),
        ScormError::AlreadyCompleted => (
            axum::http::StatusCode::CONFLICT,
            "already-completed".to_string(),
            "enrollment is already completed".to_string(),
        ),
        ScormError::AttemptFinal => (
            axum::http::StatusCode::CONFLICT,
            "attempt-final".to_string(),
            "the attempt is final (LMSFinish) — no further mutations".to_string(),
        ),
        ScormError::Validation { field, message } => (
            axum::http::StatusCode::BAD_REQUEST,
            "validation".to_string(),
            format!("{field}: {message}"),
        ),
        ScormError::Unavailable => (
            axum::http::StatusCode::SERVICE_UNAVAILABLE,
            "storage-unavailable".to_string(),
            "storage temporarily unavailable".to_string(),
        ),
        // Internal (the 2601/2627 loser path) — never an error response:
        // the launch procedure reads the winner's row instead. Defensive
        // 503 should it ever surface at a route edge.
        ScormError::AttemptExists => (
            axum::http::StatusCode::SERVICE_UNAVAILABLE,
            "storage-unavailable".to_string(),
            "storage temporarily unavailable".to_string(),
        ),
    };
    (
        status,
        Json(ErrorEnvelope {
            error: code,
            message,
        }),
    )
        .into_response()
}

/// Read the live bag, or rehydrate it from the row's committed state
/// (FR-006: a store flush loses only the bag — the committed state
/// survives; no commit → a fresh `in_progress`).
async fn bag_or_rehydrate(
    state: &RepoState,
    token_hash: &str,
    row: Option<&ScormAttempt>,
) -> Result<(CmiBag, bool), ScormError> {
    if let Some(json) = state.bag().get_bag(token_hash).await? {
        return serde_json::from_str::<CmiBag>(&json)
            .map(|b| (b, false))
            .map_err(|_| ScormError::Unavailable);
    }
    match row {
        Some(r) => {
            let b = CmiBag {
                lesson_status: r.state.clone(),
                score: r.score_raw,
                suspend_data: r.suspend_data.clone(),
                session_time: r.session_time,
            };
            // Rehydrate: write the bag back (a set failure is a 503 — the
            // row survives; the next call rehydrates again).
            let json = serde_json::to_string(&b).map_err(|_| ScormError::Unavailable)?;
            state.bag().set_bag(token_hash, &json, BAG_TTL_SECS).await?;
            Ok((b, true))
        }
        None => Ok((CmiBag::fresh(), false)),
    }
}

/// LMSInitialize (FR-001/FR-002, plan Launch Procedure — the gate runs
/// BEFORE the idempotent resume; a re-launch of a completed enrollment is
/// 409 already-completed, never a resume of the final attempt).
async fn initialize(
    Extension(caller): Extension<ScormCaller>,
    AxumState(state): AxumState<RepoState>,
    Path(course_id): Path<String>,
) -> Response {
    // Step 1 — the enrollment gate (before the resume; FR-002).
    match state
        .port()
        .enrollment_status(&caller.user_id, &course_id, &caller.organization_id)
        .await
    {
        Err(ScormError::NotFound) => return error_response(&ScormError::NotFound),
        Err(ScormError::Scope) => return error_response(&ScormError::Scope),
        Err(e) => return error_response(&e),
        Ok(LaunchGate::NotEnrolled) => return error_response(&ScormError::NotEnrolled),
        Ok(LaunchGate::Completed) => return error_response(&ScormError::AlreadyCompleted),
        Ok(LaunchGate::Enrolled) => {}
    }
    // Step 2 — idempotent resume (FR-001: one attempt per (session, course)).
    let existing = match state
        .repo()
        .find_by_session_course(&caller.token_hash, &course_id)
        .await
    {
        Ok(r) => r,
        Err(e) => return error_response(&e),
    };
    match existing {
        Some(row) if row.finished_at.is_some() => {
            // The race-window belt: a finalized row with a still-Enrolled
            // gate can only exist inside the completion race — a final
            // attempt is never resumed (plan).
            error_response(&ScormError::AttemptFinal)
        }
        Some(row) => {
            // Resume (rehydrate if the bag was lost — FR-006).
            match bag_or_rehydrate(&state, &caller.token_hash, Some(&row)).await {
                Ok((_, _)) => Json(row).into_response(),
                Err(e) => error_response(&e),
            }
        }
        None => {
            // Step 3 — create; the UNIQUE constraint is the US1.6 engine
            // arbiter: a 2601/2627 violation (AttemptExists) reads the
            // winner's row (200 — never a 409, never a 500, never a
            // second row).
            let fresh = ScormAttempt {
                id: String::new(),
                session_id: caller.token_hash.clone(),
                course_id: course_id.clone(),
                organization_id: caller.organization_id.clone(),
                state: "in_progress".to_string(),
                suspend_data: String::new(),
                score_raw: None,
                session_time: 0,
                finished_at: None,
                created_at: 0,
            };
            match state.repo().create(&fresh).await {
                Ok(()) => {}
                Err(ScormError::AttemptExists) => {
                    // The loser reads the winner's row (US1.6).
                    let winner = match state
                        .repo()
                        .find_by_session_course(&caller.token_hash, &course_id)
                        .await
                    {
                        Ok(Some(w)) => w,
                        Ok(None) => return error_response(&ScormError::Unavailable),
                        Err(e) => return error_response(&e),
                    };
                    if winner.finished_at.is_some() {
                        return error_response(&ScormError::AttemptFinal);
                    }
                    let _ = bag_or_rehydrate(&state, &caller.token_hash, Some(&winner)).await;
                    return Json(winner).into_response();
                }
                Err(e) => return error_response(&e),
            }
            // Initialize the live bag (the CmiBag::fresh shape; a bag write
            // failure is a 503 — the row survives and rehydrates on the
            // next launch, FR-006).
            let fresh_bag = CmiBag::fresh();
            let json = match serde_json::to_string(&fresh_bag) {
                Ok(j) => j,
                Err(_) => return error_response(&ScormError::Unavailable),
            };
            if let Err(e) = state
                .bag()
                .set_bag(&caller.token_hash, &json, BAG_TTL_SECS)
                .await
            {
                return error_response(&e);
            }
            let row = match state
                .repo()
                .find_by_session_course(&caller.token_hash, &course_id)
                .await
            {
                Ok(Some(r)) => r,
                Ok(None) => return error_response(&ScormError::Unavailable),
                Err(e) => return error_response(&e),
            };
            Json(row).into_response()
        }
    }
}

/// The live-or-rehydrated bag for a mutation (set/commit/finish): the row
/// must exist (404) and be live (409 attempt-final).
async fn live_bag(
    state: &RepoState,
    caller: &ScormCaller,
    course_id: &str,
) -> Result<(CmiBag, ScormAttempt), ScormError> {
    let row = match state
        .repo()
        .find_by_session_course(&caller.token_hash, course_id)
        .await?
    {
        Some(r) => r,
        None => return Err(ScormError::NotFound),
    };
    if row.finished_at.is_some() {
        return Err(ScormError::AttemptFinal);
    }
    let (bag, _) = bag_or_rehydrate(state, &caller.token_hash, Some(&row)).await?;
    Ok((bag, row))
}

/// LMSGetValue (FR-004 — read the element; a get before any init returns
/// the fresh `in_progress` defaults read-only, plan; an unknown element is
/// a 400 naming it, FR-011).
async fn get_value(
    Extension(caller): Extension<ScormCaller>,
    AxumState(state): AxumState<RepoState>,
    Path(course_id): Path<String>,
    Json(body): Json<ElementBody>,
) -> Response {
    let Some(element) = body.element.filter(|e| !e.trim().is_empty()) else {
        return error_response(&ScormError::Validation {
            field: "element".to_string(),
            message: "element is required".to_string(),
        });
    };
    // The row (for rehydration; absent → the fresh read-only defaults).
    let row = match state
        .repo()
        .find_by_session_course(&caller.token_hash, &course_id)
        .await
    {
        Ok(r) => r,
        Err(e) => return error_response(&e),
    };
    let (bag, _) = match bag_or_rehydrate(&state, &caller.token_hash, row.as_ref()).await {
        Ok(v) => v,
        Err(e) => return error_response(&e),
    };
    // Unknown element → 400 naming it (the value is irrelevant for a get,
    // so the battery checks element membership only).
    let value = match element.as_str() {
        "cmi.core.lesson_status" => bag.lesson_status,
        "cmi.core.score" => match bag.score {
            Some(n) => n.to_string(),
            None => "null".to_string(),
        },
        "cmi.suspend_data" => bag.suspend_data,
        "cmi.session_time" | "cmi.core.session_time" => bag.session_time.to_string(),
        other => {
            return error_response(&ScormError::Validation {
                field: other.to_string(),
                message: "unknown cmi element".to_string(),
            })
        }
    };
    Json(serde_json::json!({ "element": element, "value": value })).into_response()
}

/// LMSSetValue (FR-011 validation battery; the bag stays live).
async fn set_value(
    Extension(caller): Extension<ScormCaller>,
    AxumState(state): AxumState<RepoState>,
    Path(course_id): Path<String>,
    Json(body): Json<ElementBody>,
) -> Response {
    let (Some(element), Some(value)) = (body.element.filter(|e| !e.trim().is_empty()), body.value)
    else {
        return error_response(&ScormError::Validation {
            field: "element".to_string(),
            message: "element and value are required".to_string(),
        });
    };
    if let Err(e) = validate_value(&element, &value) {
        return error_response(&e);
    }
    let (mut bag, row) = match live_bag(&state, &caller, &course_id).await {
        Ok(v) => v,
        Err(e) => return error_response(&e),
    };
    match element.as_str() {
        "cmi.core.lesson_status" => bag.lesson_status = value,
        "cmi.core.score" => bag.score = Some(value.parse::<i32>().unwrap_or(0)),
        "cmi.suspend_data" => bag.suspend_data = value,
        "cmi.session_time" | "cmi.core.session_time" => {
            bag.session_time = value.parse::<i64>().unwrap_or(0)
        }
        _ => unreachable!("validate_value gates the element set"),
    }
    let json = match serde_json::to_string(&bag) {
        Ok(j) => j,
        Err(_) => return error_response(&ScormError::Unavailable),
    };
    if let Err(e) = state
        .bag()
        .set_bag(&caller.token_hash, &json, BAG_TTL_SECS)
        .await
    {
        return error_response(&e);
    }
    let _ = row; // the row is the attempt anchor (re-read on the next call)
                 // Echo the post-set bag (the value was moved into it above).
    Json(serde_json::json!({ "element": element, "bag": bag })).into_response()
}

/// LMSCommit (FR-005 — persist the committed state to the system of
/// record; the bag remains live in the session store).
async fn commit(
    Extension(caller): Extension<ScormCaller>,
    AxumState(state): AxumState<RepoState>,
    Path(course_id): Path<String>,
) -> Response {
    let (bag, row) = match live_bag(&state, &caller, &course_id).await {
        Ok(v) => v,
        Err(e) => return error_response(&e),
    };
    match state
        .repo()
        .update_committed(
            &row.id,
            &bag.lesson_status,
            &bag.suspend_data,
            bag.score,
            bag.session_time,
        )
        .await
    {
        Ok(()) => Json(serde_json::json!({ "committed": true })).into_response(),
        Err(e) => error_response(&e),
    }
}

/// LMSFinish (FR-005 CR-1 — an implicit final LMSCommit: the bag at finish
/// time is persisted BEFORE finalization; a Finish with no prior Commit is
/// the common path). `passed`/`completed` drive the 004 `enrolled →
/// completed` transition (first-wins through the port; a completion outage
/// is a 503 — never a fake success); `failed`/`incomplete` persist without
/// recording a completion. After this the attempt is final (409
/// `attempt-final` on further mutations).
async fn finish(
    Extension(caller): Extension<ScormCaller>,
    AxumState(state): AxumState<RepoState>,
    Path(course_id): Path<String>,
) -> Response {
    let (bag, row) = match live_bag(&state, &caller, &course_id).await {
        Ok(v) => v,
        Err(e) => return error_response(&e),
    };
    // The implicit final commit + finalization (one scoped write in the
    // real repo; the finished_at belt makes a second finish a 0-row no-op
    // → AttemptFinal).
    if let Err(e) = state
        .repo()
        .finalize(
            &row.id,
            &bag.lesson_status,
            &bag.suspend_data,
            bag.score,
            bag.session_time,
        )
        .await
    {
        return error_response(&e);
    }
    // The completion transition — ONLY for the passing final states
    // (first-wins is the 004 engine's guarantee, consumed through the
    // port; a port outage → 503, never a fake completion).
    let passing = matches!(bag.lesson_status.as_str(), "passed" | "completed");
    if passing {
        if let Err(e) = state
            .port()
            .record_completion(&caller.user_id, &course_id, &caller.organization_id)
            .await
        {
            return error_response(&e);
        }
    }
    Json(serde_json::json!({ "finished": true, "state": bag.lesson_status })).into_response()
}

/// The slice's routes (host mount pattern — one line in the host).
/// State-generic: the host's router state must provide `RepoState` via
/// `FromRef` (axum 0.8: no unit→stateful router upcast).
pub fn routes<S>() -> Router<S>
where
    S: Clone + Send + Sync + 'static,
    RepoState: FromRef<S>,
{
    Router::new()
        .route("/scorm/{course_id}/initialize", post(initialize))
        .route("/scorm/{course_id}/get", post(get_value))
        .route("/scorm/{course_id}/set", post(set_value))
        .route("/scorm/{course_id}/commit", post(commit))
        .route("/scorm/{course_id}/finish", post(finish))
}
