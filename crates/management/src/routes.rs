//! The management HTTP surface (spec 006 US1/US2, T006). Envelope mapping
//! (the 001 api.md stable-code table, spec 006 FR-011): 401
//! `unauthenticated` (the host's `Authed` extractor) / 403 `forbidden`
//! (non-admin — the role-gate, BEFORE any id check) / 403 `scope`
//! (cross-org — content-identical to any other 403; no existence oracle) /
//! 404 `not-found` (org-ids-only, FR-010/CR-3) / 409 `duplicate-email` /
//! 409 `not-empty` / 400 `validation` (the field is named in the body —
//! FR-012 incl. role, CR-1) / 503 `storage-unavailable` (never a 500, never
//! a silent empty list). The caller (org + role) arrives as an
//! `Extension<ManagementCaller>` inserted by the host's middleware (the
//! 003/005 pattern).

use axum::extract::{FromRef, Path as AxumPath, State as AxumState};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{delete, post, put};
use axum::{Extension, Json, Router};
use serde::Deserialize;
use std::sync::Arc;

use crate::repo::{parse_role, ManagementError, ManagementRepo};

/// What the host's auth middleware resolves the session to (management-owned
/// shape so the module stays host-independent — Principle III).
#[derive(Debug, Clone)]
pub struct ManagementCaller {
    pub user_id: String,
    pub organization_id: String,
    pub role: contracts::Role,
}

/// State holder (satisfies axum 0.8's `State(S: Clone + Send + Sync)` bound).
#[derive(Clone)]
pub struct RepoState {
    repo: Arc<dyn ManagementRepo>,
}

impl RepoState {
    pub fn new(repo: Arc<dyn ManagementRepo>) -> Self {
        Self { repo }
    }

    pub fn repo(&self) -> &Arc<dyn ManagementRepo> {
        &self.repo
    }
}

fn require_admin(role: &contracts::Role) -> Result<(), ManagementError> {
    if *role == contracts::Role::Admin {
        Ok(())
    } else {
        Err(ManagementError::Forbidden)
    }
}

/// The X-Organization header (the target-org channel; absent → an empty
/// sentinel the scope gate maps to 403 `scope` — never a 200 with an
/// ambiguous target org, never a 400).
#[derive(Debug, Clone)]
pub struct XOrganization(String);

impl<S> axum::extract::FromRequestParts<S> for XOrganization
where
    S: Send + Sync,
{
    type Rejection = axum::response::Response;
    fn from_request_parts(
        parts: &mut axum::http::request::Parts,
        _state: &S,
    ) -> impl std::future::Future<Output = Result<Self, Self::Rejection>> + Send {
        // Absent / non-UTF-8 → the empty sentinel (the scope gate maps it
        // to 403 `scope` — never a 400, never an ambiguous 200).
        let v = parts
            .headers
            .get("x-organization")
            .and_then(|v| v.to_str().ok())
            .unwrap_or_default()
            .to_string();
        async move { Ok(Self(v)) }
    }
}

/// The org-scope gate (403 `scope`, content-identical to any other 403 —
/// the no-existence-oracle rule; the header must match the session org;
/// an absent header is a scope failure — the target org is the header).
fn require_scope(caller: &ManagementCaller, header_org: &str) -> Result<(), ManagementError> {
    if !header_org.is_empty() && header_org == caller.organization_id {
        Ok(())
    } else {
        Err(ManagementError::Scope)
    }
}

/// Error envelope mapping (the spec 006 stable-code table; never a 500,
/// never a silent empty list — FR-011).
pub fn error_response(e: &ManagementError) -> Response {
    let (status, code, message) = match e {
        ManagementError::Scope => (
            StatusCode::FORBIDDEN,
            "scope".to_string(),
            "organization scope mismatch".to_string(),
        ),
        ManagementError::Forbidden => (
            StatusCode::FORBIDDEN,
            "forbidden".to_string(),
            "role does not allow this action".to_string(),
        ),
        ManagementError::NotFound => (
            StatusCode::NOT_FOUND,
            "not-found".to_string(),
            "organization not found".to_string(),
        ),
        ManagementError::DuplicateEmail => (
            StatusCode::CONFLICT,
            "duplicate-email".to_string(),
            "a user with this email already exists".to_string(),
        ),
        ManagementError::NotEmpty => (
            StatusCode::CONFLICT,
            "not-empty".to_string(),
            "organization is not empty".to_string(),
        ),
        ManagementError::Validation { field, message } => (
            StatusCode::BAD_REQUEST,
            "validation".to_string(),
            format!("{field}: {message}"),
        ),
        ManagementError::Unavailable => (
            StatusCode::SERVICE_UNAVAILABLE,
            "storage-unavailable".to_string(),
            "storage unavailable".to_string(),
        ),
    };
    (
        status,
        Json(serde_json::json!({ "error": code, "message": message })),
    )
        .into_response()
}

// ---------- US1 (P1): learner management ----------

#[derive(Deserialize)]
struct CreateUserBody {
    email: Option<String>,
    name: Option<String>,
    role: Option<String>,
}

#[derive(Deserialize)]
struct RoleBody {
    role: Option<String>,
}

#[derive(Deserialize)]
struct OrgBody {
    name: Option<String>,
}

async fn create_user(
    Extension(caller): Extension<ManagementCaller>,
    AxumState(state): AxumState<RepoState>,
    x_org: XOrganization,
    Json(body): Json<CreateUserBody>,
) -> Response {
    // The role-gate BEFORE any id check (403 `forbidden`).
    if let Err(e) = require_admin(&caller.role) {
        return error_response(&e);
    }
    // The org-scope gate (403 `scope`).
    if let Err(e) = require_scope(&caller, &x_org.0) {
        return error_response(&e);
    }
    // FR-012/CR-1: the field-named 400s — email/name presence first, then
    // the role parse (the CHECK constraint is the belt).
    let email = match body.email {
        Some(e) => e,
        None => {
            return error_response(&ManagementError::Validation {
                field: "email",
                message: "email is required".to_string(),
            })
        }
    };
    let name = match body.name {
        Some(n) => n,
        None => {
            return error_response(&ManagementError::Validation {
                field: "name",
                message: "name is required".to_string(),
            })
        }
    };
    let role = match body.role {
        Some(r) => match parse_role(&r) {
            Ok(r) => r,
            Err(e) => return error_response(&e),
        },
        None => {
            return error_response(&ManagementError::Validation {
                field: "role",
                message: "role is required".to_string(),
            })
        }
    };
    match state
        .repo()
        .create_user(&caller.organization_id, &email, &name, &role)
        .await
    {
        Ok(mu) => (
            StatusCode::CREATED,
            Json(serde_json::json!({
                "id": mu.user.id,
                "email": mu.user.email,
                "name": mu.user.name,
                "role": mu.role,
            })),
        )
            .into_response(),
        Err(e) => error_response(&e),
    }
}

async fn list_users(
    Extension(caller): Extension<ManagementCaller>,
    AxumState(state): AxumState<RepoState>,
    x_org: XOrganization,
) -> Response {
    if let Err(e) = require_admin(&caller.role) {
        return error_response(&e);
    }
    if let Err(e) = require_scope(&caller, &x_org.0) {
        return error_response(&e);
    }
    match state.repo().list_users(&caller.organization_id).await {
        Ok(users) => {
            let list: Vec<serde_json::Value> = users
                .iter()
                .map(|mu| {
                    serde_json::json!({
                        "id": mu.user.id,
                        "email": mu.user.email,
                        "name": mu.user.name,
                        "role": mu.role,
                    })
                })
                .collect();
            (StatusCode::OK, Json(serde_json::json!(list))).into_response()
        }
        Err(e) => error_response(&e),
    }
}

async fn set_role(
    Extension(caller): Extension<ManagementCaller>,
    AxumState(state): AxumState<RepoState>,
    AxumPath(id): AxumPath<String>,
    x_org: XOrganization,
    Json(body): Json<RoleBody>,
) -> Response {
    if let Err(e) = require_admin(&caller.role) {
        return error_response(&e);
    }
    if let Err(e) = require_scope(&caller, &x_org.0) {
        return error_response(&e);
    }
    let role = match body.role {
        Some(r) => match parse_role(&r) {
            Ok(r) => r,
            Err(e) => return error_response(&e),
        },
        None => {
            return error_response(&ManagementError::Validation {
                field: "role",
                message: "role is required".to_string(),
            })
        }
    };
    match state
        .repo()
        .set_role(&caller.organization_id, &id, &role)
        .await
    {
        Ok(()) => (
            StatusCode::OK,
            Json(serde_json::json!({ "re-roled": true })),
        )
            .into_response(),
        Err(e) => error_response(&e),
    }
}

async fn remove_user(
    Extension(caller): Extension<ManagementCaller>,
    AxumState(state): AxumState<RepoState>,
    AxumPath(id): AxumPath<String>,
    x_org: XOrganization,
) -> Response {
    if let Err(e) = require_admin(&caller.role) {
        return error_response(&e);
    }
    if let Err(e) = require_scope(&caller, &x_org.0) {
        return error_response(&e);
    }
    match state
        .repo()
        .remove_membership(&caller.organization_id, &id)
        .await
    {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => error_response(&e),
    }
}

// ---------- US2 (P2): organization management (flat) ----------

async fn create_org(
    Extension(caller): Extension<ManagementCaller>,
    AxumState(state): AxumState<RepoState>,
    Json(body): Json<OrgBody>,
) -> Response {
    if let Err(e) = require_admin(&caller.role) {
        return error_response(&e);
    }
    let name = match body.name {
        Some(n) => n,
        None => {
            return error_response(&ManagementError::Validation {
                field: "name",
                message: "name is required".to_string(),
            })
        }
    };
    match state.repo().create_org(&name).await {
        Ok(org) => (
            StatusCode::CREATED,
            Json(serde_json::json!({ "id": org.id, "name": org.name })),
        )
            .into_response(),
        Err(e) => error_response(&e),
    }
}

async fn list_orgs(
    Extension(caller): Extension<ManagementCaller>,
    AxumState(state): AxumState<RepoState>,
) -> Response {
    if let Err(e) = require_admin(&caller.role) {
        return error_response(&e);
    }
    match state.repo().list_orgs().await {
        Ok(orgs) => {
            let list: Vec<serde_json::Value> = orgs
                .iter()
                .map(|(org, count)| {
                    serde_json::json!({ "id": org.id, "name": org.name, "member_count": count })
                })
                .collect();
            (StatusCode::OK, Json(serde_json::json!(list))).into_response()
        }
        Err(e) => error_response(&e),
    }
}

async fn rename_org(
    Extension(caller): Extension<ManagementCaller>,
    AxumState(state): AxumState<RepoState>,
    AxumPath(id): AxumPath<String>,
    Json(body): Json<OrgBody>,
) -> Response {
    if let Err(e) = require_admin(&caller.role) {
        return error_response(&e);
    }
    let name = match body.name {
        Some(n) => n,
        None => {
            return error_response(&ManagementError::Validation {
                field: "name",
                message: "name is required".to_string(),
            })
        }
    };
    match state.repo().rename_org(&id, &name).await {
        Ok(()) => (StatusCode::OK, Json(serde_json::json!({ "renamed": true }))).into_response(),
        Err(e) => error_response(&e),
    }
}

async fn delete_org(
    Extension(caller): Extension<ManagementCaller>,
    AxumState(state): AxumState<RepoState>,
    AxumPath(id): AxumPath<String>,
) -> Response {
    if let Err(e) = require_admin(&caller.role) {
        return error_response(&e);
    }
    match state.repo().delete_org(&id).await {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
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
        .route("/admin/users", post(create_user).get(list_users))
        .route("/admin/users/{id}/role", put(set_role))
        .route("/admin/users/{id}", delete(remove_user))
        .route("/admin/organizations", post(create_org).get(list_orgs))
        .route(
            "/admin/organizations/{id}",
            put(rename_org).delete(delete_org),
        )
}
