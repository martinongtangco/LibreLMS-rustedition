//! Enrollment HTTP surface (spec 004). The routes are state-bound to the
//! repo + port (host mount pattern: the host does
//! `router.merge(enrollment::routes(...))`); the caller (user + org + role)
//! arrives as an `Extension<EnrollmentCaller>` inserted by the host's auth
//! middleware — the module never depends on host types (Principle III).
//!
//! Stub for the red commit — the full implementation (T009/T012/T015)
//! implements POST /enrollments, GET /enrollments (own + org-wide) and
//! POST /enrollments/{id}/completion with the plan's envelope mapping.

use std::sync::Arc;

use axum::extract::FromRef;
use axum::Router;

use crate::repo::{CourseSource, EnrollmentRepo};

/// What the host's auth middleware resolves the session to (enrollment-owned
/// shape so the module stays host-independent — Principle III).
#[derive(Debug, Clone)]
pub struct EnrollmentCaller {
    pub user_id: String,
    pub organization_id: String,
    pub role: contracts::Role,
}

/// State holder (satisfies axum 0.8's `State(S: Clone + Send + Sync)` bound).
#[derive(Clone)]
pub struct RepoState {
    #[allow(dead_code)] // red commit stub — the full routes read them
    repo: Arc<dyn EnrollmentRepo>,
    #[allow(dead_code)] // red commit stub — the full routes read them
    source: Arc<dyn CourseSource>,
}

impl RepoState {
    pub fn new(repo: Arc<dyn EnrollmentRepo>, source: Arc<dyn CourseSource>) -> Self {
        Self { repo, source }
    }

    #[allow(dead_code)] // red commit stub
    fn repo(&self) -> &Arc<dyn EnrollmentRepo> {
        &self.repo
    }
    #[allow(dead_code)] // red commit stub
    fn source(&self) -> &Arc<dyn CourseSource> {
        &self.source
    }
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
    // Red commit: the routes exist but 404 — the flow tests go RED here.
}
