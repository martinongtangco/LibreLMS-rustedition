//! Phase 4 cross-slice adapters (host composition point — Principle III:
//! module crates never depend on each other; the host wires the ports).
//!
//! `CatalogCourseSource` wires 004's `CourseSource` port to the 003
//! catalog provider (the PQ-2 agreed shape — the catalog's `find_by_id`
//! is the unscoped by-id seam; scope is the caller's decision via the
//! 404/403 decision procedure, carried verbatim from the 004 plan):
//! 1. `course_title(id) = None` → 404-class (unknown course).
//! 2. `Some(_)` + `course_exists_in_org(id, org) = false` → 403-class
//!    (content-identical to any other 403 — no existence oracle).
//! 3. `Some` + `true` → proceed.
//!
//! Outage discipline (004 N1, extended): an outage MUST be
//! `Err(Unavailable)` — never `Ok(None)`/`Ok(false)` (an outage must
//! never read as "absent" or "out of scope").
//!
//! `EnrollmentRepoPort` wires 005's `EnrollmentPort` to the 004
//! repository (the FR-008 host-wired port) + the 003 catalog (for the
//! 404/403 course check the port's error type carries, FR-002). First-
//! completion-wins is the 004 engine's guarantee (`dbo.completions` PK):
//! an `AlreadyCompleted` from a racing/second call maps to an IDEMPOTENT
//! `Ok(())` — never an error, never a second timestamp.

use std::sync::Arc;

use async_trait::async_trait;
use enrollment::repo::{CourseSource, EnrollmentError, EnrollmentRepo, Status};
use scorm::repo::{EnrollmentPort, LaunchGate, ScormError};

use self::adapters_err::{map_catalog, map_enrollment};

mod adapters_err {
    //! Adapter error mapping (never a fake 404/403/success on an outage).
    use catalog::repo::CatalogError;
    use enrollment::repo::EnrollmentError;
    use scorm::repo::ScormError;

    /// Catalog → enrollment-port error (the CourseSource leg). `NotFound`
    /// cannot come from `find_by_id` (absent is `Ok(None)`); the other
    /// variants are unexpected server states → 503, never a fake 404/403.
    pub(crate) fn map_catalog(e: CatalogError) -> EnrollmentError {
        match e {
            CatalogError::Unavailable => EnrollmentError::Unavailable,
            _ => EnrollmentError::Unavailable,
        }
    }

    /// Enrollment → scorm-port error (the state/completion leg). Outage
    /// stays an outage; the 004 guard errors map to their 005 analogs.
    pub(crate) fn map_enrollment(e: EnrollmentError) -> ScormError {
        match e {
            EnrollmentError::Unavailable => ScormError::Unavailable,
            EnrollmentError::Scope => ScormError::Scope,
            EnrollmentError::Forbidden => ScormError::Scope,
            EnrollmentError::NotFound => ScormError::NotFound,
            // AlreadyEnrolled/NotEnrolled/Validation/AlreadyCompleted are
            // not produced by list_for_learner; defensively: an unexpected
            // guard state is an outage-class 503, never a fake gate.
            _ => ScormError::Unavailable,
        }
    }
}

/// 004 `CourseSource` ← 003 catalog (the host's 1:1 adapter — the spec 003
/// plan's PQ-2 seam).
pub struct CatalogCourseSource {
    catalog: Arc<dyn catalog::repo::CourseRepo>,
}

impl CatalogCourseSource {
    pub fn new(catalog: Arc<dyn catalog::repo::CourseRepo>) -> Self {
        Self { catalog }
    }
}

#[async_trait]
impl CourseSource for CatalogCourseSource {
    async fn course_title(&self, course_id: &str) -> Result<Option<String>, EnrollmentError> {
        // The unscoped by-id seam (PQ-2): Ok(None) = absent (valid 404
        // signal ONLY because an outage is Err below), Err = outage (503 —
        // never a fake Ok(None)).
        match self.catalog.find_by_id(course_id).await {
            Ok(None) => Ok(None),
            Ok(Some(c)) => Ok(Some(c.title)),
            Err(e) => Err(map_catalog(e)),
        }
    }

    async fn course_exists_in_org(
        &self,
        course_id: &str,
        org_id: &str,
    ) -> Result<bool, EnrollmentError> {
        // The scope decision is the CALLER's (the decision procedure): the
        // adapter reports the course's org binding; an outage is never a
        // fake `false` (that would read as 403 — an existence-oracle leak
        // through the outage path).
        match self.catalog.find_by_id(course_id).await {
            Ok(None) => Ok(false),
            Ok(Some(c)) => Ok(c.organization_id == org_id),
            Err(e) => Err(map_catalog(e)),
        }
    }
}

/// 005 `EnrollmentPort` ← 004 repository (+ the 003 catalog for the
/// port's 404/403 course check, FR-002).
pub struct EnrollmentRepoPort {
    enrollment: Arc<dyn EnrollmentRepo>,
    course_source: Arc<dyn CourseSource>,
}

impl EnrollmentRepoPort {
    pub fn new(enrollment: Arc<dyn EnrollmentRepo>, course_source: Arc<dyn CourseSource>) -> Self {
        Self {
            enrollment,
            course_source,
        }
    }
}

#[async_trait]
impl EnrollmentPort for EnrollmentRepoPort {
    async fn enrollment_status(
        &self,
        learner_id: &str,
        course_id: &str,
        org_id: &str,
    ) -> Result<LaunchGate, ScormError> {
        // Step 1 — the 404/403 course check (the decision procedure,
        // verbatim): title None → NotFound; title Some + exists_in_org
        // false → Scope. An outage at either step is Unavailable (N1 —
        // never a fake gate outcome).
        match self.course_source.course_title(course_id).await {
            Ok(None) => return Err(ScormError::NotFound),
            Ok(Some(_)) => {
                let in_org = self
                    .course_source
                    .course_exists_in_org(course_id, org_id)
                    .await
                    // An outage is never a fake `false` (that would read
                    // as 403 — an existence-oracle leak through the
                    // outage path).
                    .map_err(|_| ScormError::Unavailable)?;
                if !in_org {
                    return Err(ScormError::Scope);
                }
            }
            // An outage at either step is Unavailable (N1 — never a fake
            // gate outcome); all CourseSource errors map to the 503 class.
            Err(_) => return Err(ScormError::Unavailable),
        }
        // Step 2 — the enrollment state (the 004 two-state lifecycle).
        let list = self
            .enrollment
            .list_for_learner(learner_id, org_id)
            .await
            .map_err(map_enrollment)?;
        match list.iter().find(|e| e.course_id == course_id) {
            Some(e) => match e.status {
                Status::Completed => Ok(LaunchGate::Completed),
                Status::Enrolled => Ok(LaunchGate::Enrolled),
            },
            None => Ok(LaunchGate::NotEnrolled),
        }
    }

    async fn record_completion(
        &self,
        learner_id: &str,
        course_id: &str,
        org_id: &str,
    ) -> Result<(), ScormError> {
        // The 004 `enrolled → completed` transition. The enrollment is
        // located by (learner, course) in the session org; the transition
        // itself is the 004 engine's (the `dbo.completions` PK makes a
        // second call an idempotent no-op — mapped below).
        let list = self
            .enrollment
            .list_for_learner(learner_id, org_id)
            .await
            .map_err(map_enrollment)?;
        let enrollment = match list.iter().find(|e| e.course_id == course_id) {
            Some(e) => e,
            None => return Err(ScormError::NotEnrolled),
        };
        match self
            .enrollment
            .complete(learner_id, &enrollment.id, org_id)
            .await
        {
            Ok(_) => Ok(()),
            // First-completion-wins (the 004 engine anchor): a racing or
            // second call is an IDEMPOTENT no-op — never an error, never
            // a second timestamp (the port contract).
            Err(EnrollmentError::AlreadyCompleted) => Ok(()),
            Err(EnrollmentError::NotEnrolled) => Err(ScormError::NotEnrolled),
            Err(e) => Err(match e {
                EnrollmentError::Unavailable => ScormError::Unavailable,
                EnrollmentError::Scope => ScormError::Scope,
                EnrollmentError::Forbidden => ScormError::Scope,
                EnrollmentError::NotFound => ScormError::NotFound,
                // AlreadyEnrolled/Validation/AlreadyCompleted are not
                // produced by complete (AlreadyCompleted handled above);
                // defensively: 503, never a fake success.
                _ => ScormError::Unavailable,
            }),
        }
    }
}
