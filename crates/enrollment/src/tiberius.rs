//! The real-engine `EnrollmentRepo` (spec 004, Module Persistence Seam):
//! tiberius against `dbo.enrollments` / `dbo.completions` (0005_enrollment.sql).
//!
//! Stub for the red commit — the full implementation (T005) maps the
//! unique-constraint violation (MSSQL 2627) to `AlreadyEnrolled` and the
//! completion PK violation to `AlreadyCompleted` (never a 500, never a
//! second row/record — plan PQ-1).

use std::sync::Arc;
use tokio::sync::Mutex;

use crate::repo::{Enrollment, EnrollmentError, EnrollmentRepo};

pub struct TiberiusEnrollmentRepo {
    client: Arc<Mutex<Option<tiberius::Client<tokio_util::compat::Compat<tokio::net::TcpStream>>>>>,
}

impl TiberiusEnrollmentRepo {
    /// Stub — T005.
    pub async fn connect(_url: &str) -> Result<Self, EnrollmentError> {
        Err(EnrollmentError::Unavailable)
    }

    async fn guard(
        &self,
    ) -> Result<
        tokio::sync::MutexGuard<
            '_,
            Option<tiberius::Client<tokio_util::compat::Compat<tokio::net::TcpStream>>>,
        >,
        EnrollmentError,
    > {
        Ok(self.client.lock().await)
    }

    /// Production connection proof (mirrors 001/003) — stub for T005.
    pub async fn verify(&self) -> Result<(), EnrollmentError> {
        let _ = self.guard().await?;
        Err(EnrollmentError::Unavailable)
    }
}

#[async_trait::async_trait]
impl EnrollmentRepo for TiberiusEnrollmentRepo {
    async fn create(
        &self,
        _learner_id: &str,
        _course_id: &str,
        _organization_id: &str,
    ) -> Result<Enrollment, EnrollmentError> {
        Err(EnrollmentError::Unavailable)
    }
    async fn list_for_learner(
        &self,
        _learner_id: &str,
        _organization_id: &str,
    ) -> Result<Vec<Enrollment>, EnrollmentError> {
        Err(EnrollmentError::Unavailable)
    }
    async fn list_for_org(
        &self,
        _organization_id: &str,
    ) -> Result<Vec<Enrollment>, EnrollmentError> {
        Err(EnrollmentError::Unavailable)
    }
    async fn complete(
        &self,
        _actor_id: &str,
        _enrollment_id: &str,
        _organization_id: &str,
    ) -> Result<Enrollment, EnrollmentError> {
        Err(EnrollmentError::Unavailable)
    }
}
