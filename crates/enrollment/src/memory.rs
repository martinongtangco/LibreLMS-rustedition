//! Deterministic double + the CourseSource fixture (T004 — red-first: the
//! tests in tests/repo.rs exist before the implementation).

use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::Mutex;

use crate::repo::{CourseSource, Enrollment, EnrollmentError, EnrollmentRepo};

/// Deterministic double (Principle XIII/XVII): all enrollment logic is
/// testable without containers; the full-env test proves the real engine
/// (the sequential duplicate check here is LOGIC-LAYER — the race proof is
/// the full-env test with N independent connections, plan PQ-1).
#[derive(Default)]
pub struct InMemoryEnrollmentRepo {
    #[allow(dead_code)] // red commit stub — the full implementation reads it
    rows: Arc<Mutex<HashMap<String, Enrollment>>>,
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
}

/// Stub for the red commit — replaced by the full implementation.
#[async_trait::async_trait]
impl EnrollmentRepo for InMemoryEnrollmentRepo {
    async fn create(
        &self,
        _learner_id: &str,
        _course_id: &str,
        _organization_id: &str,
    ) -> Result<Enrollment, EnrollmentError> {
        if self.is_down() {
            return Err(EnrollmentError::Unavailable);
        }
        Err(EnrollmentError::Validation {
            field: "stub",
            message: "red commit stub".into(),
        })
    }
    async fn list_for_learner(
        &self,
        _learner_id: &str,
        _organization_id: &str,
    ) -> Result<Vec<Enrollment>, EnrollmentError> {
        if self.is_down() {
            return Err(EnrollmentError::Unavailable);
        }
        Err(EnrollmentError::Validation {
            field: "stub",
            message: "red commit stub".into(),
        })
    }
    async fn list_for_org(
        &self,
        _organization_id: &str,
    ) -> Result<Vec<Enrollment>, EnrollmentError> {
        if self.is_down() {
            return Err(EnrollmentError::Unavailable);
        }
        Err(EnrollmentError::Validation {
            field: "stub",
            message: "red commit stub".into(),
        })
    }
    async fn complete(
        &self,
        _actor_id: &str,
        _enrollment_id: &str,
        _organization_id: &str,
    ) -> Result<Enrollment, EnrollmentError> {
        if self.is_down() {
            return Err(EnrollmentError::Unavailable);
        }
        Err(EnrollmentError::Validation {
            field: "stub",
            message: "red commit stub".into(),
        })
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

/// Stub for the red commit — replaced by the full implementation.
#[async_trait::async_trait]
impl CourseSource for InMemoryCourseSource {
    async fn course_title(&self, _course_id: &str) -> Result<Option<String>, EnrollmentError> {
        if self.is_down() {
            return Err(EnrollmentError::Unavailable);
        }
        Err(EnrollmentError::Validation {
            field: "stub",
            message: "red commit stub".into(),
        })
    }
    async fn course_exists_in_org(
        &self,
        _course_id: &str,
        _org_id: &str,
    ) -> Result<bool, EnrollmentError> {
        if self.is_down() {
            return Err(EnrollmentError::Unavailable);
        }
        Err(EnrollmentError::Validation {
            field: "stub",
            message: "red commit stub".into(),
        })
    }
}
