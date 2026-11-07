//! US1 (P1) — the SCORM real-engine battery (spec 005, gated full-env).
//! T014 red-first. GATED: runs only when DATABASE_URL/VALKEY_URL are set
//! (the fresh-container run is the deciding evidence, Principle XVI; a
//! SKIP is not a pass, Principle XVII).

use std::sync::Arc;

use futures::StreamExt;
use scorm::migrations::MIGRATIONS as SCORM_MIGRATIONS;
use scorm::repo::{ScormError, ScormRepo};
use scorm::tiberius::TiberiusScormRepo;
use shared::storage::{Storage as _, TiberiusStorage};
use tokio_util::compat::TokioAsyncWriteCompatExt;

const SESSION: &str = "full-env-scorm-session";
const ORG: &str = "full-env-scorm-org";

fn env(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|v| !v.is_empty())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn real_engine_full_battery() {
    let url = match env("DATABASE_URL") {
        Some(u) => u,
        None => {
            eprintln!(
                "SKIP: DATABASE_URL not set — real-engine scorm tests need a \
                 disposable environment (Principle XVII). The fresh-container \
                 run is the deciding evidence."
            );
            return;
        }
    };
    assert!(
        env("VALKEY_URL").is_some(),
        "the full-env battery asserts both backends are configured"
    );

    // 1. Migrations (foundation + scorm) — idempotent on the real engine.
    let storage = TiberiusStorage::connect(&url)
        .await
        .expect("connect storage");
    // The FULL host manifest (all mounted slices) — the post-state probe
    // asserts the five-migration state the real host produces (T018).
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
    manifest.extend(SCORM_MIGRATIONS.iter().map(|(n, sql)| (*n, *sql)));
    manifest.sort_by(|a, b| a.0.cmp(b.0));
    let applied = storage
        .run_migrations(&manifest)
        .await
        .expect("migration 1");
    eprintln!("applied on run 1: {applied:?}");
    let applied2 = storage
        .run_migrations(&manifest)
        .await
        .expect("migration 2");
    assert!(
        applied2.is_empty(),
        "idempotent: second run is a no-op (applied2={applied2:?})"
    );

    // 2. Repo against the real engine.
    let repo = TiberiusScormRepo::connect(&url)
        .await
        .expect("connect repo");
    repo.verify().await.expect("verify");

    // 3. The US1.6 RACE PROOF: N=8 independent repos (one connection each)
    // contest the UNIQUE (session_id, course_id) constraint on the real
    // engine. A unique session suffix keeps repeat runs from colliding
    // with their own previous rows.
    let suffix = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let race_session = format!("{SESSION}-{suffix}");
    let race_course = format!("race-course-{suffix}");
    let mut repos: Vec<Arc<TiberiusScormRepo>> = Vec::with_capacity(8);
    for _ in 0..8 {
        repos.push(Arc::new(
            TiberiusScormRepo::connect(&url)
                .await
                .expect("race repo connect"),
        ));
    }
    // 8 independent repo instances, one connection each — a single-client
    // repo would serialize the contest. (Spawn ≠ immediate execution; the
    // proof stands on the independent connections + engine arbitration +
    // the row-count/code assertions below.)
    let handles: Vec<_> = repos
        .iter()
        .map(|r| {
            let r = Arc::clone(r);
            let session = race_session.clone();
            let course = race_course.clone();
            let attempt = scorm::repo::ScormAttempt {
                id: String::new(), // the engine assigns the NEWID-string id
                session_id: session,
                course_id: course,
                organization_id: ORG.to_string(),
                state: "in_progress".to_string(),
                suspend_data: String::new(),
                score_raw: None,
                session_time: 0,
                finished_at: None,
                created_at: 0,
            };
            tokio::spawn(async move { r.create(&attempt).await })
        })
        .collect();
    let mut results: Vec<Result<(), ScormError>> = Vec::with_capacity(8);
    for h in handles {
        results.push(h.await.expect("race join"));
    }
    let wins = results.iter().filter(|r| r.is_ok()).count();
    let losses = results
        .iter()
        .filter(|r| matches!(r, Err(ScormError::AttemptExists)))
        .count();
    assert_eq!(wins, 1, "exactly one winner; results: {results:?}");
    assert_eq!(
        losses, 7,
        "the losers ALL got the stable code (2601/2627 mapping fired — never a \
         500, never a second row): {results:?}"
    );

    // The loser reads the winner's row (the launch procedure's
    // read-winner step) — exactly one row exists in the engine.
    let winner = repo
        .find_by_session_course(&race_session, &race_course)
        .await
        .expect("race read")
        .expect("the winner's row exists");
    assert_eq!(winner.organization_id, ORG);
    assert_eq!(winner.state, "in_progress");

    // 4. Commit durability across a reconnect (FR-004: the commit is the
    // durability anchor — a dropped connection must not drop the state).
    let mut row = repo
        .find_by_session_course(SESSION, "durability-course")
        .await
        .expect("durability pre-read");
    let seed = scorm::repo::ScormAttempt {
        id: String::new(),
        session_id: SESSION.to_string(),
        course_id: "durability-course".to_string(),
        organization_id: ORG.to_string(),
        state: "in_progress".to_string(),
        suspend_data: String::new(),
        score_raw: None,
        session_time: 0,
        finished_at: None,
        created_at: 0,
    };
    match row {
        Some(existing) => row = Some(existing),
        None => {
            repo.create(&seed).await.expect("durability create");
            row = repo
                .find_by_session_course(SESSION, "durability-course")
                .await
                .expect("durability read-back")
        }
    }
    let row = row.expect("the row");
    repo.update_committed(&row.id, "in_progress", "checkpoint-A", Some(55), 120)
        .await
        .expect("commit A");
    // Drop `repo` (its connection pool) — a fresh connection must see the
    // committed state.
    drop(repo);
    let repo2 = TiberiusScormRepo::connect(&url).await.expect("reconnect");
    let reread = repo2
        .find_by_session_course(SESSION, "durability-course")
        .await
        .expect("reconnect read")
        .expect("the row survives the reconnect");
    assert_eq!(
        reread.suspend_data, "checkpoint-A",
        "committed suspend survives"
    );
    assert_eq!(reread.score_raw, Some(55), "committed score survives");
    assert_eq!(reread.session_time, 120, "committed time survives");

    // 5. The finished_at belt on the real engine (FR-005: a second
    // finalize is a no-op-belt miss → AttemptFinal; first-wins).
    repo2
        .finalize(&reread.id, "passed", "final-suspend", Some(87), 600)
        .await
        .expect("finalize");
    let fin = repo2
        .find_by_session_course(SESSION, "durability-course")
        .await
        .expect("finish read")
        .unwrap();
    assert_eq!(fin.state, "passed");
    assert_eq!(fin.score_raw, Some(87));
    assert!(fin.finished_at.is_some(), "finished_at set");
    let belt = repo2.finalize(&reread.id, "failed", "x", None, 1).await;
    assert!(
        matches!(belt, Err(ScormError::AttemptFinal)),
        "second finalize → the belt fires (never a re-record): {belt:?}"
    );
    // (A post-finish COMMIT belt is app-level — the routes re-read and
    // gate on finished_at before persisting; the HTTP flow test proves
    // the 409. The repo's update_committed deliberately has no belt:
    // the engine guard is the finalize WHERE clause, the app is the
    // commit gate — the two layers stay separable.)

    // 6. FR-011/FR-012 wire edge: a 4,000-char injection-shaped
    // suspend_data round-trips on the real engine (NVARCHAR(MAX) +
    // parameterized SQL — no truncation, no structural mutation).
    let row3 = scorm::repo::ScormAttempt {
        id: String::new(),
        session_id: format!("sess-payload-{suffix}"),
        course_id: format!("payload-course-{suffix}"),
        organization_id: ORG.to_string(),
        state: "in_progress".to_string(),
        suspend_data: String::new(),
        score_raw: None,
        session_time: 0,
        finished_at: None,
        created_at: 0,
    };
    repo2.create(&row3).await.expect("payload create");
    let row3 = repo2
        .find_by_session_course(&row3.session_id, &row3.course_id)
        .await
        .expect("payload read")
        .unwrap();
    let prefix = "'); DROP TABLE scorm_attempts;-- <script>alert(1)</script>".to_string();
    let payload = format!("{}{}", prefix, "x".repeat(4_000 - prefix.len()));
    assert_eq!(payload.len(), 4_000, "the boundary is inclusive");
    repo2
        .update_committed(&row3.id, "in_progress", &payload, None, 0)
        .await
        .expect("payload commit");
    let reread3 = repo2
        .find_by_session_course(&row3.session_id, &row3.course_id)
        .await
        .expect("payload read-back")
        .unwrap();
    assert_eq!(
        reread3.suspend_data, payload,
        "4,000-char injection-shaped suspend round-trips as plain text"
    );

    // 7. Post-state probe (the 004 convention: dbo.schema_migrations):
    // a fresh DB holds exactly the five migrations (0001, 0002, 0003,
    // 0005, 0006). A direct engine connection (the probe is a test-side
    // read-only inspection, not a module path).
    let (host, port, user, pass, db, encrypt) = parse_probe_url(&url).expect("probe parse");
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
        .expect("probe tcp");
    tcp.set_nodelay(true).expect("probe nodelay");
    let mut probe = tiberius::Client::connect(config, tcp.compat_write())
        .await
        .expect("probe tds");
    let mut probe_stream = probe
        .query("SELECT name FROM dbo.schema_migrations ORDER BY name", &[])
        .await
        .expect("probe query");
    let mut rows: Vec<String> = Vec::new();
    while let Some(item) = probe_stream.next().await {
        let item = item.expect("probe stream");
        if let tiberius::QueryItem::Row(row) = item {
            if let Some(s) = row.get::<&str, _>(0) {
                rows.push(s.to_string());
            }
        }
    }
    eprintln!("post-state schema_migrations: {rows:?}");
    for want in [
        "0001_base",
        "0002_seed",
        "0003_catalog",
        "0005_enrollment",
        "0006_scorm",
    ] {
        assert!(rows.contains(&want.to_string()), "{want} missing: {rows:?}");
    }
    assert_eq!(
        rows.len(),
        5,
        "exactly the five migrations on a fresh DB: {rows:?}"
    );

    eprintln!("FULL-ENV SCORM: OK (real MSSQL)");
}

/// Minimal probe URL parser (mirrors the 001 parse discipline: the db
/// name is derived AFTER stripping the query string — the CR-B lesson;
/// encrypt=disable|off|false → OFF). mssql://user:pass@host:port/db.
fn parse_probe_url(url: &str) -> Result<(String, u16, String, String, String, bool), String> {
    let rest = url
        .strip_prefix("mssql://")
        .or_else(|| url.strip_prefix("sqlserver://"))
        .ok_or_else(|| format!("bad url scheme: {url}"))?;
    // CR-B: strip the query string BEFORE deriving the db name.
    let (rest, query) = rest.split_once('?').unwrap_or((rest, ""));
    // Mirror the 001 canonical semantics: no encrypt param → ON;
    // encrypt=disable|off|false → OFF.
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
    let (host, portdb) = hostpart
        .split_once(':')
        .ok_or_else(|| format!("missing port: {url}"))?;
    let (port, db) = portdb
        .split_once('/')
        .ok_or_else(|| format!("missing db: {url}"))?;
    Ok((
        host.to_string(),
        port.parse().map_err(|e| format!("bad port: {e}"))?,
        user.to_string(),
        pass.to_string(),
        db.to_string(),
        encrypt,
    ))
}
