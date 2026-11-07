//! Enrollment HTTP surface (spec 004). The routes are state-bound to the
//! repo and port (host mount pattern: the host merges this router and
//! inserts the auth-resolved caller as an extension) — the module never
//! depends on host types (Principle III).

use std::sync::Arc;

use axum::extract::{FromRef, Path, Query, State as AxumState};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Extension, Json, Router};
use serde::Deserialize;

use contracts::ErrorEnvelope;

use crate::repo::{CourseSource, Enrollment, EnrollmentError, EnrollmentRepo, EnrollmentView};

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
    repo: Arc<dyn EnrollmentRepo>,
    source: Arc<dyn CourseSource>,
}

impl RepoState {
    pub fn new(repo: Arc<dyn EnrollmentRepo>, source: Arc<dyn CourseSource>) -> Self {
        Self { repo, source }
    }

    fn repo(&self) -> &Arc<dyn EnrollmentRepo> {
        &self.repo
    }
    fn source(&self) -> &Arc<dyn CourseSource> {
        &self.source
    }
}

#[derive(Deserialize)]
struct EnrollBody {
    course_id: Option<String>,
}

#[derive(Deserialize)]
struct ListQuery {
    scope: Option<String>,
}

/// Error envelope mapping (plan table — hyphen stable codes, 001 api.md
/// style; never leaks backend internals).
pub fn error_response(e: &EnrollmentError) -> Response {
    let (status, code, message) = match e {
        EnrollmentError::NotFound => (
            axum::http::StatusCode::NOT_FOUND,
            "not-found".to_string(),
            "enrollment not found".to_string(),
        ),
        EnrollmentError::Scope => (
            axum::http::StatusCode::FORBIDDEN,
            "scope".to_string(),
            "organization scope mismatch".to_string(),
        ),
        EnrollmentError::Forbidden => (
            axum::http::StatusCode::FORBIDDEN,
            "forbidden".to_string(),
            "actor does not allow this action".to_string(),
        ),
        EnrollmentError::AlreadyEnrolled => (
            axum::http::StatusCode::CONFLICT,
            "already-enrolled".to_string(),
            "already enrolled".to_string(),
        ),
        EnrollmentError::NotEnrolled => (
            axum::http::StatusCode::CONFLICT,
            "not-enrolled".to_string(),
            "enrollment is not in enrolled state".to_string(),
        ),
        EnrollmentError::AlreadyCompleted => (
            axum::http::StatusCode::CONFLICT,
            "already-completed".to_string(),
            "completion was already recorded".to_string(),
        ),
        EnrollmentError::Validation { field, message } => (
            axum::http::StatusCode::BAD_REQUEST,
            "validation".to_string(),
            format!("{field}: {message}"),
        ),
        EnrollmentError::Unavailable => (
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

fn require_admin(role: &contracts::Role) -> Result<(), EnrollmentError> {
    if *role == contracts::Role::Admin {
        Ok(())
    } else {
        Err(EnrollmentError::Forbidden)
    }
}

/// Resolve the course title for a view (N1: a port outage is a 503 for the
/// whole view — never a fake absence; a dangling course title is None).
async fn title_or_err(
    source: &Arc<dyn CourseSource>,
    course_id: &str,
) -> Result<Option<String>, EnrollmentError> {
    source.course_title(course_id).await
}

/// US1: POST /enrollments — the 404/403 decision procedure (plan), then the
/// repository create (409 on the UNIQUE constraint).
async fn enroll(
    Extension(caller): Extension<EnrollmentCaller>,
    AxumState(state): AxumState<RepoState>,
    Json(body): Json<EnrollBody>,
) -> Response {
    let Some(course_id) = body.course_id.filter(|c| !c.trim().is_empty()) else {
        return error_response(&EnrollmentError::Validation {
            field: "course_id",
            message: "course_id is required".to_string(),
        });
    };
    // Decision procedure step 1: absent → 404 (outage → 503, never a fake
    // 404 — plan N1).
    match state.source().course_title(&course_id).await {
        Ok(None) => return error_response(&EnrollmentError::NotFound),
        Ok(Some(_)) => {}
        Err(e) => return error_response(&e),
    }
    // Step 2: exists + out-of-scope → 403 (content-identical to any other
    // 403 — no existence oracle).
    let in_org = match state
        .source()
        .course_exists_in_org(&course_id, &caller.organization_id)
        .await
    {
        Ok(b) => b,
        Err(e) => return error_response(&e),
    };
    if !in_org {
        return error_response(&EnrollmentError::Scope);
    }
    match state
        .repo()
        .create(&caller.user_id, &course_id, &caller.organization_id)
        .await
    {
        Ok(e) => (axum::http::StatusCode::CREATED, Json(e)).into_response(),
        Err(e) => error_response(&e),
    }
}

/// US2: GET /enrollments — own list (any member) or the org-wide view
/// (admin-only, `?scope=org`).
async fn list(
    Extension(caller): Extension<EnrollmentCaller>,
    AxumState(state): AxumState<RepoState>,
    Query(q): Query<ListQuery>,
) -> Response {
    let org_wide = q.scope.as_deref() == Some("org");
    if org_wide {
        if let Err(e) = require_admin(&caller.role) {
            return error_response(&e);
        }
    }
    let rows = if org_wide {
        state.repo().list_for_org(&caller.organization_id).await
    } else {
        state
            .repo()
            .list_for_learner(&caller.user_id, &caller.organization_id)
            .await
    };
    let rows = match rows {
        Ok(r) => r,
        Err(e) => return error_response(&e),
    };
    let mut out = Vec::with_capacity(rows.len());
    for e in rows {
        let title = match title_or_err(state.source(), &e.course_id).await {
            Ok(t) => t,
            Err(err) => return error_response(&err),
        };
        out.push(view(e, title));
    }
    Json(out).into_response()
}

/// US3: POST /enrollments/{id}/completion — the enrolled→completed
/// transition (first-wins; gates live in the repository).
async fn complete(
    Extension(caller): Extension<EnrollmentCaller>,
    AxumState(state): AxumState<RepoState>,
    Path(id): Path<String>,
) -> Response {
    match state
        .repo()
        .complete(&caller.user_id, &id, &caller.organization_id)
        .await
    {
        Ok(e) => Json(e).into_response(),
        Err(e) => error_response(&e),
    }
}

fn view(e: Enrollment, title: Option<String>) -> EnrollmentView {
    EnrollmentView {
        id: e.id,
        learner_id: e.learner_id,
        course_id: e.course_id,
        title,
        status: e.status,
        created_at: e.created_at,
        completed_at: e.completed_at,
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
        .route("/enrollments", get(list).post(enroll))
        .route("/enrollments/{id}/completion", post(complete))
}
