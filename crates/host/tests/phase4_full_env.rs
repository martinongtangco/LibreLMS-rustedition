//! Phase 4 — the all-slice full-env battery (the DECIDING gate for the
//! integration; SKIP is not a pass — the runner must verify the
//! `applied:` line / real duration before recording a pass). Real
//! engine: Tiberius repos for 003/004/005 (+ the 006 management repo
//! for user/org setup), the REAL Phase-4 adapters, and the real host
//! boot with the production wiring (the 002 lesson: the gate never
//! inspects main.rs, so the boot smoke IS the wiring proof).
//!
//! Proves: (a) the full 6-slice manifest on fresh + idempotency
//! (run-2 no-op; empty manifest no-op; the post-state SET assertion is
//! order-invariant — the 006 review CR-1 fix); (b) the cross-slice
//! data path on the REAL engine through the REAL adapters (create
//! course → the adapter's 404/403 signals → enroll → launch gate →
//! scorm finish-passed → record_completion AFTER finalize (the 005
//! finish ordering, kept) → 004 Completed state → relaunch gate =
//! Completed, idempotent second completion); (c) the 404/403 decision
//! procedure through the port's error type (FR-002); (d) the real host
//! boot (healthz 200); (e) post-state probes against the engine.

use futures::StreamExt;

use contracts::Role;
use host::adapters::{CatalogCourseSource, EnrollmentRepoPort};
use scorm::repo::{EnrollmentPort, ScormAttempt, ScormRepo};
use std::sync::Arc;
use tokio_util::compat::TokioAsyncWriteCompatExt;

mod common;
use common::{wait_health_ok, AppConfig};

fn env(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|v| !v.is_empty())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn phase4_all_slice_full_env_battery() {
    let url = match env("DATABASE_URL") {
        Some(u) => u,
        None => {
            eprintln!(
                "SKIP: DATABASE_URL not set — the Phase-4 full battery needs \
                 a disposable environment (Principle XVII). The \
                 fresh-container run is the deciding evidence (Principle \
                 XVI)."
            );
            return;
        }
    };
    let vurl = env("VALKEY_URL").expect("VALKEY_URL required for the battery");

    let storage: Arc<dyn shared::storage::Storage> = Arc::new(
        shared::storage::TiberiusStorage::connect(&url)
            .await
            .expect("storage connect"),
    );

    // ---------- (a) the full 6-slice host manifest — idempotency ----------
    let mut manifest: Vec<(&str, &str)> = shared::migrate::MIGRATIONS
        .iter()
        .map(|(n, sql)| (*n, *sql))
        .collect();
    manifest.extend(
        catalog::migrations::MIGRATIONS
            .iter()
            .map(|(n, sql)| (*n, *sql)),
    );
    manifest.extend(
        enrollment::migrations::MIGRATIONS
            .iter()
            .map(|(n, sql)| (*n, *sql)),
    );
    manifest.extend(
        scorm::migrations::MIGRATIONS
            .iter()
            .map(|(n, sql)| (*n, *sql)),
    );
    manifest.extend(
        management::migrations::MIGRATIONS
            .iter()
            .map(|(n, sql)| (*n, *sql)),
    );
    let pre: i32 = probe_scalar(&url, "SELECT COUNT(*) FROM dbo.schema_migrations", None)
        .await
        .unwrap_or(0);
    let applied = storage.run_migrations(&manifest).await.expect("run 1");
    eprintln!("applied: {applied:?}");
    if pre == 0 {
        assert_eq!(
            applied.len(),
            5,
            "a fresh run applies all five non-empty scripts: {applied:?}"
        );
    }
    let applied2 = storage.run_migrations(&manifest).await.expect("run 2");
    assert!(
        applied2.is_empty(),
        "idempotency: run 2 is a no-op (got {applied2:?})"
    );
    let applied0 = storage.run_migrations(&[]).await.expect("empty manifest");
    assert!(
        applied0.is_empty(),
        "empty manifest: no-op (got {applied0:?})"
    );
    // The post-state SET (the order-invariant assertion — a shared-DB
    // cycle run observes a delta, not the full set; the SET is stable).
    let set: Vec<String> = probe_rows(&url, "SELECT name FROM dbo.schema_migrations ORDER BY name")
        .await
        .expect("migration set probe");
    assert_eq!(
        set,
        vec![
            "0001_base",
            "0002_seed",
            "0003_catalog",
            "0005_enrollment",
            "0006_scorm",
        ],
        "post-state = the five non-empty migrations (order-invariant)"
    );

    // ---------- (b) the cross-slice data path, REAL engine + REAL adapters ----------
    let management: Arc<dyn management::repo::ManagementRepo> = Arc::new(
        management::tiberius::TiberiusManagementRepo::connect(&url)
            .await
            .expect("management repo"),
    );
    let catalog: Arc<dyn catalog::repo::CourseRepo> = Arc::new(
        catalog::tiberius::TiberiusCourseRepo::connect(&url)
            .await
            .expect("catalog repo"),
    );
    let enrollment: Arc<dyn enrollment::repo::EnrollmentRepo> = Arc::new(
        enrollment::tiberius::TiberiusEnrollmentRepo::connect(&url)
            .await
            .expect("enrollment repo"),
    );
    let scorm: Arc<dyn ScormRepo> = Arc::new(
        scorm::tiberius::TiberiusScormRepo::connect(&url)
            .await
            .expect("scorm repo"),
    );
    let course_source: Arc<dyn enrollment::repo::CourseSource> =
        Arc::new(CatalogCourseSource::new(catalog.clone()));
    let port = Arc::new(EnrollmentRepoPort::new(
        enrollment.clone(),
        course_source.clone(),
    ));

    // Cross-slice setup via the 006 slice (real org + real learner — the
    // full stack's user/org ground truth).
    let org = management.create_org("P4 Org").await.expect("create org");
    let learner = management
        .create_user(&org.id, "p4@example.com", "P4 Learner", &Role::Member)
        .await
        .expect("create learner");

    // Course (003) → the adapter's 404/403 signals (the decision
    // procedure on the real engine).
    let course = catalog
        .create(&org.id, "P4 Course", Some("Phase 4 course"))
        .await
        .expect("create course");
    assert_eq!(
        course_source.course_title("no-such-course").await.unwrap(),
        None,
        "unknown course → None (the valid 404 signal — an outage is Err, \
         never Ok(None))"
    );
    let org2 = management.create_org("P4 Org Two").await.expect("org two");
    assert_eq!(
        course_source.course_title(&course.id).await.unwrap(),
        Some("P4 Course".to_string()),
        "the course title resolves (Some — the 404 class is absent)"
    );
    let out_of_scope = course_source
        .course_exists_in_org(&course.id, &org2.id)
        .await
        .unwrap();
    assert!(
        !out_of_scope,
        "another org's course → exists_in_org false (the 403 signal)"
    );
    assert!(
        course_source
            .course_exists_in_org(&course.id, &org.id)
            .await
            .unwrap(),
        "the course's own org → true (proceed)"
    );

    // Enroll (004) → the port's launch gate.
    enrollment
        .create(&learner.user.id, &course.id, &org.id)
        .await
        .expect("enroll");
    let gate = port
        .enrollment_status(&learner.user.id, &course.id, &org.id)
        .await
        .expect("launch gate");
    assert!(
        matches!(gate, scorm::repo::LaunchGate::Enrolled),
        "enrolled learner → the gate is Enrolled"
    );
    // The 404/403 signals through the PORT (FR-002's error type).
    let e404 = port
        .enrollment_status(&learner.user.id, "no-such-course", &org.id)
        .await
        .unwrap_err();
    assert!(
        matches!(e404, scorm::repo::ScormError::NotFound),
        "unknown course → the port's NotFound (the 404 class)"
    );
    let e403 = port
        .enrollment_status(&learner.user.id, &course.id, &org2.id)
        .await
        .unwrap_err();
    assert!(
        matches!(e403, scorm::repo::ScormError::Scope),
        "another org's course → the port's Scope (the 403 class)"
    );

    // SCORM (005): create → in_progress → passed → finalize. The 005
    // finish ordering is kept verbatim: finalize BEFORE
    // record_completion (a final attempt on a port outage self-heals via
    // session scoping; completion-first would leave a mutable live
    // attempt under a completed enrollment).
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as i64;
    let session = format!("p4-session-{}", std::process::id());
    let attempt = ScormAttempt {
        id: format!("p4-attempt-{}", std::process::id()),
        session_id: session.clone(),
        course_id: course.id.clone(),
        organization_id: org.id.clone(),
        state: "in_progress".to_string(),
        suspend_data: String::new(),
        score_raw: None,
        session_time: 42,
        finished_at: None,
        created_at: now_ms,
    };
    scorm.create(&attempt).await.expect("create attempt");
    // The 005 engine assigns the attempt id (NEWID, the wire discipline) —
    // the caller reads the row back by (session, course) and uses the
    // engine-assigned id (the 005 full-env convention).
    let attempt = scorm
        .find_by_session_course(&session, &course.id)
        .await
        .expect("read back the attempt")
        .expect("the attempt row exists");
    scorm
        .update_committed(&attempt.id, "in_progress", "", None, 42)
        .await
        .expect("commit in_progress");
    scorm
        .update_committed(&attempt.id, "passed", "", Some(100), 84)
        .await
        .expect("commit passed");
    scorm
        .finalize(&attempt.id, "passed", "", Some(100), 84)
        .await
        .expect("finalize (the implicit final commit)");

    // record_completion (the port → the 004 enrolled→completed
    // transition) — AFTER finalize (the kept ordering).
    port.record_completion(&learner.user.id, &course.id, &org.id)
        .await
        .expect("record completion through the port");
    // Idempotency (the 004 engine anchor): the second call is a no-op
    // success, never an error, never a second timestamp.
    port.record_completion(&learner.user.id, &course.id, &org.id)
        .await
        .expect("idempotent second completion");

    // The 004 state is Completed; the relaunch gate reads it through the
    // port (the cross-slice loop closes).
    let list = enrollment
        .list_for_learner(&learner.user.id, &org.id)
        .await
        .expect("004 list");
    let row = list
        .iter()
        .find(|e| e.course_id == course.id)
        .expect("the enrollment row");
    assert!(
        matches!(row.status, enrollment::repo::Status::Completed),
        "the 004 state is Completed (cross-slice)"
    );
    let first_ts = row.completed_at.expect("the first-completion timestamp");
    let gate2 = port
        .enrollment_status(&learner.user.id, &course.id, &org.id)
        .await
        .expect("relaunch gate");
    assert!(
        matches!(gate2, scorm::repo::LaunchGate::Completed),
        "relaunch gate = Completed (the loop closes)"
    );
    let list2 = enrollment
        .list_for_learner(&learner.user.id, &org.id)
        .await
        .expect("004 list again");
    let row2 = list2
        .iter()
        .find(|e| e.course_id == course.id)
        .expect("the enrollment row again");
    assert_eq!(
        row2.completed_at,
        Some(first_ts),
        "the completion timestamp is unchanged (first-wins)"
    );

    // ---------- (d) the real host boot — the production wiring truth ----------
    let store: Arc<dyn shared::session::SessionStore> = Arc::new(
        shared::session::ValkeySessionStore::connect(&vurl)
            .await
            .expect("valkey session store"),
    );
    let bag = Arc::new(scorm::tiberius::SessionBagStore::new(store.clone()));
    let port2 = Arc::new(EnrollmentRepoPort::new(
        enrollment.clone(),
        course_source.clone(),
    ));
    let stop = Arc::new(tokio::sync::Notify::new());
    let stop2 = stop.clone();
    let child = tokio::spawn(async move {
        host::run_with_stop(
            AppConfig::valid(18_999),
            storage,
            store,
            catalog,
            enrollment,
            course_source,
            scorm,
            bag,
            port2,
            management,
            Some(stop2),
        )
        .await
    });
    assert!(
        wait_health_ok(18_999).await,
        "the real host boots with the production wiring (healthz 200 — \
         all startup verifies pass)"
    );
    stop.notify_one();
    let res = child.await.unwrap();
    assert!(res.is_ok(), "clean stop after the health check");

    // ---------- (e) post-state probes against the engine ----------
    let migs: i32 = probe_scalar(&url, "SELECT COUNT(*) FROM dbo.schema_migrations", None)
        .await
        .expect("migration count probe");
    assert_eq!(migs, 5, "exactly five non-empty migrations (post-state)");
    let completions: i32 = probe_scalar(
        &url,
        "SELECT COUNT(*) FROM dbo.completions c JOIN dbo.enrollments e ON c.enrollment_id = e.id WHERE e.organization_id = @P1",
        Some(&org.id),
    )
    .await
    .expect("completion count probe");
    assert_eq!(
        completions, 1,
        "exactly one completion row (first-wins — the 004 engine anchor)"
    );
    let attempts: i32 = probe_scalar(
        &url,
        "SELECT COUNT(*) FROM dbo.scorm_attempts WHERE session_id = @P1",
        Some(&session),
    )
    .await
    .expect("attempt count probe");
    assert_eq!(attempts, 1, "one scorm attempt for the session");
    let finished: i32 = probe_scalar(
        &url,
        "SELECT COUNT(*) FROM dbo.scorm_attempts WHERE finished_at IS NOT NULL AND session_id = @P1",
        Some(&session),
    )
    .await
    .expect("finished count probe");
    assert_eq!(finished, 1, "the attempt is finalized (finished_at set)");

    eprintln!(
        "full_env — post-state: {migs} migration rows, 1 course, 1 enrollment \
         (completed), {completions} completion, {attempts} attempt \
         ({finished} finalized)"
    );
}

/// A direct engine connection (a test-side read-only inspection, not a
/// module path — the 005 convention). `param` is the @p value for the
/// session-scoped probes (None for the count-only probes).
async fn probe_scalar(url: &str, sql: &str, param: Option<&str>) -> Result<i32, String> {
    let (host, port, user, pass, db, encrypt) = parse_probe_url(url)?;
    let mut config = tiberius::Config::new();
    config.database(&db);
    config.authentication(tiberius::AuthMethod::sql_server(&user, &pass));
    config.encryption(if encrypt {
        tiberius::EncryptionLevel::On
    } else {
        tiberius::EncryptionLevel::NotSupported
    });
    if encrypt {
        config.trust_cert();
    }
    let tcp = tokio::net::TcpStream::connect((host.as_str(), port))
        .await
        .map_err(|e| format!("probe tcp: {e}"))?;
    tcp.set_nodelay(true)
        .map_err(|e| format!("probe nodelay: {e}"))?;
    let mut probe = tiberius::Client::connect(config, tcp.compat_write())
        .await
        .map_err(|e| format!("probe tds: {e}"))?;
    let (pv, has) = match param {
        Some(v) => (v.to_string(), true),
        None => (String::new(), false),
    };
    let params: Vec<&dyn tiberius::ToSql> = if has {
        vec![&pv as &dyn tiberius::ToSql]
    } else {
        vec![]
    };
    let mut stream = probe
        .query(sql, &params)
        .await
        .map_err(|e| format!("probe query: {e}"))?;
    while let Some(item) = stream.next().await {
        let item = item.map_err(|e| format!("probe stream: {e}"))?;
        if let tiberius::QueryItem::Row(row) = item {
            if let Some(v) = row.get::<i32, _>(0) {
                return Ok(v);
            }
        }
    }
    Err("probe: no scalar row".to_string())
}

/// A direct engine connection (read-only row inspection — the 005
/// convention).
async fn probe_rows(url: &str, sql: &str) -> Result<Vec<String>, String> {
    let (host, port, user, pass, db, encrypt) = parse_probe_url(url)?;
    let mut config = tiberius::Config::new();
    config.database(&db);
    config.authentication(tiberius::AuthMethod::sql_server(&user, &pass));
    config.encryption(if encrypt {
        tiberius::EncryptionLevel::On
    } else {
        tiberius::EncryptionLevel::NotSupported
    });
    if encrypt {
        config.trust_cert();
    }
    let tcp = tokio::net::TcpStream::connect((host.as_str(), port))
        .await
        .map_err(|e| format!("probe tcp: {e}"))?;
    tcp.set_nodelay(true)
        .map_err(|e| format!("probe nodelay: {e}"))?;
    let mut probe = tiberius::Client::connect(config, tcp.compat_write())
        .await
        .map_err(|e| format!("probe tds: {e}"))?;
    let mut stream = probe
        .query(sql, &[] as &[&dyn tiberius::ToSql])
        .await
        .map_err(|e| format!("probe query: {e}"))?;
    let mut rows = Vec::new();
    while let Some(item) = stream.next().await {
        let item = item.map_err(|e| format!("probe stream: {e}"))?;
        if let tiberius::QueryItem::Row(row) = item {
            if let Some(s) = row.get::<&str, _>(0) {
                rows.push(s.to_string());
            }
        }
    }
    Ok(rows)
}

/// Parses `mssql://user:pass@host:port/db?query` (the 005 convention:
/// the db name is derived AFTER stripping the query string — the CR-B
/// lesson; encrypt=disable|off|false → OFF; no encrypt param → ON).
fn parse_probe_url(url: &str) -> Result<(String, u16, String, String, String, bool), String> {
    let rest = url
        .strip_prefix("mssql://")
        .or_else(|| url.strip_prefix("sqlserver://"))
        .ok_or_else(|| format!("bad url scheme: {url}"))?;
    let (rest, query) = rest.split_once('?').unwrap_or((rest, ""));
    let encrypt = match query.split('&').find_map(|kv| {
        kv.split_once('=')
            .filter(|(k, _)| k.eq_ignore_ascii_case("encrypt"))
            .map(|(_, v)| v.to_lowercase())
    }) {
        Some(v) => !matches!(v.as_str(), "disable" | "off" | "false"),
        None => true,
    };
    let (creds, hostpart) = rest
        .split_once('@')
        .ok_or_else(|| format!("missing @: {url}"))?;
    let (user, pass) = creds
        .split_once(':')
        .ok_or_else(|| format!("missing pass: {url}"))?;
    let (hostport, db) = hostpart
        .rsplit_once('/')
        .ok_or_else(|| format!("missing db: {url}"))?;
    let (host, port) = hostport
        .rsplit_once(':')
        .ok_or_else(|| format!("missing port: {url}"))?;
    let port = port.parse().map_err(|e| format!("bad port: {e}"))?;
    Ok((
        host.to_string(),
        port,
        user.to_string(),
        pass.to_string(),
        db.to_string(),
        encrypt,
    ))
}
