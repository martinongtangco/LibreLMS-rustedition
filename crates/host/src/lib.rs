//! Host wiring (ADR 0001): config → startup → migrations → axum router →
//! graceful shutdown. Testable via `run_with_stop` (Principle XIII gates run
//! against the deterministic doubles — Principle XVII).

use std::sync::Arc;

use askama::Template;
use axum::extract::State;
use axum::response::IntoResponse;
use axum::routing::get;
use axum::Router;
use shared::session::SessionStore;
use shared::storage::{SchemaStatus, Storage};
use shared::Config;
use tokio::sync::Notify;
use tracing::{info, warn};

/// A 404 page for unknown routes (spec edge case: plain, non-leaky body).
const NOT_FOUND_BODY: &str = "404 Not Found\n";

/// Unified error envelope (contracts/api.md).
#[derive(Debug, serde::Serialize)]
struct ErrorBody {
    error: &'static str,
    message: String,
}

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
}

/// The home page (askama — R3).
#[derive(Template)]
#[template(path = "index.html")]
struct HomePage {
    title: String,
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

fn app(state: AppState) -> Router {
    Router::new()
        .route("/healthz", get(healthz))
        .route("/", get(home))
        .with_state(state)
        .fallback(not_found)
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
async fn not_found(
    _req: axum::extract::Request,
) -> (axum::http::StatusCode, &'static str) {
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
    stop: Option<Arc<Notify>>,
) -> Result<(), HostError> {
    // Fail-fast backend verification (spec US1: startup proof).
    if let Err(e) = storage.verify().await {
        return Err(HostError::Startup(format!("DATABASE_URL backend: {e}")));
    }
    if let Err(e) = store.verify().await {
        return Err(HostError::Startup(format!("VALKEY_URL backend: {e}")));
    }
    if let Err(e) = storage.run_migrations().await {
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
        storage,
        store,
        cfg,
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

    match axum::serve(listener, router).with_graceful_shutdown(shutdown).await {
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
) -> Result<(), HostError> {
    run_with_stop(cfg, storage, store, None).await
}
