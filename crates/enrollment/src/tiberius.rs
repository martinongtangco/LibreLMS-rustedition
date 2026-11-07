//! The real-engine `EnrollmentRepo` (spec 004, Module Persistence Seam):
//! tiberius against `dbo.enrollments` / `dbo.completions` (0005_enrollment.sql).
//! Parameterized queries throughout; the double's semantics (scope/404-oracle/
//! first-wins) are the contract.
//!
//! SC-003 (plan PQ-1): the engine's UNIQUE (learner_id, course_id) constraint
//! is the final arbiter under concurrency — a unique-constraint violation
//! (MSSQL 2601/2627 duplicate-key) maps to `AlreadyEnrolled`, never a 500,
//! never a second row. The completion PK analog maps to `AlreadyCompleted`.

use std::sync::Arc;

use async_trait::async_trait;
use futures::StreamExt;
use tiberius::{AuthMethod, Client, Config, EncryptionLevel, ToSql};
use tokio::net::TcpStream;
use tokio::sync::Mutex;
use tokio_util::compat::TokioAsyncWriteCompatExt;

use crate::repo::{Enrollment, EnrollmentError, EnrollmentRepo, Status};

/// The whole-script-batch URL parse is the 001 mssql one (db name clean,
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
/// 2627 with index name — both observed on real 2022) is the SC-003 engine
/// anchor: it maps to the given stable code, never a 500, never a silent
/// second row. Every other server error is an outage (503).
fn map_unique(e: tiberius::error::Error, unique: EnrollmentError) -> EnrollmentError {
    if matches!(e.code(), Some(2601) | Some(2627)) {
        unique
    } else {
        EnrollmentError::Unavailable
    }
}

fn now_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// A single client behind a mutex (001/003 pattern — no pool layer,
/// Principle II). The SC-003 race test uses N INDEPENDENT instances (one
/// connection each) so the constraint sees a real contest (plan CR-1).
pub struct TiberiusEnrollmentRepo {
    client: Arc<Mutex<Option<Client<tokio_util::compat::Compat<TcpStream>>>>>,
}

impl TiberiusEnrollmentRepo {
    pub async fn connect(url: &str) -> Result<Self, EnrollmentError> {
        let (user, port, host, db, pass, encrypt) =
            parse_database_url(url).map_err(|e| EnrollmentError::Validation {
                field: "DATABASE_URL",
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
            .map_err(|_| EnrollmentError::Unavailable)?;
        tcp.set_nodelay(true)
            .map_err(|_| EnrollmentError::Unavailable)?;
        let client = Client::connect(config, tcp.compat_write())
            .await
            .map_err(|_| EnrollmentError::Unavailable)?;
        Ok(Self {
            client: Arc::new(Mutex::new(Some(client))),
        })
    }

    /// Production connection proof (mirrors 001/003).
    pub async fn verify(&self) -> Result<(), EnrollmentError> {
        let mut g = self.guard().await?;
        let g = g.as_mut().ok_or(EnrollmentError::Unavailable)?;
        let n: i32 = g
            .query("SELECT 1", &[])
            .await
            .map_err(|_| EnrollmentError::Unavailable)?
            .into_row()
            .await
            .map_err(|_| EnrollmentError::Unavailable)?
            .and_then(|r| r.get::<i32, _>(0))
            .unwrap_or(0);
        if n == 1 {
            Ok(())
        } else {
            Err(EnrollmentError::Unavailable)
        }
    }

    async fn guard(
        &self,
    ) -> Result<
        tokio::sync::MutexGuard<'_, Option<Client<tokio_util::compat::Compat<TcpStream>>>>,
        EnrollmentError,
    > {
        let g = self.client.lock().await;
        if g.is_none() {
            return Err(EnrollmentError::Unavailable);
        }
        Ok(g)
    }

    /// Row read: (id, learner_id, course_id, organization_id, status,
    /// created_at, completed_at?). `learner_id` is a UNIQUEIDENTIFIER column
    /// read as `&str` via a CAST at the SELECT wire edge — tiberius 0.13
    /// cannot interpret a Guid column as a String value (proven on the real
    /// engine by the full-env battery; the 003 slice's fetch has the same
    /// latent gap, CR for PR #2).
    fn parse_row(row: &tiberius::Row) -> Option<Enrollment> {
        let id: Option<&str> = row.get(0);
        let learner: Option<&str> = row.get(1);
        let course: Option<&str> = row.get(2);
        let org: Option<&str> = row.get(3);
        let status_s: Option<&str> = row.get(4);
        let created: Option<i64> = row.get(5);
        let completed: Option<i64> = row.get(6);
        let status = match status_s.unwrap_or_default() {
            "enrolled" => Status::Enrolled,
            "completed" => Status::Completed,
            _ => return None,
        };
        Some(Enrollment {
            id: id.unwrap_or_default().to_string(),
            learner_id: learner.unwrap_or_default().to_string(),
            course_id: course.unwrap_or_default().to_string(),
            organization_id: org.unwrap_or_default().to_string(),
            status,
            created_at: created.unwrap_or(0),
            completed_at: completed,
        })
    }

    async fn query_rows(
        &self,
        sql: &str,
        params: &[String],
    ) -> Result<Vec<Enrollment>, EnrollmentError> {
        let mut g = self.guard().await?;
        let g = g.as_mut().ok_or(EnrollmentError::Unavailable)?;
        let dyn_params: Vec<&dyn ToSql> = params.iter().map(|p| p as &dyn ToSql).collect();
        let stream = g
            .query(sql, &dyn_params)
            .await
            .map_err(|_| EnrollmentError::Unavailable)?;
        let mut out = Vec::new();
        let mut rows = stream.into_row_stream();
        while let Some(row) = rows.next().await {
            let row = row.map_err(|_| EnrollmentError::Unavailable)?;
            if let Some(e) = Self::parse_row(&row) {
                out.push(e);
            }
        }
        Ok(out)
    }
}

#[async_trait]
impl EnrollmentRepo for TiberiusEnrollmentRepo {
    async fn create(
        &self,
        learner_id: &str,
        course_id: &str,
        organization_id: &str,
    ) -> Result<Enrollment, EnrollmentError> {
        let now = now_secs();
        let now_sql = now.to_string();
        let learner = learner_id.to_string();
        let course = course_id.to_string();
        let org = organization_id.to_string();
        let mut g = self.guard().await?;
        let g = g.as_mut().ok_or(EnrollmentError::Unavailable)?;
        // One round-trip: NEWID() id returned via OUTPUT (contract ids are
        // strings; CAST at the wire edge — 001 convention).
        //
        // NOTE (tiberius 0.13): the query() future resolves OK even when the
        // server rejects the batch — a 2601/2627 unique-violation surfaces when the
        // stream is consumed (into_row). Both legs must map it.
        let stream = g
            .query(
                "DECLARE @id NVARCHAR(40) = CAST(NEWID() AS NVARCHAR(40));
                 INSERT INTO dbo.enrollments (id, learner_id, course_id, organization_id, status, created_at)
                 OUTPUT inserted.id
                 VALUES (@id, CAST(@P1 AS UNIQUEIDENTIFIER), @P2, @P3, N'enrolled', @P4)",
                &[&learner as &dyn ToSql, &course as &dyn ToSql, &org as &dyn ToSql, &now_sql as &dyn ToSql],
            )
            .await
            .map_err(|e| map_unique(e, EnrollmentError::AlreadyEnrolled))?;
        let row = stream
            .into_row()
            .await
            .map_err(|e| map_unique(e, EnrollmentError::AlreadyEnrolled))?;
        let Some(row) = row else {
            return Err(EnrollmentError::Unavailable);
        };
        let cid: Option<&str> = row.get(0);
        let cid = cid.unwrap_or_default();
        if cid.is_empty() {
            return Err(EnrollmentError::Unavailable);
        }
        Ok(Enrollment {
            id: cid.to_string(),
            learner_id: learner,
            course_id: course,
            organization_id: org,
            status: Status::Enrolled,
            created_at: now,
            completed_at: None,
        })
    }

    async fn list_for_learner(
        &self,
        learner_id: &str,
        organization_id: &str,
    ) -> Result<Vec<Enrollment>, EnrollmentError> {
        self.query_rows(
            "SELECT e.id, CAST(e.learner_id AS NVARCHAR(40)), e.course_id, e.organization_id, e.status, e.created_at, c.completed_at
             FROM dbo.enrollments e
             LEFT JOIN dbo.completions c ON c.enrollment_id = e.id
             WHERE e.learner_id = CAST(@P1 AS UNIQUEIDENTIFIER)
               AND e.organization_id = @P2
             ORDER BY e.id",
            &[learner_id.to_string(), organization_id.to_string()],
        )
        .await
    }

    async fn list_for_org(
        &self,
        organization_id: &str,
    ) -> Result<Vec<Enrollment>, EnrollmentError> {
        self.query_rows(
            "SELECT e.id, CAST(e.learner_id AS NVARCHAR(40)), e.course_id, e.organization_id, e.status, e.created_at, c.completed_at
             FROM dbo.enrollments e
             LEFT JOIN dbo.completions c ON c.enrollment_id = e.id
             WHERE e.organization_id = @P1
             ORDER BY e.id",
            &[organization_id.to_string()],
        )
        .await
    }

    async fn complete(
        &self,
        actor_id: &str,
        enrollment_id: &str,
        organization_id: &str,
    ) -> Result<Enrollment, EnrollmentError> {
        // Existence + scope first (404 vs 403 semantics — the no-oracle
        // discriminator on the real engine).
        let mut g = self.guard().await?;
        let g = g.as_mut().ok_or(EnrollmentError::Unavailable)?;
        let eid = enrollment_id.to_string();
        let row = g
            .query(
                "SELECT TOP 1 e.id, CAST(e.learner_id AS NVARCHAR(40)), e.course_id, e.organization_id, e.status, e.created_at, c.completed_at
                 FROM dbo.enrollments e
                 LEFT JOIN dbo.completions c ON c.enrollment_id = e.id
                 WHERE e.id = @P1",
                &[&eid as &dyn ToSql],
            )
            .await
            .map_err(|_| EnrollmentError::Unavailable)?
            .into_row()
            .await
            .map_err(|_| EnrollmentError::Unavailable)?;
        let Some(row) = row else {
            return Err(EnrollmentError::NotFound);
        };
        // Gate order: scope (403) before status — an out-of-scope row
        // answers Scope regardless of its status (no existence oracle).
        let org: Option<&str> = row.get(3);
        if org.unwrap_or_default() != organization_id {
            // Exists but foreign → Scope (403), not NotFound (no oracle).
            return Err(EnrollmentError::Scope);
        }
        // Only the enrolled learner records their own completion. The engine
        // normalizes UNIQUEIDENTIFIER casing (CAST read-back is uppercase),
        // so the comparison is case-insensitive (a no-op for the memory
        // double's session-source strings).
        let learner: Option<&str> = row.get(1);
        if !learner.unwrap_or_default().eq_ignore_ascii_case(actor_id) {
            return Err(EnrollmentError::Forbidden);
        }
        // Status gate (2-state design: only 'enrolled'/'completed' are
        // app-writable; any other value is the defensive NotEnrolled — the
        // forward-compatible gate, spec 004 two-state lifecycle / 005 hook).
        let status_s: Option<&str> = row.get(4);
        match status_s.unwrap_or_default() {
            "enrolled" => {}
            "completed" => {
                // First-wins: the original record is unchanged (no UPDATE
                // path — the PK is the engine anchor).
                return Err(EnrollmentError::AlreadyCompleted);
            }
            _ => return Err(EnrollmentError::NotEnrolled),
        }
        let id: Option<&str> = row.get(0);
        let course: Option<&str> = row.get(2);
        let created: Option<i64> = row.get(5);
        let ts = now_secs();
        let ts_sql = ts.to_string();
        // The PK is the first-wins engine anchor: a concurrent second
        // completion loses with 2627 → AlreadyCompleted (never a 500, never
        // a second record).
        g.execute(
            "INSERT INTO dbo.completions (enrollment_id, completed_at)
             VALUES (@P1, @P2)",
            &[&eid as &dyn ToSql, &ts_sql as &dyn ToSql],
        )
        .await
        .map_err(|e| map_unique(e, EnrollmentError::AlreadyCompleted))?;
        let affected = g
            .execute(
                "UPDATE dbo.enrollments SET status = N'completed'
                 WHERE id = @P1 AND status = N'enrolled'",
                &[&eid as &dyn ToSql],
            )
            .await
            .map_err(|e| map_unique(e, EnrollmentError::AlreadyCompleted))?;
        if affected.rows_affected().iter().sum::<u64>() == 0 {
            // Lost the status transition race — the completion INSERT above
            // was the primary guard; this is the belt.
            return Err(EnrollmentError::AlreadyCompleted);
        }
        let out = Enrollment {
            id: id.unwrap_or_default().to_string(),
            learner_id: learner.unwrap_or_default().to_string(),
            course_id: course.unwrap_or_default().to_string(),
            organization_id: organization_id.to_string(),
            status: Status::Completed,
            created_at: created.unwrap_or(0),
            completed_at: Some(ts),
        };
        Ok(out)
    }
}
