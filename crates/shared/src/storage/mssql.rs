//! tiberius (TDS) `Storage` implementation — the full-environment system of
//! record (ADR 0003, Principle VI).
//!
//! Parameterized queries throughout (`@Pn` placeholders, `ToSql` values —
//! `&str` → nvarchar, `i64` → bigint); identifiers are internal constants.
//! Timestamps cross the wire as `YYYY-MM-DD HH:MM:SS` (UTC) strings and are
//! `CAST` to/from `DATETIME2` on the server side; ids cross as NVARCHAR and are
//! `CAST` to `UNIQUEIDENTIFIER` where needed — this keeps the contract shapes
//! (String ids, i64 unix seconds) implementation-agnostic (data-model.md).

use async_trait::async_trait;
use tiberius::{AuthMethod, Client, Config, EncryptionLevel};
use tokio::net::TcpStream;
use tokio_util::compat::TokioAsyncWriteCompatExt;

use super::{NewSession, SchemaStatus, SessionRow, Storage, StorageError};
use crate::migrate::checksum;
use contracts::{Role, User};

/// A single client behind a mutex (the modular monolith needs one connection at
/// this scale — no pool layer until a specific, current problem requires it,
/// Principle II).
#[derive(Clone)]
pub struct TiberiusStorage {
    client:
        std::sync::Arc<tokio::sync::Mutex<Option<Client<tokio_util::compat::Compat<TcpStream>>>>>,
}

/// Parse `mssql://user:pass@host:port/db?encrypt=disable|true` (contracts/api.md).
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
    let (hostport, query) = match hostpart.split_once('?') {
        Some((hp, q)) => (hp, q),
        None => (hostpart, ""),
    };
    let hostport = hostport.split('/').next().unwrap_or(hostport);
    let (host, port) = match hostport.rsplit_once(':') {
        Some((h, p)) => (
            h.to_string(),
            p.parse::<u16>()
                .map_err(|_| format!("DATABASE_URL bad port in {hostport}"))?,
        ),
        None => (hostport.to_string(), 1433),
    };
    let db = hostpart
        .strip_prefix(hostport)
        .map(|rest| rest.trim_start_matches('/'))
        .unwrap_or("")
        .to_string();
    if db.is_empty() {
        return Err("DATABASE_URL missing database name".to_string());
    }
    let encrypt = !matches!(query.split_whitespace().next(), Some("encrypt=false"));
    Ok((user.to_string(), port, host, db, pass.to_string(), encrypt))
}

/// Unix seconds → `YYYY-MM-DD HH:MM:SS` UTC (civil calendar, no chrono needed).
fn unix_ts_to_sql(secs: i64) -> String {
    let days = secs.div_euclid(86_400);
    let rem = secs.rem_euclid(86_400);
    let (h, m, s) = (rem / 3600, (rem % 3600) / 60, rem % 60);
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (yoe * 365 + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let mo = mp + if mp < 10 { 3 } else { -9 };
    let y = if mo <= 2 { y + 1 } else { y };
    format!("{y:04}-{mo:02}-{d:02} {h:02}:{m:02}:{s:02}")
}

impl TiberiusStorage {
    pub async fn connect(url: &str) -> Result<Self, StorageError> {
        let (user, port, host, db, pass, encrypt) =
            parse_database_url(url).map_err(StorageError::Unavailable)?;
        let mut config = Config::new();
        config.database(&db);
        config.authentication(AuthMethod::sql_server(user, pass));
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
            .map_err(|e| StorageError::Unavailable(format!("mssql tcp: {e}")))?;
        tcp.set_nodelay(true)
            .map_err(|e| StorageError::Unavailable(format!("mssql nodelay: {e}")))?;
        let client = Client::connect(config, tcp.compat_write())
            .await
            .map_err(|e| StorageError::Unavailable(format!("mssql connect: {e}")))?;
        Ok(Self {
            client: std::sync::Arc::new(tokio::sync::Mutex::new(Some(client))),
        })
    }

    async fn guard(
        &self,
    ) -> Result<
        tokio::sync::MutexGuard<'_, Option<Client<tokio_util::compat::Compat<TcpStream>>>>,
        StorageError,
    > {
        let g = self.client.lock().await;
        if g.is_none() {
            return Err(StorageError::Unavailable(
                "client not connected".to_string(),
            ));
        }
        Ok(g)
    }
}

#[async_trait]
impl Storage for TiberiusStorage {
    async fn verify(&self) -> Result<(), StorageError> {
        let mut g = self.guard().await?;
        let client = g.as_mut().unwrap();
        client
            .execute("SELECT 1", &[])
            .await
            .map_err(|e| StorageError::Unavailable(format!("mssql verify: {e}")))?;
        Ok(())
    }

    async fn schema_status(&self) -> Result<SchemaStatus, StorageError> {
        let mut g = self.guard().await?;
        let client = g.as_mut().unwrap();
        let stream = client
            .query(
                "SELECT COUNT(*) FROM INFORMATION_SCHEMA.TABLES WHERE TABLE_NAME = 'users'",
                &[],
            )
            .await
            .map_err(|e| StorageError::Unavailable(format!("mssql schema_status: {e}")))?;
        let Some(row) = stream
            .into_row()
            .await
            .map_err(|e| StorageError::Unavailable(e.to_string()))?
        else {
            return Err(StorageError::Unavailable("no row".to_string()));
        };
        let count: i32 = row
            .get(0)
            .ok_or_else(|| StorageError::Other("count column missing".to_string()))?;
        Ok(if count > 0 {
            SchemaStatus::Ready
        } else {
            SchemaStatus::NotReady
        })
    }

    async fn run_migrations(&self, manifest: &[(&str, &str)]) -> Result<Vec<String>, StorageError> {
        let mut g = self.guard().await?;
        let client = g.as_mut().unwrap();
        client
            .execute(
                "IF OBJECT_ID(N'dbo.schema_migrations', N'U') IS NULL
                 CREATE TABLE dbo.schema_migrations (
                    name NVARCHAR(200) PRIMARY KEY,
                    checksum NVARCHAR(64) NOT NULL,
                    applied_at DATETIME2 NOT NULL)",
                &[],
            )
            .await
            .map_err(|e| StorageError::Unavailable(format!("mssql migrate: {e}")))?;

        let mut applied_now = Vec::new();
        for &(name, sql) in manifest {
            let stream = client
                .query(
                    "SELECT COUNT(*) FROM dbo.schema_migrations WHERE name = @P1",
                    &[&name],
                )
                .await
                .map_err(|e| StorageError::Unavailable(format!("mssql migrate: {e}")))?;
            let row = stream
                .into_row()
                .await
                .map_err(|e| StorageError::Unavailable(e.to_string()))?
                .ok_or_else(|| StorageError::Unavailable("no row".to_string()))?;
            let already: i32 = row.get(0).unwrap_or(0);
            if already > 0 {
                continue; // idempotent (research R1)
            }
            let stmts: Vec<&str> = sql
                .split(';')
                .map(|s| s.trim())
                .filter(|s| !s.is_empty())
                .collect();
            for stmt in stmts {
                client
                    .execute(stmt, &[])
                    .await
                    .map_err(|e| StorageError::Unavailable(format!("mssql migrate {name}: {e}")))?;
            }
            let sum = checksum(sql);
            client
                .execute(
                    "INSERT INTO dbo.schema_migrations (name, checksum, applied_at)
                     VALUES (@P1, @P2, SYSDATETIME())",
                    &[&name, &sum],
                )
                .await
                .map_err(|e| StorageError::Unavailable(format!("mssql migrate: {e}")))?;
            applied_now.push(name.to_string());
        }
        Ok(applied_now)
    }

    async fn create_session(&self, s: &NewSession) -> Result<(), StorageError> {
        let mut g = self.guard().await?;
        let client = g.as_mut().unwrap();
        let created = unix_ts_to_sql(s.created_at);
        let activity = unix_ts_to_sql(s.last_activity);
        let expires = unix_ts_to_sql(s.expires_at);
        client
            .execute(
                "INSERT INTO dbo.sessions (token_hash, user_id, organization_id,
                  created_at, last_activity, expires_at)
                 VALUES (@P1, CAST(@P2 AS UNIQUEIDENTIFIER), CAST(@P3 AS UNIQUEIDENTIFIER),
                  CAST(@P4 AS DATETIME2), CAST(@P5 AS DATETIME2), CAST(@P6 AS DATETIME2))",
                &[
                    &s.token_hash,
                    &s.user_id,
                    &s.organization_id,
                    &created,
                    &activity,
                    &expires,
                ],
            )
            .await
            .map_err(|e| StorageError::Unavailable(format!("mssql create_session: {e}")))?;
        Ok(())
    }

    async fn get_session(&self, token_hash: &str) -> Result<Option<SessionRow>, StorageError> {
        let mut g = self.guard().await?;
        let client = g.as_mut().unwrap();
        let stream = client
            .query(
                "SELECT token_hash, CAST(user_id AS NVARCHAR(40)),
                       CAST(organization_id AS NVARCHAR(40)),
                       DATEDIFF(SECOND, '1970-01-01', created_at),
                       DATEDIFF(SECOND, '1970-01-01', last_activity),
                       DATEDIFF(SECOND, '1970-01-01', expires_at)
                 FROM dbo.sessions WHERE token_hash = @P1",
                &[&token_hash],
            )
            .await
            .map_err(|e| StorageError::Unavailable(format!("mssql get_session: {e}")))?;
        let Some(row) = stream
            .into_row()
            .await
            .map_err(|e| StorageError::Unavailable(e.to_string()))?
        else {
            return Ok(None);
        };
        let sget = |i: usize| -> Result<String, StorageError> {
            let v: &str = row
                .get(i)
                .ok_or_else(|| StorageError::Other("column missing".to_string()))?;
            Ok(v.to_string())
        };
        let i64 = |v: i32| v as i64;
        Ok(Some(SessionRow {
            token_hash: sget(0)?,
            user_id: sget(1)?,
            organization_id: sget(2)?,
            created_at: i64(row
                .get(3)
                .ok_or_else(|| StorageError::Other("col".into()))?),
            last_activity: i64(row
                .get(4)
                .ok_or_else(|| StorageError::Other("col".into()))?),
            expires_at: i64(row
                .get(5)
                .ok_or_else(|| StorageError::Other("col".into()))?),
        }))
    }

    async fn touch_session(
        &self,
        token_hash: &str,
        now: i64,
        new_expiry: i64,
    ) -> Result<bool, StorageError> {
        let mut g = self.guard().await?;
        let client = g.as_mut().unwrap();
        let activity = unix_ts_to_sql(now);
        let expiry = unix_ts_to_sql(new_expiry);
        let affected = client
            .execute(
                "UPDATE dbo.sessions
                 SET last_activity = CAST(@P1 AS DATETIME2),
                     expires_at = CAST(@P3 AS DATETIME2)
                 WHERE token_hash = @P2",
                &[&activity, &token_hash, &expiry],
            )
            .await
            .map_err(|e| StorageError::Unavailable(format!("mssql touch_session: {e}")))?;
        Ok(affected.rows_affected().iter().sum::<u64>() > 0)
    }

    async fn delete_session(&self, token_hash: &str) -> Result<(), StorageError> {
        let mut g = self.guard().await?;
        let client = g.as_mut().unwrap();
        client
            .execute(
                "DELETE FROM dbo.sessions WHERE token_hash = @P1",
                &[&token_hash],
            )
            .await
            .map_err(|e| StorageError::Unavailable(format!("mssql delete_session: {e}")))?;
        Ok(())
    }

    async fn get_user(&self, id: &str) -> Result<Option<User>, StorageError> {
        let mut g = self.guard().await?;
        let client = g.as_mut().unwrap();
        let stream = client
            .query(
                "SELECT CAST(id AS NVARCHAR(40)), email, name, theme
                 FROM dbo.users WHERE id = CAST(@P1 AS UNIQUEIDENTIFIER)",
                &[&id],
            )
            .await
            .map_err(|e| StorageError::Unavailable(format!("mssql get_user: {e}")))?;
        let Some(row) = stream
            .into_row()
            .await
            .map_err(|e| StorageError::Unavailable(e.to_string()))?
        else {
            return Ok(None);
        };
        let get = |i: usize| -> Result<String, StorageError> {
            let v: &str = row
                .get(i)
                .ok_or_else(|| StorageError::Other("column missing".to_string()))?;
            Ok(v.to_string())
        };
        Ok(Some(User {
            id: get(0)?,
            email: get(1)?,
            name: get(2)?,
            theme: get(3)?,
        }))
    }

    async fn get_user_by_email(&self, email: &str) -> Result<Option<User>, StorageError> {
        let mut g = self.guard().await?;
        let client = g.as_mut().unwrap();
        let stream = client
            .query(
                "SELECT CAST(id AS NVARCHAR(40)), email, name, theme
                 FROM dbo.users WHERE email = @P1",
                &[&email],
            )
            .await
            .map_err(|e| StorageError::Unavailable(format!("mssql get_user_by_email: {e}")))?;
        let Some(row) = stream
            .into_row()
            .await
            .map_err(|e| StorageError::Unavailable(e.to_string()))?
        else {
            return Ok(None);
        };
        let get = |i: usize| -> Result<String, StorageError> {
            let v: &str = row
                .get(i)
                .ok_or_else(|| StorageError::Other("column missing".to_string()))?;
            Ok(v.to_string())
        };
        Ok(Some(User {
            id: get(0)?,
            email: get(1)?,
            name: get(2)?,
            theme: get(3)?,
        }))
    }

    async fn org_role(
        &self,
        user_id: &str,
        organization_id: &str,
    ) -> Result<Option<Role>, StorageError> {
        let mut g = self.guard().await?;
        let client = g.as_mut().unwrap();
        let stream = client
            .query(
                "SELECT role FROM dbo.user_organizations
                 WHERE user_id = CAST(@P1 AS UNIQUEIDENTIFIER)
                   AND organization_id = CAST(@P2 AS UNIQUEIDENTIFIER)",
                &[&user_id, &organization_id],
            )
            .await
            .map_err(|e| StorageError::Unavailable(format!("mssql org_role: {e}")))?;
        let Some(row) = stream
            .into_row()
            .await
            .map_err(|e| StorageError::Unavailable(e.to_string()))?
        else {
            return Ok(None);
        };
        let role: &str = row
            .get(0)
            .ok_or_else(|| StorageError::Other("column missing".to_string()))?;
        let _ = role;
        match role {
            "admin" => Ok(Some(Role::Admin)),
            "member" => Ok(Some(Role::Member)),
            other => Err(StorageError::Other(format!("unknown role {other}"))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn url_parsing_names_bad_inputs() {
        assert!(parse_database_url("postgres://x").is_err());
        assert!(parse_database_url("mssql://user@localhost/db").is_err());
        assert!(parse_database_url("mssql://u:p@h:1/db").unwrap().5);
        assert!(
            !parse_database_url("mssql://u:p@h:1/db?encrypt=false")
                .unwrap()
                .5
        );
        let (user, port, host, db, _pass, _enc) =
            parse_database_url("mssql://sa:pw@db.example:1434/LibreLms").unwrap();
        assert_eq!(
            (user.as_str(), port, host.as_str(), db.as_str()),
            ("sa", 1434, "db.example", "LibreLms")
        );
    }

    #[test]
    fn ts_conversion_roundtrips_known_values() {
        assert_eq!(unix_ts_to_sql(0), "1970-01-01 00:00:00");
        assert_eq!(unix_ts_to_sql(86_400), "1970-01-02 00:00:00");
        assert_eq!(unix_ts_to_sql(1_750_000_000), "2025-06-15 15:06:40");
    }
}
