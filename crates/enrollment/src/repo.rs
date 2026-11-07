//! Enrollment repository (spec 004, Module Persistence Seam).
//!
//! The trait is defined IN THIS crate (the seam decision from spec 001 CR-2);
//! `InMemoryEnrollmentRepo` is the deterministic double, `TiberiusEnrollmentRepo`
//! (tiberius.rs) is the real engine. Scope/role rules live here, not in the
//! routes. Course data arrives through the `CourseSource` port — never a
//! catalog-crate dependency (Principle III).

use serde::Serialize;

/// Enrollment lifecycle (spec 004: two states only — no withdrawal /
/// re-enroll / waitlist; the SCORM slice (spec 005) drives the same
/// enrolled→completed transition later).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Status {
    Enrolled,
    Completed,
}

/// An enrollment row (crate-owned shape — `contracts` is frozen post-001).
/// Timestamps are unix seconds (001 wire convention).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Enrollment {
    pub id: String,
    pub learner_id: String,
    pub course_id: String,
    pub organization_id: String,
    pub status: Status,
    pub created_at: i64,
    /// The first (and only) completion's timestamp (spec US3.2 first-wins).
    pub completed_at: Option<i64>,
}

/// The API view (US2): the enrollment + the course title resolved through
/// the `CourseSource` port. `title = None` = dangling reference (the course
/// was deleted — spec 003 CR-2 assumption: enrollments persist, dangling
/// reference handling is the Phase 4 integration decision).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct EnrollmentView {
    pub id: String,
    pub learner_id: String,
    pub course_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    pub status: Status,
    pub created_at: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub completed_at: Option<i64>,
}

/// Enrollment failures → envelope mapping (host): 404 / 403 / 403 / 400(field)
/// / 409 / 409 / 409 / 503.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EnrollmentError {
    /// The resource exists but belongs to another organization (403 — the
    /// no-existence-oracle rule: out-of-scope answers 403, not 404).
    Scope,
    /// The session's role/actor does not allow the action (403: non-admin
    /// org-wide view; a non-learner completing someone else's enrollment).
    Forbidden,
    /// Unknown enrollment id (genuinely absent — 404, no internals).
    NotFound,
    /// (learner, course) already enrolled (409 — the engine's UNIQUE
    /// constraint is the final arbiter under concurrency; the double enforces
    /// the same rule sequentially).
    AlreadyEnrolled,
    /// Completion attempted on an enrollment that is not "enrolled" (409).
    NotEnrolled,
    /// Completion attempted a second time (409 — first-wins; the original
    /// completion record is unchanged).
    AlreadyCompleted,
    /// Invalid input; the message names the offending field (400).
    Validation {
        field: &'static str,
        message: String,
    },
    /// The system of record is unreachable (503 — never 500, never a silent
    /// empty list).
    Unavailable,
}

/// The course seam (PQ-2 agreed shape — the provider's 2-method minimal set;
/// the spec 003 plan mirrors it in `CourseRepo`, the host adapts 1:1).
///
/// 404/403 decision procedure (plan, per call site):
/// 1. `course_title(id) = None` → 404-class (unknown course).
/// 2. `course_title(id) = Some(_)` + `course_exists_in_org(id, org) = false`
///    → 403-class (out-of-scope; content-identical to any other 403).
/// 3. `Some` + `true` → proceed; `title` is available for display.
///
/// Outage discipline (plan N1): an outage MUST be `Err(Unavailable)` → 503,
/// never `Ok(None)` — step 1's `None → 404` is only valid because an ABSENT
/// course returns `None` for everyone.
#[async_trait::async_trait]
pub trait CourseSource: Send + Sync {
    /// `None` = the course does not exist (→ 404-class).
    async fn course_title(&self, course_id: &str) -> Result<Option<String>, EnrollmentError>;
    /// `true` only if the course exists AND belongs to `org_id`.
    async fn course_exists_in_org(
        &self,
        course_id: &str,
        org_id: &str,
    ) -> Result<bool, EnrollmentError>;
}

/// The enrollment repository seam (in-crate per the Module Persistence Seam).
#[async_trait::async_trait]
pub trait EnrollmentRepo: Send + Sync {
    /// Create an enrollment (status `enrolled`). A duplicate (learner, course)
    /// → `AlreadyEnrolled` (single row; the engine's UNIQUE constraint is the
    /// final arbiter under concurrency — plan PQ-1).
    async fn create(
        &self,
        learner_id: &str,
        course_id: &str,
        organization_id: &str,
    ) -> Result<Enrollment, EnrollmentError>;

    /// The actor's own enrollments in the session organization (org-scoped).
    /// Empty → `Ok(vec![])` (not an error).
    async fn list_for_learner(
        &self,
        learner_id: &str,
        organization_id: &str,
    ) -> Result<Vec<Enrollment>, EnrollmentError>;

    /// The organization-wide view (admin-gated at the route).
    async fn list_for_org(&self, organization_id: &str)
        -> Result<Vec<Enrollment>, EnrollmentError>;

    /// Record completion: the enrolled→completed transition with a timestamped
    /// completion record (first-wins — the `enrollment_id` PK is the engine
    /// anchor; no UPDATE path). Only the enrolled learner may record it
    /// (spec US3: "the learner's completion of an enrolled course").
    /// Out-of-scope → `Scope` (nothing persisted); unknown id → `NotFound`.
    async fn complete(
        &self,
        actor_id: &str,
        enrollment_id: &str,
        organization_id: &str,
    ) -> Result<Enrollment, EnrollmentError>;
}
