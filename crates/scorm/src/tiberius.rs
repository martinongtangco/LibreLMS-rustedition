//! The real-engine `ScormRepo` (spec 005, Module Persistence Seam): tiberius
//! against `dbo.scorm_attempts` (0006_scorm.sql). Parameterized queries
//! throughout; the double's semantics (gate/idempotent-resume/finish-
//! discipline) are the contract.
//!
//! US1.6 (plan): the engine's UNIQUE (session_id, course_id) constraint is
//! the final arbiter under concurrency — a unique-constraint violation
//! (MSSQL 2601/2627 duplicate-key — BOTH codes observed on real 2022) maps
//! to `AttemptExists`, never a 500, never a second row. Both error legs
//! must map it: tiberius 0.13 can resolve the `query()` future OK even when
//! the server rejects the batch, with the error surfacing during stream
//! consumption (`into_row`) — proven on the live engine by the 004 battery.
//!
//! Guard-scoping (the 003 CR-9 standing instruction): every write executes
//! inside a scoped guard block, dropped BEFORE any sibling call — the client
//! mutex is non-reentrant (a sibling call while holding it self-deadlocks;
//! the in-memory double has no mutex, so the deterministic suite is
//! structurally blind to it).

use std::sync::Arc;

use async_trait::async_trait;
use shared::session::{SessionStore, StoreError};
use tiberius::{AuthMethod, Client, Config, EncryptionLevel, ToSql};
use tokio::net::TcpStream;
use tokio::sync::Mutex;
use tokio_util::compat::TokioAsyncWriteCompatExt;

use crate::repo::{BagStore, ScormAttempt, ScormError, ScormRepo};

/// The whole-script-batch URL parse is the 001/003/004 one (db name clean,
/// encrypt disable|off|false → OFF — the 001 CR-B fix).
fn parse_database_url(url: &str) -> Result<(String, u16, String, String, String, bool), String> {
    let rest = url
        .strip_prefix("mssql://")
        .ok_or_else(|| format!("DATABASE_URL must use the mssql:// scheme: {url}"))?;
    let (auth, hostpart) = rest
        .split_once('@')
        .ok_or_else(|| "DATABASE_URL missing user:pass@host".to_string())?;
    let (user, pass) = auth
        .split_once(':')
        .ok_or_else(|| "DATABASE_URL missing user:pass".to_string())?;
    let (noquery, query) = match hostpart.split_once('?') {
        Some((hp, q)) => (hp, q),
        None => (hostpart, ""),
    };
    let (hostport, db) = match noquery.rsplit_once('/') {
        Some((hp, db)) => (hp.to_string(), db.to_string()),
        None => (noquery.to_string(), String::new()),
    };
    let (host, port) = match hostport.rsplit_once(':') {
        Some((h, p)) => (
            h.to_string(),
            p.parse::<u16>()
                .map_err(|_| format!("DATABASE_URL bad port in {hostport}"))?,
        ),
        None => (hostport.to_string(), 1433u16),
    };
    if db.is_empty() {
        return Err("DATABASE_URL missing database name".to_string());
    }
    let encrypt = match query.split('&').find_map(|kv| {
        kv.split_once('=')
            .filter(|(k, _)| *k == "encrypt")
            .map(|(_, v)| v.to_lowercase())
    }) {
        Some(v) => !matches!(v.as_str(), "disable" | "off" | "false"),
        None => true,
    };
    Ok((user.to_string(), port, host, db, pass.to_string(), encrypt))
}

/// The unique-constraint violation (MSSQL duplicate-key: 2601 general /
/// 2627 with index name) is the US1.6 engine anchor: it maps to
/// `AttemptExists` (the loser reads the winner's row), never a 500, never a
/// second row. Every other server error is an outage (503).
fn map_unique(e: tiberius::error::Error) -> ScormError {
    if matches!(e.code(), Some(2601) | Some(2627)) {
        ScormError::AttemptExists
    } else {
        ScormError::Unavailable
    }
}

fn now_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// A single client behind a mutex (001/003/004 pattern — no pool layer,
/// Principle II). The US1.6 race test uses N INDEPENDENT instances (one
/// connection each) so the constraint sees a real contest (plan).
pub struct TiberiusScormRepo {
    client: Arc<Mutex<Option<Client<tokio_util::compat::Compat<TcpStream>>>>>,
}

impl TiberiusScormRepo {
    pub async fn connect(url: &str) -> Result<Self, ScormError> {
        let (user, port, host, db, pass, encrypt) =
            parse_database_url(url).map_err(|e| ScormError::Validation {
                field: "DATABASE_URL".to_string(),
                message: e,
            })?;
        let mut config = Config::new();
        config.database(&db);
        config.authentication(AuthMethod::sql_server(&user, &pass));
        config.encryption(if encrypt {
            EncryptionLevel::On
        } else {
            EncryptionLevel::NotSupported
        });
        if encrypt {
            config.trust_cert();
        }
        let tcp = TcpStream::connect((host.as_str(), port))
            .await
            .map_err(|_| ScormError::Unavailable)?;
        tcp.set_nodelay(true).map_err(|_| ScormError::Unavailable)?;
        let client = Client::connect(config, tcp.compat_write())
            .await
            .map_err(|_| ScormError::Unavailable)?;
        Ok(Self {
            client: Arc::new(Mutex::new(Some(client))),
        })
    }

    /// Production connection proof (mirrors 001/003/004).
    pub async fn verify(&self) -> Result<(), ScormError> {
        let mut g = self.guard().await?;
        let g = g.as_mut().ok_or(ScormError::Unavailable)?;
        let n: i32 = g
            .query("SELECT 1", &[])
            .await
            .map_err(|_| ScormError::Unavailable)?
            .into_row()
            .await
            .map_err(|_| ScormError::Unavailable)?
            .and_then(|r| r.get::<i32, _>(0))
            .unwrap_or(0);
        if n == 1 {
            Ok(())
        } else {
            Err(ScormError::Unavailable)
        }
    }

    async fn guard(
        &self,
    ) -> Result<
        tokio::sync::MutexGuard<'_, Option<Client<tokio_util::compat::Compat<TcpStream>>>>,
        ScormError,
    > {
        let g = self.client.lock().await;
        if g.is_none() {
            return Err(ScormError::Unavailable);
        }
        Ok(g)
    }

    /// Row read — STRING columns only (plus the INT/BIGINT numerics): this
    /// table has no UNIQUEIDENTIFIER columns (the wire discipline), so no
    /// Guid/DateTime2 materialization is possible by construction (the 003
    /// CR-7/CR-8 class).
    fn parse_row(row: &tiberius::Row) -> Option<ScormAttempt> {
        let id: Option<&str> = row.get(0);
        let session: Option<&str> = row.get(1);
        let course: Option<&str> = row.get(2);
        let org: Option<&str> = row.get(3);
        let state: Option<&str> = row.get(4);
        let suspend: Option<&str> = row.get(5);
        let score: Option<i32> = row.get(6);
        let time: Option<i64> = row.get(7);
        let finished: Option<i64> = row.get(8);
        let created: Option<i64> = row.get(9);
        Some(ScormAttempt {
            id: id?.to_string(),
            session_id: session?.to_string(),
            course_id: course?.to_string(),
            organization_id: org?.to_string(),
            state: state?.to_string(),
            suspend_data: suspend.unwrap_or_default().to_string(),
            score_raw: score,
            session_time: time.unwrap_or(0),
            finished_at: finished,
            created_at: created.unwrap_or(0),
        })
    }

    async fn select_by_session_course(
        &self,
        session_id: &str,
        course_id: &str,
    ) -> Result<Option<ScormAttempt>, ScormError> {
        let session = session_id.to_string();
        let course = course_id.to_string();
        let mut g = self.guard().await?;
        let g = g.as_mut().ok_or(ScormError::Unavailable)?;
        let stream = g
            .query(
                "SELECT TOP 1 id, session_id, course_id, organization_id, state, suspend_data, score_raw, session_time, finished_at, created_at
                 FROM dbo.scorm_attempts
                 WHERE session_id = @P1 AND course_id = @P2",
                &[&session as &dyn ToSql, &course as &dyn ToSql],
            )
            .await
            .map_err(|_| ScormError::Unavailable)?;
        let row = stream
            .into_row()
            .await
            .map_err(|_| ScormError::Unavailable)?;
        let Some(row) = row else {
            return Ok(None);
        };
        // A row that fails to parse is an outage-class surprise (every
        // selected column is a string/int/bigint the row can materialize —
        // the wire discipline), never a silent None (FR-010).
        Self::parse_row(&row)
            .map(Some)
            .ok_or(ScormError::Unavailable)
    }
}

#[async_trait]
impl ScormRepo for TiberiusScormRepo {
    async fn find_by_session_course(
        &self,
        session_id: &str,
        course_id: &str,
    ) -> Result<Option<ScormAttempt>, ScormError> {
        self.select_by_session_course(session_id, course_id).await
    }

    async fn create(&self, attempt: &ScormAttempt) -> Result<(), ScormError> {
        let now = now_secs();
        let now_sql = now.to_string();
        let session = attempt.session_id.clone();
        let course = attempt.course_id.clone();
        let org = attempt.organization_id.clone();
        // Guard-scoped write (003 CR-9 lesson): the guard is dropped before
        // any sibling call — the client mutex is non-reentrant.
        {
            let mut g = self.guard().await?;
            let g = g.as_mut().ok_or(ScormError::Unavailable)?;
            // One round-trip: NEWID-string id (the wire discipline), the
            // committed state initialized from the CmiBag::fresh shape the
            // caller carries.
            //
            // NOTE (tiberius 0.13): the query() future resolves OK even when
            // the server rejects the batch — a 2601/2627 unique-violation
            // surfaces when the stream is consumed (into_row). Both legs
            // must map it (US1.6 engine anchor).
            let stream = g
                .query(
                    "DECLARE @id NVARCHAR(40) = CAST(NEWID() AS NVARCHAR(40));
                     INSERT INTO dbo.scorm_attempts
                         (id, session_id, course_id, organization_id, state, suspend_data, score_raw, session_time, finished_at, created_at, updated_at)
                     VALUES
                         (@id, @P1, @P2, @P3, @P4, @P5, @P6, @P7, NULL, @P8, @P8)",
                    &[
                        &session as &dyn ToSql,
                        &course as &dyn ToSql,
                        &org as &dyn ToSql,
                        &attempt.state.clone() as &dyn ToSql,
                        &attempt.suspend_data.clone() as &dyn ToSql,
                        &attempt.score_raw as &dyn ToSql,
                        &attempt.session_time.to_string() as &dyn ToSql,
                        &now_sql as &dyn ToSql,
                    ],
                )
                .await
                .map_err(map_unique)?;
            stream.into_row().await.map_err(map_unique)?;
        }
        Ok(())
    }

    async fn update_committed(
        &self,
        id: &str,
        state: &str,
        suspend_data: &str,
        score_raw: Option<i32>,
        session_time: i64,
    ) -> Result<(), ScormError> {
        self.persist(id, state, suspend_data, score_raw, session_time, false)
            .await
    }

    async fn finalize(
        &self,
        id: &str,
        state: &str,
        suspend_data: &str,
        score_raw: Option<i32>,
        session_time: i64,
    ) -> Result<(), ScormError> {
        self.persist(id, state, suspend_data, score_raw, session_time, true)
            .await
    }
}

impl TiberiusScormRepo {
    /// The commit/finalize write (one scoped guard block — the 003 CR-9
    /// lesson). `finalizing` sets `finished_at` (exactly once — the belt:
    /// the WHERE clause makes a second finalize a 0-row no-op that the
    /// caller maps to `AttemptFinal` at the app layer; the double mirrors
    /// the same belt).
    async fn persist(
        &self,
        id: &str,
        state: &str,
        suspend_data: &str,
        score_raw: Option<i32>,
        session_time: i64,
        finalizing: bool,
    ) -> Result<(), ScormError> {
        let now = now_secs();
        let now_sql = now.to_string();
        let iid = id.to_string();
        let state_s = state.to_string();
        let suspend = suspend_data.to_string();
        let time_s = session_time.to_string();
        let affected;
        {
            let mut g = self.guard().await?;
            let g = g.as_mut().ok_or(ScormError::Unavailable)?;
            if finalizing {
                // The finished_at belt: only a LIVE row is finalizable — a
                // second finalize matches 0 rows (never a re-record).
                affected = g
                    .execute(
                        "UPDATE dbo.scorm_attempts
                         SET state = @P1, suspend_data = @P2, score_raw = @P3,
                             session_time = @P4, updated_at = @P5, finished_at = @P5
                         WHERE id = @P6 AND finished_at IS NULL",
                        &[
                            &state_s as &dyn ToSql,
                            &suspend as &dyn ToSql,
                            &score_raw as &dyn ToSql,
                            &time_s as &dyn ToSql,
                            &now_sql as &dyn ToSql,
                            &iid as &dyn ToSql,
                        ],
                    )
                    .await
                    .map_err(|_| ScormError::Unavailable)?;
            } else {
                affected = g
                    .execute(
                        "UPDATE dbo.scorm_attempts
                         SET state = @P1, suspend_data = @P2, score_raw = @P3,
                             session_time = @P4, updated_at = @P5
                         WHERE id = @P6",
                        &[
                            &state_s as &dyn ToSql,
                            &suspend as &dyn ToSql,
                            &score_raw as &dyn ToSql,
                            &time_s as &dyn ToSql,
                            &now_sql as &dyn ToSql,
                            &iid as &dyn ToSql,
                        ],
                    )
                    .await
                    .map_err(|_| ScormError::Unavailable)?;
            }
        }
        let n = affected.rows_affected().iter().sum::<u64>();
        if n == 0 {
            // A commit on a vanished row is an outage-class surprise (the
            // app layer re-reads before persisting; the double mirrors the
            // NotFound mapping). A finalize belt-miss is `AttemptFinal`.
            return if finalizing {
                Err(ScormError::AttemptFinal)
            } else {
                Err(ScormError::NotFound)
            };
        }
        Ok(())
    }
}

/// The production `BagStore`: wraps the shared session store (the 001-
/// reserved key `lms:session:{token_hash}:bag` is constructed INSIDE the
/// shared store — the port never names it, plan FR-003). `StoreError`
/// variants all map to `Unavailable` (503 — the bag store's ephemerality
/// never leaks a fake absence).
pub struct SessionBagStore {
    store: Arc<dyn SessionStore>,
}

impl SessionBagStore {
    pub fn new(store: Arc<dyn SessionStore>) -> Self {
        Self { store }
    }
}

fn map_store(e: &StoreError) -> ScormError {
    // Every StoreError variant is an outage-class condition (the shared
    // store has no domain errors — its failures are connectivity/TTL
    // infrastructure). 503, never a fake absence.
    let _ = e;
    ScormError::Unavailable
}

#[async_trait]
impl BagStore for SessionBagStore {
    async fn get_bag(&self, token_hash: &str) -> Result<Option<String>, ScormError> {
        self.store
            .get_bag(token_hash)
            .await
            .map_err(|e| map_store(&e))
    }

    async fn set_bag(&self, token_hash: &str, bag: &str, ttl_secs: u64) -> Result<(), ScormError> {
        self.store
            .set_bag(token_hash, bag, ttl_secs)
            .await
            .map_err(|e| map_store(&e))
    }
}
