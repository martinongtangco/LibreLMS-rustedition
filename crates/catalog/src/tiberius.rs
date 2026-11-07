//! The real-engine `CourseRepo` (spec 003, Module Persistence Seam): tiberius
//! against `dbo.courses` (0003_catalog.sql). Parameterized queries throughout;
//! the double's semantics (scope/404-oracle/validation) are the contract.

use std::sync::Arc;

use async_trait::async_trait;
use tiberius::{AuthMethod, Client, Config, EncryptionLevel};
use tokio::net::TcpStream;
use tokio::sync::Mutex;
use tokio_util::compat::TokioAsyncWriteCompatExt;

use contracts::Course;
use futures::StreamExt;

use crate::repo::{CatalogError, CourseRepo};

/// The whole-script-batch URL parse is the 001 mssql one (db name clean,
/// encrypt disable|off|false → OFF).
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

/// Unix seconds → `YYYY-MM-DD HH:MM:SS` UTC (001 convention).
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

/// A single client behind a mutex (001 pattern — no pool layer, Principle II).
pub struct TiberiusCourseRepo {
    client: Arc<Mutex<Option<Client<tokio_util::compat::Compat<TcpStream>>>>>,
}

impl TiberiusCourseRepo {
    pub async fn connect(url: &str) -> Result<Self, CatalogError> {
        let (user, port, host, db, pass, encrypt) =
            parse_database_url(url).map_err(|e| CatalogError::Validation {
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
            .map_err(|_| CatalogError::Unavailable)?;
        tcp.set_nodelay(true)
            .map_err(|_| CatalogError::Unavailable)?;
        let client = Client::connect(config, tcp.compat_write())
            .await
            .map_err(|_| CatalogError::Unavailable)?;
        Ok(Self {
            client: Arc::new(Mutex::new(Some(client))),
        })
    }

    async fn guard(
        &self,
    ) -> Result<
        tokio::sync::MutexGuard<'_, Option<Client<tokio_util::compat::Compat<TcpStream>>>>,
        CatalogError,
    > {
        let g = self.client.lock().await;
        if g.is_none() {
            return Err(CatalogError::Unavailable);
        }
        Ok(g)
    }
}

/// The scope/404-oracle rule (spec 003 story 3.2) implemented against the real
/// engine: `WHERE id = @id AND organization_id = @org` yields zero rows for
/// both absent and foreign — the discriminator is a second existence check.
async fn fetch(
    g: &mut tokio::sync::MutexGuard<'_, Option<Client<tokio_util::compat::Compat<TcpStream>>>>,
    id: &str,
    org: &str,
) -> Result<Option<Course>, CatalogError> {
    let g = g.as_mut().ok_or(CatalogError::Unavailable)?;
    let row = g
        .query(
            "SELECT TOP 1 id, title, description, organization_id, created_at, updated_at
             FROM dbo.courses WHERE id = @P1",
            &[&id],
        )
        .await
        .map_err(|_| CatalogError::Unavailable)?
        .into_row()
        .await
        .map_err(|_| CatalogError::Unavailable)?;
    let Some(row) = row else {
        return Ok(None); // genuinely absent → NotFound (404)
    };
    let cid: Option<&str> = row.get(0);
    let title: Option<&str> = row.get(1);
    let desc: Option<&str> = row.get(2);
    let org_id: Option<&str> = row.get(3);
    let _created: Option<&str> = row.get(4);
    let _updated: Option<&str> = row.get(5);
    let org_id = org_id.unwrap_or("");
    if org_id != org {
        // Exists but foreign → Scope (403), not NotFound (no oracle).
        return Err(CatalogError::Scope);
    }
    Ok(Some(Course {
        id: cid.unwrap_or(id).to_string(),
        title: title.unwrap_or("").to_string(),
        description: desc.unwrap_or("").to_string(),
        organization_id: org_id.to_string(),
    }))
}

#[async_trait]
impl CourseRepo for TiberiusCourseRepo {
    async fn list_for(
        &self,
        org: &str,
        title_filter: Option<&str>,
    ) -> Result<Vec<Course>, CatalogError> {
        let mut g = self.guard().await?;
        let g = g.as_mut().ok_or(CatalogError::Unavailable)?;
        let stream = g
            .query(
                "SELECT id, title, description, organization_id
                 FROM dbo.courses
                 WHERE organization_id = @P1
                   AND (@P2 IS NULL OR title LIKE '%' + @P2 + '%')",
                &[&org, &title_filter.map(str::to_string)],
            )
            .await
            .map_err(|_| CatalogError::Unavailable)?;
        let mut out = Vec::new();
        let mut rows = stream.into_row_stream();
        while let Some(row) = rows.next().await {
            let row = row.map_err(|_| CatalogError::Unavailable)?;
            let cid: Option<&str> = row.get(0);
            let title: Option<&str> = row.get(1);
            let desc: Option<&str> = row.get(2);
            out.push(Course {
                id: cid.unwrap_or("").to_string(),
                title: title.unwrap_or("").to_string(),
                description: desc.unwrap_or("").to_string(),
                organization_id: org.to_string(),
            });
        }
        Ok(out)
    }

    async fn get(&self, course_id: &str, session_org: &str) -> Result<Course, CatalogError> {
        let mut g = self.guard().await?;
        match fetch(&mut g, course_id, session_org).await? {
            Some(c) => Ok(c),
            None => Err(CatalogError::NotFound),
        }
    }

    async fn find_by_id(&self, course_id: &str) -> Result<Option<Course>, CatalogError> {
        let mut g = self.guard().await?;
        let g = g.as_mut().ok_or(CatalogError::Unavailable)?;
        let row = g
            .query(
                "SELECT TOP 1 id, title, description, organization_id
                 FROM dbo.courses WHERE id = @P1",
                &[&course_id],
            )
            .await
            .map_err(|_| CatalogError::Unavailable)?
            .into_row()
            .await
            .map_err(|_| CatalogError::Unavailable)?;
        let Some(row) = row else {
            return Ok(None);
        };
        let cid: Option<&str> = row.get(0);
        let title: Option<&str> = row.get(1);
        let desc: Option<&str> = row.get(2);
        let org: Option<&str> = row.get(3);
        Ok(Some(Course {
            id: cid.unwrap_or(course_id).to_string(),
            title: title.unwrap_or("").to_string(),
            description: desc.unwrap_or("").to_string(),
            organization_id: org.unwrap_or("").to_string(),
        }))
    }

    async fn create(
        &self,
        session_org: &str,
        title: &str,
        description: Option<&str>,
    ) -> Result<Course, CatalogError> {
        use crate::repo::{validate_description, validate_title};
        validate_title(title)?;
        if let Some(d) = description {
            validate_description(d)?;
        }
        let mut g = self.guard().await?;
        let g = g.as_mut().ok_or(CatalogError::Unavailable)?;
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);
        let now_sql = unix_ts_to_sql(now);
        // One round-trip: NEWID() id returned via OUTPUT (contract ids are
        // strings; CAST at the wire edge — 001 convention).
        let row = g
            .query(
                "DECLARE @id NVARCHAR(40) = CAST(NEWID() AS NVARCHAR(40));
                 INSERT INTO dbo.courses (id, organization_id, title, description, created_at, updated_at)
                 OUTPUT inserted.id
                 VALUES (@P1, @P2, @P3, @P4, CAST(@P5 AS DATETIME2), CAST(@P5 AS DATETIME2))",
                &[&session_org, &title, &description, &now_sql, &now_sql],
            )
            .await
            .map_err(|_| CatalogError::Unavailable)?
            .into_row()
            .await
            .map_err(|_| CatalogError::Unavailable)?;
        let Some(row) = row else {
            return Err(CatalogError::Unavailable);
        };
        let cid: Option<&str> = row.get(0);
        Ok(Course {
            id: cid.unwrap_or("").to_string(),
            title: title.to_string(),
            description: description.unwrap_or_default().to_string(),
            organization_id: session_org.to_string(),
        })
    }

    async fn update(
        &self,
        course_id: &str,
        session_org: &str,
        title: Option<&str>,
        description: Option<&str>,
    ) -> Result<Course, CatalogError> {
        // Existence + scope first (404 vs 403 semantics).
        let _ = self.get(course_id, session_org).await?;
        let mut g = self.guard().await?;
        let g = g.as_mut().ok_or(CatalogError::Unavailable)?;
        let now_sql = unix_ts_to_sql(
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs() as i64)
                .unwrap_or(0),
        );
        // COALESCE keeps the old value when the field is absent from the update.
        let affected = g
            .execute(
                "UPDATE dbo.courses
                 SET title = COALESCE(@P1, title),
                     description = COALESCE(@P2, description),
                     updated_at = CAST(@P3 AS DATETIME2)
                 WHERE id = @P4 AND organization_id = @P5",
                &[&title, &description, &now_sql, &course_id, &session_org],
            )
            .await
            .map_err(|_| CatalogError::Unavailable)?;
        if affected.rows_affected().iter().sum::<u64>() == 0 {
            return Err(CatalogError::NotFound);
        }
        self.get(course_id, session_org).await
    }

    async fn delete(&self, course_id: &str, session_org: &str) -> Result<(), CatalogError> {
        let _ = self.get(course_id, session_org).await?;
        let mut g = self.guard().await?;
        let g = g.as_mut().ok_or(CatalogError::Unavailable)?;
        let affected = g
            .execute(
                "DELETE FROM dbo.courses WHERE id = @P1 AND organization_id = @P2",
                &[&course_id, &session_org],
            )
            .await
            .map_err(|_| CatalogError::Unavailable)?;
        if affected.rows_affected().iter().sum::<u64>() == 0 {
            return Err(CatalogError::NotFound);
        }
        Ok(())
    }
}

/// `verify` — the production connection proof (mirrors 001 TiberiusStorage).
impl TiberiusCourseRepo {
    pub async fn verify(&self) -> Result<(), CatalogError> {
        let mut g = self.guard().await?;
        if g.is_none() {
            return Err(CatalogError::Unavailable);
        }
        let n: i32 = g
            .as_mut()
            .unwrap()
            .query("SELECT 1", &[])
            .await
            .map_err(|_| CatalogError::Unavailable)?
            .into_row()
            .await
            .map_err(|_| CatalogError::Unavailable)?
            .and_then(|r| r.get::<i32, _>(0))
            .unwrap_or(0);
        if n == 1 {
            Ok(())
        } else {
            Err(CatalogError::Unavailable)
        }
    }
}
