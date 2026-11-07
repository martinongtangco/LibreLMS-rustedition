//! Host wiring (ADR 0001): config → startup → migrations → axum router →
//! graceful shutdown. Testable via `run_with_stop` (Principle XIII gates run
//! against the deterministic doubles — Principle XVII).

use std::sync::Arc;

use askama::Template;
use axum::extract::{FromRef, FromRequestParts, State};
use axum::http::request::Parts;
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::{Json, Router};
use shared::auth::{self, AuthContext, AuthError};
use shared::session::SessionStore;
use shared::storage::{SchemaStatus, Storage};
use shared::Config;
use tokio::sync::Notify;
use tracing::{info, warn};

/// A 404 page for unknown routes (spec edge case: plain, non-leaky body).
const NOT_FOUND_BODY: &str = "404 Not Found\n";

/// Startup/execution failures. `Startup` messages name the offending item
/// (fail-fast, spec FR-001 / SC-002) — never a stack trace.
#[derive(Debug)]
pub enum HostError {
    Startup(String),
}
impl std::fmt::Display for HostError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            HostError::Startup(m) => write!(f, "startup failed: {m}"),
        }
    }
}
impl std::error::Error for HostError {}

/// Everything the handlers need.
#[derive(Clone)]
pub struct AppState {
    pub storage: Arc<dyn Storage>,
    pub store: Arc<dyn SessionStore>,
    pub cfg: Config,
    /// Catalog slice state (spec 003; host mount pattern).
    pub catalog_state: catalog::routes::RepoState,
    /// Enrollment slice state (spec 004; host mount pattern).
    pub enrollment_state: enrollment::routes::RepoState,
}

/// The home page (askama — R3).
#[derive(Template)]
#[template(path = "index.html")]
struct HomePage {
    title: String,
}

impl axum::extract::FromRef<AppState> for catalog::routes::RepoState {
    fn from_ref(state: &AppState) -> Self {
        // RepoState is an Arc holder — cloning is cheap (0.8: owned return).
        state.catalog_state.clone()
    }
}

impl axum::response::IntoResponse for HomePage {
    fn into_response(self) -> axum::response::Response {
        let mut html = String::new();
        self.render_into(&mut html)
            .expect("home page renders into a String (template has no fallible I/O)");
        (
            axum::http::StatusCode::OK,
            [(axum::http::header::CONTENT_TYPE, "text/html; charset=utf-8")],
            html,
        )
            .into_response()
    }
}

impl axum::extract::FromRef<AppState> for enrollment::routes::RepoState {
    fn from_ref(state: &AppState) -> Self {
        // RepoState is an Arc holder — cloning is cheap (0.8: owned return).
        state.enrollment_state.clone()
    }
}

fn app(state: AppState) -> Router {
    // The caller middleware wraps ONLY the catalog routes — /healthz and / must
    // stay open (an auth-requiring layer there breaks the health gate).
    let catalog_routes = catalog::routes::routes()
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            |Authed(ctx): Authed, mut req: axum::extract::Request, next: axum::middleware::Next| {
                async move {
                    req.extensions_mut().insert(catalog::routes::CatalogCaller {
                        organization_id: ctx.organization_id,
                        role: ctx.role,
                    });
                    next.run(req).await
                }
            },
        ));
    // The caller middleware wraps ONLY the enrollment routes — /healthz and /
    // must stay open (an auth-requiring layer there breaks the health gate).
    let enrollment_routes = enrollment::routes::routes().layer(
        axum::middleware::from_fn_with_state(
            state.clone(),
            |Authed(ctx): Authed, mut req: axum::extract::Request, next: axum::middleware::Next| {
                async move {
                    req.extensions_mut().insert(enrollment::routes::EnrollmentCaller {
                        user_id: ctx.user.id.clone(),
                        organization_id: ctx.organization_id,
                        role: ctx.role,
                    });
                    next.run(req).await
                }
            },
        ),
    );
    Router::new()
        .route("/healthz", get(healthz))
        .route("/me", get(me))
        .merge(catalog_routes)
        .merge(enrollment_routes)
        .route("/", get(home))
        .with_state(state)
        .fallback(not_found)
}

/// The authenticated, scope-checked actor (US3). Rejected = an error
/// response already (401/403/503 envelope).
#[derive(Debug, Clone)]
pub struct Authed(pub AuthContext);

impl FromRequestParts<AppState> for Authed {
    type Rejection = Response;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        let org = parts
            .headers
            .get("X-Organization")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");
        let token = parts
            .headers
            .get(axum::http::header::COOKIE)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| {
                v.split(';').find_map(|kv| {
                    let (k, val) = kv.trim().split_once('=')?;
                    (k == "session").then_some(val)
                })
            });
        let Some(token) = token else {
            return Err(envelope(
                axum::http::StatusCode::UNAUTHORIZED,
                "unauthenticated",
                "missing or invalid session cookie",
            ));
        };
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);
        match auth::verify(
            &*state.storage,
            token,
            org,
            now,
            state.cfg.session_ttl_secs as i64,
            state.cfg.session_max_ttl_secs as i64,
        )
        .await
        {
            Ok(ctx) => Ok(Authed(ctx)),
            Err(AuthError::Unauthenticated) => Err(envelope(
                axum::http::StatusCode::UNAUTHORIZED,
                "unauthenticated",
                "unknown, expired, or out-of-scope session",
            )),
            Err(AuthError::Scope) => Err(envelope(
                axum::http::StatusCode::FORBIDDEN,
                "scope",
                "organization scope mismatch",
            )),
            Err(AuthError::Unavailable) => Err(envelope(
                axum::http::StatusCode::SERVICE_UNAVAILABLE,
                "unavailable",
                "session backend temporarily unavailable",
            )),
        }
    }
}

/// /me — the authenticated identity (contracts/api.md).
async fn me(Authed(ctx): Authed) -> Json<contracts::Identity> {
    Json(contracts::Identity {
        user_id: ctx.user.id.clone(),
        organization_id: ctx.organization_id.clone(),
        role: ctx.role,
    })
}

/// Error envelope (contracts/api.md) — no backend internals, no secrets.
fn envelope(status: axum::http::StatusCode, code: &'static str, message: &'static str) -> Response {
    (
        status,
        Json(contracts::ErrorEnvelope {
            error: code.to_string(),
            message: message.to_string(),
        }),
    )
        .into_response()
}

/// Home page handler (R3: askama server-rendered).
async fn home(State(s): State<AppState>) -> axum::response::Response {
    let mut html = String::new();
    HomePage {
        title: s.cfg.log_level.clone(),
    }
    .render_into(&mut html)
    .expect("home page renders into a String (template has no fallible I/O)");
    (
        axum::http::StatusCode::OK,
        [(axum::http::header::CONTENT_TYPE, "text/html; charset=utf-8")],
        html,
    )
        .into_response()
}

/// Unknown routes: plain, non-leaky 404 (spec edge case).
async fn not_found(_req: axum::extract::Request) -> (axum::http::StatusCode, &'static str) {
    (axum::http::StatusCode::NOT_FOUND, NOT_FOUND_BODY)
}

/// Health check (contracts/api.md). No auth.
async fn healthz(State(s): State<AppState>) -> axum::response::Response {
    let db = match s.storage.verify().await {
        Ok(()) => match s.storage.schema_status().await {
            Ok(SchemaStatus::Ready) => "reachable".to_string(),
            Ok(SchemaStatus::NotReady) => "schema-not-ready".to_string(),
            Err(e) => {
                warn!(target: "health", "db verify failed: {e}");
                "unreachable".to_string()
            }
        },
        Err(e) => {
            warn!(target: "health", "db verify failed: {e}");
            "unreachable".to_string()
        }
    };
    let store = match s.store.verify().await {
        Ok(()) => "reachable".to_string(),
        Err(e) => {
            warn!(target: "health", "session store verify failed: {e}");
            "unreachable".to_string()
        }
    };
    let status = if db == "unreachable" || store == "unreachable" {
        "degraded"
    } else {
        "ok"
    };
    let body = serde_json::json!({
        "status": status,
        "process": "alive",
        "database": db,
        "session_store": store,
    });
    let code = if status == "ok" {
        axum::http::StatusCode::OK
    } else {
        axum::http::StatusCode::SERVICE_UNAVAILABLE
    };
    (code, axum::Json(body)).into_response()
}

/// Start the host: verify backends (fail-fast naming the item), run migrations,
/// bind (port-in-use fails fast naming the port), serve until `stop` fires or
/// the process signal arrives.
pub async fn run_with_stop(
    cfg: Config,
    storage: Arc<dyn Storage>,
    store: Arc<dyn SessionStore>,
    catalog_repo: Arc<dyn catalog::repo::CourseRepo>,
    enrollment_repo: Arc<dyn enrollment::repo::EnrollmentRepo>,
    course_source: Arc<dyn enrollment::repo::CourseSource>,
    stop: Option<Arc<Notify>>,
) -> Result<(), HostError> {
    // Fail-fast backend verification (spec US1: startup proof).
    if let Err(e) = storage.verify().await {
        return Err(HostError::Startup(format!("DATABASE_URL backend: {e}")));
    }
    if let Err(e) = store.verify().await {
        return Err(HostError::Startup(format!("VALKEY_URL backend: {e}")));
    }
    // CR-2 seam: base manifest + every module's MIGRATIONS (name-ordered).
    // Module manifests land with their slices; the runner is already generic.
    let mut manifest: Vec<(&str, &str)> = shared::migrate::MIGRATIONS
        .iter()
        .map(|(n, sql)| (*n, *sql))
        .collect();
    // Spec 003 lane: catalog's own migrations (Module Persistence Seam).
    manifest.extend(
        catalog::migrations::MIGRATIONS
            .iter()
            .map(|(n, sql)| (*n, *sql)),
    );
    // Spec 004 lane: enrollment's own migrations (Module Persistence Seam).
    manifest.extend(
        enrollment::migrations::MIGRATIONS
            .iter()
            .map(|(n, sql)| (*n, *sql)),
    );
    manifest.sort_by(|a, b| a.0.cmp(b.0));
    if let Err(e) = storage.run_migrations(&manifest).await {
        return Err(HostError::Startup(format!("migrations: {e}")));
    }

    let addr = std::net::SocketAddr::from(([127, 0, 0, 1], cfg.port));
    let listener = match tokio::net::TcpListener::bind(addr).await {
        Ok(l) => l,
        Err(e) => {
            return Err(HostError::Startup(format!(
                "PORT {port} in use or invalid: {e}",
                port = cfg.port
            )));
        }
    };

    let state = AppState {
        storage: storage.clone(),
        store: store.clone(),
        cfg: cfg.clone(),
        catalog_state: catalog::routes::RepoState::new(catalog_repo.clone()),
        enrollment_state: enrollment::routes::RepoState::new(enrollment_repo, course_source),
    };
    let router = app(state);

    info!("LibreLMS-rustedition host listening on {addr}");

    let shutdown = async move {
        if let Some(stop) = stop {
            tokio::select! {
                _ = stop.notified() => {}
                _ = tokio::signal::ctrl_c() => {}
            }
        } else {
            let _ = tokio::signal::ctrl_c().await;
        }
    };

    match axum::serve(listener, router)
        .with_graceful_shutdown(shutdown)
        .await
    {
        Ok(()) => {
            info!("graceful shutdown complete");
            Ok(())
        }
        Err(e) => Err(HostError::Startup(format!("server: {e}"))),
    }
}

/// Production entry: process signal only.
pub async fn run(
    cfg: Config,
    storage: Arc<dyn Storage>,
    store: Arc<dyn SessionStore>,
    catalog_repo: Arc<dyn catalog::repo::CourseRepo>,
    enrollment_repo: Arc<dyn enrollment::repo::EnrollmentRepo>,
    course_source: Arc<dyn enrollment::repo::CourseSource>,
) -> Result<(), HostError> {
    run_with_stop(cfg, storage, store, catalog_repo, enrollment_repo, course_source, None).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn envelope_maps_to_correct_status_and_shape() {
        let r = envelope(
            axum::http::StatusCode::SERVICE_UNAVAILABLE,
            "unavailable",
            "session backend temporarily unavailable",
        );
        assert_eq!(r.status(), axum::http::StatusCode::SERVICE_UNAVAILABLE);
        let ct = r
            .headers()
            .get(axum::http::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .unwrap();
        assert!(ct.contains("application/json"));
    }
}
