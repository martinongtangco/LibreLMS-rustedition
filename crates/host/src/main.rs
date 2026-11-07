//! Binary entry point: env config → real backends → host::run (FR-001).

use shared::config::Config;
use shared::session::ValkeySessionStore;
use shared::storage::TiberiusStorage;
use std::sync::Arc;
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() {
    // FR-009: the subscriber never receives config values; log lines never
    // include DATABASE_URL/VALKEY_URL content (the shared error types carry
    // only backend error text).
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();

    let cfg = match Config::from_env() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(1);
        }
    };

    let storage: Arc<dyn shared::storage::Storage> =
        match TiberiusStorage::connect(&cfg.database_url).await {
            Ok(s) => Arc::new(s),
            Err(e) => {
                eprintln!("startup failed: DATABASE_URL backend: {e}");
                std::process::exit(1);
            }
        };
    let store: Arc<dyn shared::session::SessionStore> =
        match ValkeySessionStore::connect(&cfg.valkey_url).await {
            Ok(s) => Arc::new(s),
            Err(e) => {
                eprintln!("startup failed: VALKEY_URL backend: {e}");
                std::process::exit(1);
            }
        };

    // T018 (CR-5): the PRODUCTION wiring is the real engine — fail-fast like
    // the storage/session connects above; an InMemoryCourseRepo here would
    // silently lose course data and the full-env gate does not see main.rs.
    let catalog: std::sync::Arc<dyn catalog::repo::CourseRepo> =
        match catalog::tiberius::TiberiusCourseRepo::connect(&cfg.database_url).await {
            Ok(r) => Arc::new(r),
            Err(e) => {
                eprintln!("startup failed: DATABASE_URL catalog repo: {e}");
                std::process::exit(1);
            }
        };

    // Enrollment slice (spec 004): the REAL engine (no in-memory staging —
    // the 002 lesson: the gate never inspects main.rs, so the real wiring
    // is the production truth).
    let enrollment_repo: Arc<dyn enrollment::repo::EnrollmentRepo> =
        match enrollment::tiberius::TiberiusEnrollmentRepo::connect(&cfg.database_url).await {
            Ok(r) => {
                if let Err(e) = r.verify().await {
                    eprintln!("startup failed: enrollment repo verify: {e}");
                    std::process::exit(1);
                }
                Arc::new(r)
            }
            Err(e) => {
                eprintln!("startup failed: enrollment repo: {e}");
                std::process::exit(1);
            }
        };

    // CourseSource port (spec 004, PQ-2): PHASE 4 — the catalog→port
    // adapter is wired (the 003 provider's find_by_id seam; the 404/403
    // decision procedure lives in the adapter; an outage is 503, never a
    // fake Ok(None) — N1 discipline).
    let course_source: Arc<dyn enrollment::repo::CourseSource> =
        Arc::new(host::adapters::CatalogCourseSource::new(catalog.clone()));

    // SCORM slice (spec 005): the REAL engine (no in-memory staging — the
    // 002 lesson: the gate never inspects main.rs, so the real wiring is
    // the production truth).
    let scorm_repo: Arc<dyn scorm::repo::ScormRepo> =
        match scorm::tiberius::TiberiusScormRepo::connect(&cfg.database_url).await {
            Ok(r) => {
                if let Err(e) = r.verify().await {
                    eprintln!("startup failed: scorm repo verify: {e}");
                    std::process::exit(1);
                }
                Arc::new(r)
            }
            Err(e) => {
                eprintln!("startup failed: scorm repo: {e}");
                std::process::exit(1);
            }
        };
    // BagStore (plan FR-003): production wraps the shared session store at
    // the 001-reserved key (the key stays in shared ground truth — the port
    // never names it).
    let scorm_bag: Arc<dyn scorm::repo::BagStore> =
        Arc::new(scorm::tiberius::SessionBagStore::new(store.clone()));
    // EnrollmentPort (spec 005, FR-008): PHASE 4 — the 004-repository
    // adapter is wired (+ the 003 catalog for the port's 404/403 course
    // check, FR-002). First-completion-wins is the 004 engine's
    // guarantee; a completion outage is never a fake success (N1).
    let scorm_enrollment_port: Arc<dyn scorm::repo::EnrollmentPort> = Arc::new(
        host::adapters::EnrollmentRepoPort::new(enrollment_repo.clone(), course_source.clone()),
    );

    let management_repo: Arc<dyn management::repo::ManagementRepo> = {
        let r =
            match management::tiberius::TiberiusManagementRepo::connect(cfg.database_url.as_str())
                .await
                .map_err(|e| format!("management: {e}"))
            {
                Ok(r) => r,
                Err(m) => {
                    eprintln!("management: {m}");
                    std::process::exit(1);
                }
            };
        Arc::new(r)
    };

    if let Err(e) = host::run(
        cfg,
        storage,
        store,
        catalog,
        enrollment_repo,
        course_source,
        scorm_repo,
        scorm_bag,
        scorm_enrollment_port,
        management_repo,
    )
    .await
    {
        eprintln!("{e}");
        std::process::exit(1);
    }
}
