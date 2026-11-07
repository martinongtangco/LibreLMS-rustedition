//! Deterministic double + the CourseSource fixture (spec 004, T004).
//!
//! `InMemoryEnrollmentRepo` is the deterministic double (all enrollment logic
//! testable without containers); `InMemoryCourseSource` is the fixture for
//! the CourseSource port (the host wires the catalog adapter at Phase 4).
//! The sequential duplicate check here is LOGIC-LAYER — the race proof
//! (SC-003) is the full-env test with N independent connections (plan PQ-1).

use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::Mutex;

use crate::repo::{CourseSource, Enrollment, EnrollmentError, EnrollmentRepo, Status};

fn now_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// Deterministic double (Principle XIII/XVII): all enrollment logic is
/// testable without containers; the full-env test proves the real engine.
#[derive(Default)]
pub struct InMemoryEnrollmentRepo {
    rows: Arc<Mutex<HashMap<String, Enrollment>>>,
    /// Ids: deterministic counter within the double (real engine: NEWID()).
    next: Arc<std::sync::atomic::AtomicU64>,
    /// Test-only fault injection (the 001 MemorySessionStore / 003 pattern):
    /// lets the deterministic suite exercise the 503 path without a real
    /// outage.
    down: Arc<std::sync::atomic::AtomicBool>,
}

impl InMemoryEnrollmentRepo {
    pub fn new() -> Self {
        Self::default()
    }

    /// Test-only fault injection for the 503 path.
    pub fn set_unavailable_for_test(&self, down: bool) {
        self.down.store(down, std::sync::atomic::Ordering::SeqCst);
    }

    fn is_down(&self) -> bool {
        self.down.load(std::sync::atomic::Ordering::SeqCst)
    }

    fn check(&self) -> Result<(), EnrollmentError> {
        if self.is_down() {
            return Err(EnrollmentError::Unavailable);
        }
        Ok(())
    }
}

#[async_trait::async_trait]
impl EnrollmentRepo for InMemoryEnrollmentRepo {
    async fn create(
        &self,
        learner_id: &str,
        course_id: &str,
        organization_id: &str,
    ) -> Result<Enrollment, EnrollmentError> {
        self.check()?;
        let mut g = self.rows.lock().await;
        // LOGIC-LAYER duplicate check (the engine's UNIQUE constraint is the
        // real-arbiter analog — plan PQ-1).
        if g.values()
            .any(|e| e.learner_id == learner_id && e.course_id == course_id)
        {
            return Err(EnrollmentError::AlreadyEnrolled);
        }
        let n = self.next.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let row = Enrollment {
            id: format!("enr-{organization_id}-{n}"),
            learner_id: learner_id.to_string(),
            course_id: course_id.to_string(),
            organization_id: organization_id.to_string(),
            status: Status::Enrolled,
            created_at: now_secs(),
            completed_at: None,
        };
        g.insert(row.id.clone(), row.clone());
        Ok(row)
    }

    async fn list_for_learner(
        &self,
        learner_id: &str,
        organization_id: &str,
    ) -> Result<Vec<Enrollment>, EnrollmentError> {
        self.check()?;
        let g = self.rows.lock().await;
        let mut out: Vec<Enrollment> = g
            .values()
            .filter(|e| e.learner_id == learner_id && e.organization_id == organization_id)
            .cloned()
            .collect();
        out.sort_by(|a, b| a.id.cmp(&b.id));
        Ok(out)
    }

    async fn list_for_org(
        &self,
        organization_id: &str,
    ) -> Result<Vec<Enrollment>, EnrollmentError> {
        self.check()?;
        let g = self.rows.lock().await;
        let mut out: Vec<Enrollment> = g
            .values()
            .filter(|e| e.organization_id == organization_id)
            .cloned()
            .collect();
        out.sort_by(|a, b| a.id.cmp(&b.id));
        Ok(out)
    }

    async fn complete(
        &self,
        actor_id: &str,
        enrollment_id: &str,
        organization_id: &str,
    ) -> Result<Enrollment, EnrollmentError> {
        self.check()?;
        let mut g = self.rows.lock().await;
        let row = g
            .get(enrollment_id)
            .cloned()
            .ok_or(EnrollmentError::NotFound)?;
        // Scope discipline (FR-003 both directions; US3.3 — nothing persisted
        // on rejection).
        if row.organization_id != organization_id {
            return Err(EnrollmentError::Scope);
        }
        // Only the enrolled learner records their own completion.
        if row.learner_id != actor_id {
            return Err(EnrollmentError::Forbidden);
        }
        match row.status {
            Status::Completed => {
                // First-wins: the original record is unchanged (no UPDATE
                // path — the PK's double analog).
                Err(EnrollmentError::AlreadyCompleted)
            }
            Status::Enrolled => {
                let mut done = row;
                done.status = Status::Completed;
                done.completed_at = Some(now_secs());
                g.insert(done.id.clone(), done.clone());
                Ok(done)
            }
        }
    }
}

/// The CourseSource fixture (deterministic tests; the host wires the catalog
/// adapter at Phase 4 against the b4f3008 provider shape).
#[derive(Default)]
pub struct InMemoryCourseSource {
    courses: Arc<Mutex<HashMap<String, (String, String)>>>,
    down: Arc<std::sync::atomic::AtomicBool>,
}

impl InMemoryCourseSource {
    pub fn new() -> Self {
        Self::default()
    }

    /// Test-only seeding (bypasses nothing — the fixture IS the data).
    pub async fn seed(&self, course_id: &str, organization_id: &str, title: &str) {
        self.courses.lock().await.insert(
            course_id.to_string(),
            (organization_id.to_string(), title.to_string()),
        );
    }

    /// Test-only fault injection for the 503 path.
    pub fn set_unavailable_for_test(&self, down: bool) {
        self.down.store(down, std::sync::atomic::Ordering::SeqCst);
    }

    fn is_down(&self) -> bool {
        self.down.load(std::sync::atomic::Ordering::SeqCst)
    }
}

#[async_trait::async_trait]
impl CourseSource for InMemoryCourseSource {
    async fn course_title(&self, course_id: &str) -> Result<Option<String>, EnrollmentError> {
        // Outage discipline (plan N1): outage → Err, never Ok(None).
        if self.is_down() {
            return Err(EnrollmentError::Unavailable);
        }
        Ok(self
            .courses
            .lock()
            .await
            .get(course_id)
            .map(|(_, title)| title.clone()))
    }

    async fn course_exists_in_org(
        &self,
        course_id: &str,
        org_id: &str,
    ) -> Result<bool, EnrollmentError> {
        if self.is_down() {
            return Err(EnrollmentError::Unavailable);
        }
        Ok(self
            .courses
            .lock()
            .await
            .get(course_id)
            .is_some_and(|(org, _)| org == org_id))
    }
}
