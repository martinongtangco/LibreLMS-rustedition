//! Catalog HTTP surface (spec 003). The routes are state-bound to the repo
//! (host mount pattern: `host` does `router.merge(catalog::routes(repo))`);
//! the caller (org + role) arrives as an `Extension<CatalogCaller>` inserted by
//! the host's auth middleware — the catalog never depends on host types.

use std::sync::Arc;

use axum::extract::{FromRef, Path, Query, State as AxumState};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::{Extension, Json, Router};
use serde::Deserialize;

use contracts::{ErrorEnvelope, Role};

use crate::repo::{CatalogError, CourseRepo};

/// What the host's auth middleware resolves the session to (catalog-owned shape
/// so the module stays host-independent — Principle III).
#[derive(Debug, Clone)]
pub struct CatalogCaller {
    pub organization_id: String,
    pub role: Role,
}

/// State holder (satisfies axum 0.8's `State(S: Clone + Send + Sync)` bound).
#[derive(Clone)]
pub struct RepoState(Arc<dyn CourseRepo>);

impl RepoState {
    pub fn new(repo: Arc<dyn CourseRepo>) -> Self {
        Self(repo)
    }

    fn repo(&self) -> &Arc<dyn CourseRepo> {
        &self.0
    }
}

#[derive(Deserialize)]
struct ListQuery {
    title: Option<String>,
}

#[derive(Deserialize)]
struct CourseBody {
    title: Option<String>,
    description: Option<String>,
}

/// Error envelope mapping (contracts/api.md — never leaks backend internals).
pub fn error_response(e: &CatalogError) -> Response {
    let (status, code, message) = match e {
        CatalogError::NotFound => (
            axum::http::StatusCode::NOT_FOUND,
            "not_found".to_string(),
            "course not found".to_string(),
        ),
        CatalogError::Scope => (
            axum::http::StatusCode::FORBIDDEN,
            "scope".to_string(),
            "organization scope mismatch".to_string(),
        ),
        CatalogError::Forbidden => (
            axum::http::StatusCode::FORBIDDEN,
            "forbidden".to_string(),
            "role does not allow this action".to_string(),
        ),
        CatalogError::Validation { field, message } => (
            axum::http::StatusCode::BAD_REQUEST,
            "validation".to_string(),
            format!("{field}: {message}"),
        ),
        CatalogError::Unavailable => (
            axum::http::StatusCode::SERVICE_UNAVAILABLE,
            "unavailable".to_string(),
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

fn require_admin(role: &Role) -> Result<(), CatalogError> {
    if *role == Role::Admin {
        Ok(())
    } else {
        Err(CatalogError::Forbidden)
    }
}

async fn list(
    Extension(caller): Extension<CatalogCaller>,
    AxumState(state): AxumState<RepoState>,
    Query(q): Query<ListQuery>,
) -> Response {
    match state
        .repo()
        .list_for(&caller.organization_id, q.title.as_deref())
        .await
    {
        Ok(courses) => Json(courses).into_response(),
        Err(e) => error_response(&e),
    }
}

async fn get_one(
    Extension(caller): Extension<CatalogCaller>,
    AxumState(state): AxumState<RepoState>,
    Path(id): Path<String>,
) -> Response {
    match state.repo().get(&id, &caller.organization_id).await {
        Ok(course) => Json(course).into_response(),
        Err(e) => error_response(&e),
    }
}

async fn create(
    Extension(caller): Extension<CatalogCaller>,
    AxumState(state): AxumState<RepoState>,
    Json(body): Json<CourseBody>,
) -> Response {
    if let Err(e) = require_admin(&caller.role) {
        return error_response(&e);
    }
    let Some(title) = body.title.filter(|t| !t.trim().is_empty()) else {
        return error_response(&CatalogError::Validation {
            field: "title",
            message: "title is required".to_string(),
        });
    };
    match state
        .repo()
        .create(&caller.organization_id, &title, body.description.as_deref())
        .await
    {
        Ok(course) => (axum::http::StatusCode::CREATED, Json(course)).into_response(),
        Err(e) => error_response(&e),
    }
}

async fn update(
    Extension(caller): Extension<CatalogCaller>,
    AxumState(state): AxumState<RepoState>,
    Path(id): Path<String>,
    Json(body): Json<CourseBody>,
) -> Response {
    if let Err(e) = require_admin(&caller.role) {
        return error_response(&e);
    }
    match state
        .repo()
        .update(
            &id,
            &caller.organization_id,
            body.title.as_deref(),
            body.description.as_deref(),
        )
        .await
    {
        Ok(course) => Json(course).into_response(),
        Err(e) => error_response(&e),
    }
}

async fn delete_one(
    Extension(caller): Extension<CatalogCaller>,
    AxumState(state): AxumState<RepoState>,
    Path(id): Path<String>,
) -> Response {
    if let Err(e) = require_admin(&caller.role) {
        return error_response(&e);
    }
    match state.repo().delete(&id, &caller.organization_id).await {
        Ok(()) => axum::http::StatusCode::NO_CONTENT.into_response(),
        Err(e) => error_response(&e),
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
        .route("/courses", get(list).post(create))
        .route("/courses/{id}", get(get_one).put(update).delete(delete_one))
}
