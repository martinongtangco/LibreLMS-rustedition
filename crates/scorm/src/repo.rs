//! SCORM repository + ports (spec 005, Module Persistence Seam).
//!
//! `ScormRepo` is the in-crate repository trait (001 CR-2 seam decision);
//! `InMemoryScormRepo` (memory.rs) is the deterministic double,
//! `TiberiusScormRepo` (tiberius.rs) is the real engine. `EnrollmentPort`
//! is the 2-method, host-wired port (the 004 `CourseSource` parity — no
//! enrollment-crate dependency, Principle III); `BagStore` is the module
//! owned bag port (FR-003 — production wraps the shared session store at the
//! 001-reserved key; the port never names the key).

use serde::{Deserialize, Serialize};

/// The five-value `cmi.core.lesson_status` subset (spec 005 FR-004).
pub const LESSON_STATUS_VALUES: [&str; 5] =
    ["in_progress", "passed", "failed", "completed", "incomplete"];

/// App-level `cmi.suspend_data` cap (chars — the engine cap on non-MAX
/// NVARCHAR is 4,000; the column is NVARCHAR(MAX), FR-011).
pub const SUSPEND_DATA_MAX_CHARS: usize = 4_000;

/// The cmi data subset (spec 005 FR-004). The bag is always complete —
/// init defaults: `in_progress` / no score / empty suspend / 0s.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct CmiBag {
    pub lesson_status: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub score: Option<i32>,
    #[serde(default)]
    pub suspend_data: String,
    #[serde(default)]
    pub session_time: i64,
}

impl CmiBag {
    /// The init default (a fresh `in_progress` attempt, FR-001/FR-006).
    pub fn fresh() -> Self {
        Self {
            lesson_status: "in_progress".to_string(),
            score: None,
            suspend_data: String::new(),
            session_time: 0,
        }
    }
}

/// Validation battery (FR-011) — the 400s name the element.
pub fn validate_value(element: &str, value: &str) -> Result<(), ScormError> {
    match element {
        "cmi.core.lesson_status" => {
            if LESSON_STATUS_VALUES.contains(&value) {
                Ok(())
            } else {
                Err(ScormError::Validation {
                    field: element.to_string(),
                    message: format!("must be one of {}", LESSON_STATUS_VALUES.join(" | ")),
                })
            }
        }
        "cmi.core.score" => {
            // Raw 0-100 integer (scaled = raw/100 is computed, not stored).
            match value.parse::<i32>() {
                Ok(n) if (0..=100).contains(&n) => Ok(()),
                _ => Err(ScormError::Validation {
                    field: element.to_string(),
                    message: "must be an integer in 0..=100".to_string(),
                }),
            }
        }
        "cmi.suspend_data" => {
            if value.chars().count() <= SUSPEND_DATA_MAX_CHARS {
                Ok(())
            } else {
                Err(ScormError::Validation {
                    field: element.to_string(),
                    message: format!(
                        "exceeds {SUSPEND_DATA_MAX_CHARS} chars ({})",
                        value.chars().count()
                    ),
                })
            }
        }
        "cmi.session_time" | "cmi.core.session_time" => {
            // Accumulated seconds (non-negative integer).
            match value.parse::<i64>() {
                Ok(n) if n >= 0 => Ok(()),
                _ => Err(ScormError::Validation {
                    field: element.to_string(),
                    message: "must be a non-negative integer (seconds)".to_string(),
                }),
            }
        }
        other => Err(ScormError::Validation {
            field: other.to_string(),
            message: "unknown cmi element".to_string(),
        }),
    }
}

/// SCORM failures → envelope mapping (host): 404 / 403 / 409 / 409 / 409 /
/// 400(field) / 503 — never a 500, never a silent state loss (FR-010).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScormError {
    /// The course does not exist (404 — the port's NotFound; a GENUINELY
    /// absent course, not an outage — the N1 discipline).
    NotFound,
    /// The course exists but belongs to another organization (403 — the
    /// no-existence-oracle rule: content-identical to any other 403).
    Scope,
    /// The learner is not enrolled in the course (409 `not-enrolled` —
    /// stable code, never a 404; the port's Ok(NotEnrolled)).
    NotEnrolled,
    /// The enrollment is already completed (409 `already-completed` — the
    /// 004 first-completion-wins guarantee; US1.2).
    AlreadyCompleted,
    /// An operation on a finished attempt (409 `attempt-final`, FR-005).
    AttemptFinal,
    /// An attempt for this (session, course) already exists (internal —
    /// the 2601/2627 unique-violation mapping; the loser reads the winner's
    /// row, US1.6 — never an error response, never a second row).
    AttemptExists,
    /// Invalid `cmi` input; the message names the offending element (400).
    Validation { field: String, message: String },
    /// The system of record OR the session store is unreachable (503 —
    /// never 500, never a fake success).
    Unavailable,
}

impl std::fmt::Display for ScormError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Stable, non-leaky (spec 001 FR-009 convention).
        match self {
            Self::NotFound => write!(f, "course not found"),
            Self::Scope => write!(f, "organization scope mismatch"),
            Self::NotEnrolled => write!(f, "not enrolled"),
            Self::AlreadyCompleted => write!(f, "enrollment already completed"),
            Self::AttemptFinal => write!(f, "attempt is final"),
            Self::AttemptExists => write!(f, "attempt already exists"),
            Self::Validation { field, .. } => write!(f, "invalid input: {field}"),
            Self::Unavailable => write!(f, "storage unavailable"),
        }
    }
}

/// A SCORM attempt row (crate-owned shape — `contracts` is frozen post-001;
/// the frozen `ScormAttemptStatus` placeholder is the contracts shape the
/// slice consumes/produces at its API edge).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScormAttempt {
    pub id: String,
    /// The 001 session token_hash (session-scoped attempt).
    pub session_id: String,
    pub course_id: String,
    pub organization_id: String,
    /// The five-value `cmi.core.lesson_status` subset.
    pub state: String,
    pub suspend_data: String,
    pub score_raw: Option<i32>,
    pub session_time: i64,
    /// `None` = live attempt, `Some` = final (the app-layer
    /// `attempt-final` discriminator — plan decision).
    pub finished_at: Option<i64>,
    pub created_at: i64,
}

/// The launch gate (plan Cross-Slice Contract): the Ok payload carries the
/// 409 distinction, the error type the 404/403/503 distinction.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum LaunchGate {
    /// Enrolled and still `enrolled` — launch may proceed.
    Enrolled,
    /// Not enrolled (409 `not-enrolled` — stable code, never a 404).
    NotEnrolled,
    /// Already completed (409 `already-completed` — US1.2; the prior
    /// committed attempt state remains the record).
    Completed,
}

/// The enrollment seam (2-method, host-wired — the 004 `CourseSource`
/// parity; the host wires the 004 repository adapter at Phase 4, tests use
/// `DoubleEnrollmentPort`, the honest `UnwiredEnrollmentPort` is the
/// production stub until then).
///
/// Outage discipline (004 N1 parity): an outage MUST be `Err(Unavailable)`
/// → 503 — never `Ok(NotEnrolled)`-as-404/409, never a fake 404/200/
/// completion.
#[async_trait::async_trait]
pub trait EnrollmentPort: Send + Sync {
    /// `Err(NotFound)` = the course does not exist (→ 404-class, FR-002).
    /// `Err(Scope)` = the course exists but is not in `org_id` (→ 403-class
    /// — content-identical to any other 403, no existence oracle).
    /// `Ok(NotEnrolled)` = 409 `not-enrolled`.
    /// `Ok(Completed)` = 409 `already-completed` (the 004 first-completion-
    /// wins guarantee).
    /// `Err(Unavailable)` = 503 (N1 — never conflated with absence).
    async fn enrollment_status(
        &self,
        learner_id: &str,
        course_id: &str,
        org_id: &str,
    ) -> Result<LaunchGate, ScormError>;

    /// Drives the 004 `enrolled → completed` transition (FR-005).
    /// First-completion-wins is enforced by the 004 engine
    /// (`dbo.completions` PK) — a second call is an idempotent no-op,
    /// never an error and never a second timestamp. `Err(Unavailable)` =
    /// 503 — a completion outage is recorded as an outage, never a fake
    /// success.
    async fn record_completion(
        &self,
        learner_id: &str,
        course_id: &str,
        org_id: &str,
    ) -> Result<(), ScormError>;
}

/// The module-owned bag port (FR-003). Production wraps the shared session
/// store at the 001-reserved key `lms:session:{token_hash}:bag` (the key
/// stays in `shared` ground truth — the port never names it); deterministic
/// tests use `InMemoryBagStore`.
#[async_trait::async_trait]
pub trait BagStore: Send + Sync {
    /// `None` if absent or expired (the bag is ephemeral by design — a
    /// store flush loses only the bag; the session row survives).
    async fn get_bag(&self, token_hash: &str) -> Result<Option<String>, ScormError>;
    /// Store with TTL (= session lifetime, refreshed with the sliding
    /// expiry — the shared store's Principle).
    async fn set_bag(&self, token_hash: &str, bag: &str, ttl_secs: u64) -> Result<(), ScormError>;
}

/// The SCORM repository seam (in-crate per the Module Persistence Seam).
#[async_trait::async_trait]
pub trait ScormRepo: Send + Sync {
    /// The UNIQUE (session, course) lookup — the winner's row read-back
    /// path (US1.6) and the idempotent-resume read (FR-001).
    async fn find_by_session_course(
        &self,
        session_id: &str,
        course_id: &str,
    ) -> Result<Option<ScormAttempt>, ScormError>;

    /// Create the attempt row (state `in_progress`, no score, empty
    /// suspend, 0s — the CmiBag::fresh shape). The UNIQUE (session,
    /// course) constraint is the US1.6 engine arbiter under concurrency:
    /// a unique-constraint violation (MSSQL 2601/2627 — both codes, both
    /// error legs) maps to `AttemptExists`, never a 500, never a second
    /// row (the loser reads the winner's row).
    async fn create(&self, attempt: &ScormAttempt) -> Result<(), ScormError>;

    /// LMSCommit persistence (FR-005): the committed state
    /// (state, suspend_data, score_raw, session_time) to the system of
    /// record; the bag stays live in the session store.
    async fn update_committed(
        &self,
        id: &str,
        state: &str,
        suspend_data: &str,
        score_raw: Option<i32>,
        session_time: i64,
    ) -> Result<(), ScormError>;

    /// LMSFinish — the implicit final LMSCommit (FR-005 CR-1): persists
    /// the committed state AND sets `finished_at` (exactly once — the
    /// first-wins belt; the 004 engine owns the completion record).
    async fn finalize(
        &self,
        id: &str,
        state: &str,
        suspend_data: &str,
        score_raw: Option<i32>,
        session_time: i64,
    ) -> Result<(), ScormError>;
}
