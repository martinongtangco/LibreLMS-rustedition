//! US2/US3 — the management real-engine battery (spec 006, T013, gated
//! full-env). GATED: runs only when DATABASE_URL + VALKEY_URL are set (the
//! fresh-container run is the deciding evidence, Principle XVI; a SKIP is
//! visible, never silent — Principle XVII, the runner must verify the
//! `applied:` line / real duration before recording a pass).

use futures::StreamExt;
use rand::SeedableRng;
use tokio_util::compat::TokioAsyncWriteCompatExt;

use contracts::Role;
use management::migrations::MIGRATIONS as MANAGEMENT_MIGRATIONS;
use management::repo::{ManagementError, ManagementRepo};
use management::tiberius::TiberiusManagementRepo;
use rand::rngs::StdRng;
use shared::auth;
use shared::storage::{Storage as _, TiberiusStorage};

fn env(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|v| !v.is_empty())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn management_full_env_battery() {
    let url = match env("DATABASE_URL") {
        Some(u) => u,
        None => {
            eprintln!(
                "SKIP: DATABASE_URL not set — real-engine management tests need \
                 a disposable environment (Principle XVII). The fresh-container \
                 run is the deciding evidence (Principle XVI)."
            );
            return;
        }
    };
    assert!(
        env("VALKEY_URL").is_some(),
        "the full-env battery asserts both backends are configured"
    );

    // ---------- (b) the full host manifest — idempotent, empty-mgmt no-op ----------
    let storage = TiberiusStorage::connect(&url)
        .await
        .expect("connect storage");
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
    manifest.extend(MANAGEMENT_MIGRATIONS.iter().map(|(n, sql)| (*n, *sql)));
    // CR-1 fix (the Phase-4 shared-container runner — the note is now
    // folded): the strict `len == 5` is first-touch order-sensitive in a
    // SHARED-CYCLE run (a runner that already applied part of the manifest
    // observes a smaller `applied` set — the 2026-11-07 review observed
    // [0003, 0006] at 0.11s, a runner artifact, not a product bug).
    // Order-invariant variant: the DELTA is asserted only when the database
    // was fresh; the post-state SET is asserted on every run.
    let pre: i32 = probe_migration_count(&url).await.unwrap_or(0);
    let applied = storage
        .run_migrations(&manifest)
        .await
        .expect("first run applies the full manifest");
    let names: Vec<String> = applied.clone();
    eprintln!("applied: {names:?} (pre-state migration rows: {pre})");
    if pre == 0 {
        assert_eq!(
            applied.len(),
            5,
            "a FRESH database applies exactly the five non-empty scripts \
         (management is empty by construction), got {names:?}"
        );
    }
    // The post-state SET (order-invariant on every run — a shared-cycle
    // run still converges to the full set).
    let set: Vec<String> = probe_migration_set(&url)
        .await
        .expect("post-state migration set");
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
    let applied2 = storage.run_migrations(&manifest).await.expect("second run");
    assert!(
        applied2.is_empty(),
        "idempotency on the FRESH db: the second run applies NOTHING, \
         got {:?}",
        applied2.clone()
    );
    // The empty management manifest alone: a verified no-op even on the
    // populated database (the runner's &[] contract — 001 CR-2).
    let applied_mgmt = storage
        .run_migrations(MANAGEMENT_MIGRATIONS)
        .await
        .expect("empty manifest run");
    assert!(
        applied_mgmt.is_empty(),
        "the empty management manifest is a no-op, got {:?}",
        applied_mgmt.clone()
    );

    // ---------- (a) the duplicate-email RACE PROOF (the 004 CR-1 lesson) ----------
    // N INDEPENDENT repo instances (one connection each) — never N tasks on
    // one shared-client repo (that serializes and proves nothing).
    const N: usize = 8;
    let repo = TiberiusManagementRepo::connect(&url)
        .await
        .expect("management connect");
    let org = repo
        .create_org(&format!("race-org-{}", std::process::id()))
        .await
        .expect("race org");
    let email = format!("race-{}-{}@e.com", std::process::id(), org.id);
    let results: Vec<Result<management::repo::ManagedUser, ManagementError>> =
        futures::stream::iter(
            (0..N)
                .map(|_| {
                    let url = url.clone();
                    let email = email.clone();
                    let org_id = org.id.clone();
                    async move {
                        let r = TiberiusManagementRepo::connect(&url)
                            .await
                            .expect("independent connect (one connection each)");
                        r.create_user(&org_id, &email, "Racer", &Role::Member).await
                    }
                })
                .collect::<Vec<_>>(),
        )
        .buffer_unordered(N)
        .collect()
        .await;
    let oks = results.iter().filter(|r| r.is_ok()).count();
    let dupes = results
        .iter()
        .filter(|r| matches!(r, Err(ManagementError::DuplicateEmail)))
        .count();
    eprintln!("race: {oks} ok, {dupes} duplicate-email, total {N}");
    assert!(
        oks >= 1 && oks + dupes == N,
        "every racer answers a stable code (Ok or DuplicateEmail) — \
         never a 500-shaped error: oks={oks} dupes={dupes}"
    );
    // Exactly ONE user row: a 9th independent connection counts the
    // membership rows for the org.
    let arbiter = TiberiusManagementRepo::connect(&url)
        .await
        .expect("arbiter connect");
    let users = arbiter.list_users(&org.id).await.expect("list after race");
    assert_eq!(
        users.len(),
        1,
        "exactly ONE user row survives the race (the engine's UNIQUE \
         constraint is the final arbiter), got {}",
        users.len()
    );

    // ---------- (d) the never-500 battery (stable codes on the real engine) ----------
    // A validation rejection (app layer, before SQL) — 400-shaped, never 500.
    let v = repo.create_org("").await;
    assert!(
        matches!(v, Err(ManagementError::Validation { .. })),
        "empty org name → Validation (never a 500), got {v:?}"
    );
    // A not-empty delete refusal (the membership guard on the real engine).
    let guarded = repo
        .create_org(&format!("guarded-{}", std::process::id()))
        .await
        .expect("guarded org");
    arbiter
        .create_user(
            &guarded.id,
            &format!("g-{}@e.com", std::process::id()),
            "G",
            &Role::Member,
        )
        .await
        .expect("fill the org");
    let d = repo.delete_org(&guarded.id).await;
    assert!(
        matches!(d, Err(ManagementError::NotEmpty)),
        "non-empty delete → NotEmpty (never a 500, never a silent cascade), got {d:?}"
    );

    // ---------- (c) the org-delete session cascade (sessions-before-org-row) ----------
    let rng = &mut StdRng::seed_from_u64(std::process::id() as u64);
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64;
    // Target org A: user U with a membership + a LIVE 001 session in A.
    let org_a = repo
        .create_org(&format!("cascade-A-{}", std::process::id()))
        .await
        .expect("org A");
    let u = repo
        .create_user(
            &org_a.id,
            &format!("u-{}@e.com", std::process::id()),
            "U",
            &Role::Member,
        )
        .await
        .expect("user U");
    let (token_a, row_a) =
        auth::establish(&storage, rng, &u.user.id, &org_a.id, now, 3_600, 72_000)
            .await
            .expect("establish in A (membership exists)");
    // Foreign org B: user W with a membership + a LIVE 001 session in B.
    let org_b = repo
        .create_org(&format!("cascade-B-{}", std::process::id()))
        .await
        .expect("org B");
    let w = repo
        .create_user(
            &org_b.id,
            &format!("w-{}@e.com", std::process::id()),
            "W",
            &Role::Member,
        )
        .await
        .expect("user W");
    let (token_b, _row_b) =
        auth::establish(&storage, rng, &w.user.id, &org_b.id, now, 3_600, 72_000)
            .await
            .expect("establish in B");
    // Both sessions verify before the delete.
    auth::verify(&storage, &token_a, &org_a.id, now, 3_600, 72_000)
        .await
        .expect("A session verifies before the delete");
    auth::verify(&storage, &token_b, &org_b.id, now, 3_600, 72_000)
        .await
        .expect("B session verifies before the delete");
    // Empty A (remove the membership), then delete A.
    repo.remove_membership(&org_a.id, &u.user.id)
        .await
        .expect("remove U from A (A is now empty)");
    repo.delete_org(&org_a.id)
        .await
        .expect("delete empty org A");
    // The cascade (CR-2b): A's session row is GONE — verify 401-shapes;
    // B's session SURVIVES (other orgs' sessions are untouched).
    let va = auth::verify(&storage, &token_a, &org_a.id, now, 3_600, 72_000).await;
    assert!(
        matches!(va, Err(shared::auth::AuthError::Unauthenticated)),
        "the deleted org's session is cascade-expired (401-shape), got {va:?}"
    );
    auth::verify(&storage, &token_b, &org_b.id, now, 3_600, 72_000)
        .await
        .expect("the foreign org's session survives the cascade");
    // The session row is physically gone (not just verify-scoped): the
    // system of record has no row for the hash.
    assert!(
        storage
            .get_session(&row_a.token_hash)
            .await
            .expect("get_session")
            .is_none(),
        "A's session row is physically deleted (sessions-before-org-row)"
    );

    // ---------- (e) durability across a fresh client reconnect ----------
    let durable = repo
        .create_org(&format!("durable-{}", std::process::id()))
        .await
        .expect("create via the first client");
    drop(repo);
    let reconnected = TiberiusManagementRepo::connect(&url)
        .await
        .expect("fresh client reconnect");
    let listed = reconnected.list_orgs().await.expect("list after reconnect");
    assert!(
        listed.iter().any(|(o, _)| o.id == durable.id),
        "the org created by a dropped client is visible to a fresh client \
         (the system of record is the database, not the connection)"
    );
}

/// A direct engine connection (a test-side read-only inspection — the
/// 005 convention).
async fn probe_migration_count(url: &str) -> Result<i32, String> {
    let (host, port, user, pass, db, encrypt) = parse_probe_mgmt(url)?;
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
        .query("SELECT COUNT(*) FROM dbo.schema_migrations", &[])
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
async fn probe_migration_set(url: &str) -> Result<Vec<String>, String> {
    let (host, port, user, pass, db, encrypt) = parse_probe_mgmt(url)?;
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
        .query("SELECT name FROM dbo.schema_migrations ORDER BY name", &[])
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
fn parse_probe_mgmt(url: &str) -> Result<(String, u16, String, String, String, bool), String> {
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
