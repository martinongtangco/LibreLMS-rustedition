//! The real-engine `ManagementRepo` (spec 006, Module Persistence Seam):
//! tiberius against the 001 FOUNDED tables (`dbo.users`,
//! `dbo.organizations`, `dbo.user_organizations`, `dbo.sessions` — 0001).
//! The manifest is EMPTY BY CONSTRUCTION (FR-013): this slice writes only to
//! foundation-owned DDL.
//!
//! Wire discipline (003/004 parity — the 001 tables are UNIQUEIDENTIFIER):
//! every id crosses the wire as NVARCHAR(40); `CAST(x AS UNIQUEIDENTIFIER)`
//! at the write edge, `CAST(col AS NVARCHAR(40))` at the read edge — and an
//! APP-LAYER shape check first (a non-UUID-shaped user id is by definition
//! not a known user → Scope, CR-3; never a conversion-error 500; failure
//! mode #8).
//!
//! Duplicate-email (the plan's engine arbiter, T013(a)): `dbo.users.email`
//! is UNIQUE. 2601 **and** 2627 (BOTH codes observed on real 2022) map to
//! `DuplicateEmail` on BOTH error legs — tiberius 0.13 can resolve the
//! `query()` future OK even when the server rejects the batch, with the
//! error surfacing during stream consumption (`into_row`); `execute()` has
//! its own error leg. The double's identity-upsert semantics (same
//! (email, name) → idempotent join; name conflict → 409) are mirrored: the
//! sequential path checks the discriminator before any write; the concurrent
//! loser hits the UNIQUE 2601/2627 and maps to DuplicateEmail (the stable
//! 409 code — a name-conflict loser is indistinguishable from a
//! same-identity loser at the engine).
//!
//! set_role/remove parity (the T002 cross-org battery): UPDATE/DELETE-first
//! → 0-row → Scope (a known user without the membership and an unknown user
//! id are BOTH Scope — never NotFound; CR-3: the 404 row is org-ids-only,
//! FR-010). NO bare INSERT leg: an upsert-INSERT would create a membership
//! for a known user who isn't in the org, diverging from the double's
//! approved Scope semantics; the 2601/2627 idempotent-success maps to the
//! create_user membership leg instead (the concurrent create_user race —
//! plan NOTE-2: the UPDATE leg owns the value).
//!
//! Guard-scoping (the 003 CR-9 standing instruction): every write executes
//! inside a scoped guard block, dropped BEFORE any sibling call — the
//! client mutex is non-reentrant (001/003/004/005 pattern: no pool layer,
//! Principle II; the race test uses N INDEPENDENT instances).

use std::sync::Arc;

use async_trait::async_trait;
use contracts::{Role, User};
use futures::StreamExt;
use tiberius::{AuthMethod, Client, Config, EncryptionLevel, ToSql};
use tokio::net::TcpStream;
use tokio::sync::Mutex;
use tokio_util::compat::TokioAsyncWriteCompatExt;

use crate::repo::{ManagedUser, ManagementError, ManagementRepo, Org};

// The URL parse is the 001/003/004/005 one (001 CR-B fix: db name clean,
// encrypt disable|off|false → OFF).
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

/// The duplicate-key (MSSQL 2601 general / 2627 with index name) maps to
/// `DuplicateEmail` (never a 500) — the create paths' engine anchor; every
/// other server error is an outage (503).
fn map_duplicate(e: tiberius::error::Error) -> ManagementError {
    if matches!(e.code(), Some(2601) | Some(2627)) {
        ManagementError::DuplicateEmail
    } else {
        ManagementError::Unavailable
    }
}

/// The 001 CR-F placeholder-hash convention (real argon2id lands with the
/// login slice; never a production credential).
const PLACEHOLDER_HASH: &str = "password-hash-placeholder";

/// App-layer shape check (the belt before the engine CAST — failure mode
/// #8: a non-UUID-shaped string fails `CAST(x AS UNIQUEIDENTIFIER)`).
fn is_uuid_shaped(s: &str) -> bool {
    let b = s.as_bytes();
    b.len() == 36
        && b.iter()
            .enumerate()
            .all(|(i, c)| (i != 8 && i != 13 && i != 18 && i != 23) || *c == b'-')
        && b.iter()
            .enumerate()
            .filter(|(i, _)| *i != 8 && *i != 13 && *i != 18 && *i != 23)
            .all(|(_, c)| c.is_ascii_hexdigit())
}

pub struct TiberiusManagementRepo {
    client: Arc<Mutex<Option<Client<tokio_util::compat::Compat<TcpStream>>>>>,
}

impl TiberiusManagementRepo {
    pub async fn connect(url: &str) -> Result<Self, ManagementError> {
        let (user, port, host, db, pass, encrypt) =
            parse_database_url(url).map_err(|e| ManagementError::Validation {
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
            .map_err(|_| ManagementError::Unavailable)?;
        tcp.set_nodelay(true)
            .map_err(|_| ManagementError::Unavailable)?;
        let client = Client::connect(config, tcp.compat_write())
            .await
            .map_err(|_| ManagementError::Unavailable)?;
        Ok(Self {
            client: Arc::new(Mutex::new(Some(client))),
        })
    }

    /// Production connection proof (mirrors 001/003/004/005).
    pub async fn verify(&self) -> Result<(), ManagementError> {
        let mut g = self.client.lock().await;
        let g = g.as_mut().ok_or(ManagementError::Unavailable)?;
        let n: i32 = g
            .query("SELECT 1", &[])
            .await
            .map_err(|_| ManagementError::Unavailable)?
            .into_row()
            .await
            .map_err(|_| ManagementError::Unavailable)?
            .and_then(|r| r.get::<i32, _>(0))
            .unwrap_or(0);
        if n == 1 {
            Ok(())
        } else {
            Err(ManagementError::Unavailable)
        }
    }
}

#[async_trait]
impl ManagementRepo for TiberiusManagementRepo {
    async fn create_user(
        &self,
        org: &str,
        email: &str,
        name: &str,
        role: &Role,
    ) -> Result<ManagedUser, ManagementError> {
        // FR-012 app-layer validation FIRST (the engine CHECK/UNIQUE is the belt).
        if email.is_empty() || email.len() > 320 {
            return Err(ManagementError::Validation {
                field: "email",
                message: "email must be non-empty and ≤ 320 chars".into(),
            });
        }
        if name.is_empty() || name.len() > 200 {
            return Err(ManagementError::Validation {
                field: "name",
                message: "name must be non-empty and ≤ 200 chars".into(),
            });
        }
        let role_sql = match role {
            Role::Admin => "admin",
            Role::Member => "member",
        }
        .to_string();
        let email_s = email.to_string();
        let name_s = name.to_string();
        let org_s = org.to_string();

        // Step 1 — the identity read (the sequential path of the
        // identity-upsert semantics: same (email, name) → idempotent join;
        // name conflict on the same email → DuplicateEmail).
        let existing = {
            let mut g = self.client.lock().await;
            let g = g.as_mut().ok_or(ManagementError::Unavailable)?;
            let stream = g
                .query(
                    "SELECT TOP 1 CAST(id AS NVARCHAR(40)) AS id, name FROM dbo.users
                     WHERE email = @P1",
                    &[&email_s as &dyn ToSql],
                )
                .await
                .map_err(map_duplicate)?;
            let row = stream.into_row().await.map_err(map_duplicate)?;
            row.map(|r| {
                let id: Option<&str> = r.get(0);
                let name: Option<&str> = r.get(1);
                (
                    id.map(str::to_string).unwrap_or_default(),
                    name.map(str::to_string).unwrap_or_default(),
                )
            })
        };
        let user_id = match existing {
            Some((id, existing_name)) => {
                if existing_name != name_s {
                    // The name-conflict 409 (the double's T002 parity).
                    return Err(ManagementError::DuplicateEmail);
                }
                id
            }
            None => {
                // Step 2 — the fresh-user create: one atomic batch — the
                // NEWID-string id (the wire discipline; CAST at the write
                // edge), the 001 CR-F placeholder hash, then the membership
                // INSERT (the (user_id, org) PK). A 2601/2627 here (the
                // email UNIQUE or the membership PK — the concurrent-create
                // loser, T013(a)) maps to DuplicateEmail on both legs.
                {
                    let mut g = self.client.lock().await;
                    let g = g.as_mut().ok_or(ManagementError::Unavailable)?;
                    let stream = g
                        .query(
                            "DECLARE @id NVARCHAR(40) = CAST(NEWID() AS NVARCHAR(40));
                             INSERT INTO dbo.users (id, email, name, password_hash, theme, created_at)
                             VALUES (CAST(@id AS UNIQUEIDENTIFIER), @P1, @P2, N'"
                            .to_string()
                            + PLACEHOLDER_HASH
                            + "', N'default', SYSDATETIME());
                             INSERT INTO dbo.user_organizations (user_id, organization_id, role)
                             VALUES (CAST(@id AS UNIQUEIDENTIFIER), CAST(@P3 AS UNIQUEIDENTIFIER), @P4);",
                            &[
                                &email_s as &dyn ToSql,
                                &name_s as &dyn ToSql,
                                &org_s as &dyn ToSql,
                                &role_sql as &dyn ToSql,
                            ],
                        )
                        .await
                        .map_err(map_duplicate)?;
                    // The second error leg: the server-rejected batch
                    // surfaces on stream consumption — mapped.
                    stream.into_row().await.map_err(map_duplicate)?;
                }
                // Re-read the created row (the id crosses the wire as a
                // NEWID string — the 003 CR-8 discipline: the read is a
                // separate, plain statement).
                let r = {
                    let mut g = self.client.lock().await;
                    let g = g.as_mut().ok_or(ManagementError::Unavailable)?;
                    let stream = g
                        .query(
                            "SELECT TOP 1 CAST(id AS NVARCHAR(40)) AS id FROM dbo.users WHERE email = @P1",
                            &[&email_s as &dyn ToSql],
                        )
                        .await
                        .map_err(map_duplicate)?;
                    let row = stream.into_row().await.map_err(map_duplicate)?;
                    row.and_then(|r| {
                        let id: Option<&str> = r.get(0);
                        id.map(str::to_string)
                    })
                };
                r.ok_or(ManagementError::Unavailable)?
            }
        };
        // Step 3 — the idempotent-join membership upsert (plan NOTE-2):
        // UPDATE first; a 0-row (the concurrent-create race — the other
        // writer's membership may not be visible yet) → the INSERT leg,
        // whose 2601/2627 (the concurrent create_user already committed it)
        // maps to IDEMPOTENT SUCCESS (the UPDATE leg owns the value — no
        // 409, no 500). Scoping the mapping deliberately: the blanket
        // map_duplicate would mis-map this leg to DuplicateEmail.
        let n = {
            let mut g = self.client.lock().await;
            let g = g.as_mut().ok_or(ManagementError::Unavailable)?;
            g.execute(
                "UPDATE dbo.user_organizations SET role = @P3
                 WHERE user_id = CAST(@P1 AS UNIQUEIDENTIFIER)
                   AND organization_id = CAST(@P2 AS UNIQUEIDENTIFIER)",
                &[
                    &user_id as &dyn ToSql,
                    &org_s as &dyn ToSql,
                    &role_sql as &dyn ToSql,
                ],
            )
            .await
            .map_err(map_duplicate)?
            .rows_affected()
            .iter()
            .sum::<u64>()
        };
        if n == 0 {
            let mut g = self.client.lock().await;
            let g = g.as_mut().ok_or(ManagementError::Unavailable)?;
            match g
                .execute(
                    "INSERT INTO dbo.user_organizations (user_id, organization_id, role)
                     VALUES (CAST(@P1 AS UNIQUEIDENTIFIER), CAST(@P2 AS UNIQUEIDENTIFIER), @P3)",
                    &[
                        &user_id as &dyn ToSql,
                        &org_s as &dyn ToSql,
                        &role_sql as &dyn ToSql,
                    ],
                )
                .await
            {
                Ok(_) => {}
                Err(e) => {
                    if !matches!(e.code(), Some(2601) | Some(2627)) {
                        return Err(map_duplicate(e));
                    }
                }
            }
        }
        Ok(ManagedUser {
            user: User {
                id: user_id,
                email: email_s,
                name: name_s,
                theme: "default".into(),
            },
            role: *role,
        })
    }

    async fn list_users(&self, org: &str) -> Result<Vec<ManagedUser>, ManagementError> {
        let org_s = org.to_string();
        let rows = {
            let mut g = self.client.lock().await;
            let g = g.as_mut().ok_or(ManagementError::Unavailable)?;
            let mut stream = g
                .query(
                    "SELECT CAST(u.id AS NVARCHAR(40)) AS uid, u.email, u.name, u.theme, mo.role
                     FROM dbo.user_organizations mo
                     JOIN dbo.users u ON u.id = mo.user_id
                     WHERE mo.organization_id = CAST(@P1 AS UNIQUEIDENTIFIER)
                     ORDER BY u.id",
                    &[&org_s as &dyn ToSql],
                )
                .await
                .map_err(map_duplicate)?;
            let mut out = Vec::new();
            while let Some(item) = stream.next().await {
                let row = match item {
                    Ok(tiberius::QueryItem::Row(r)) => r,
                    Ok(_) => continue,
                    Err(e) => return Err(map_duplicate(e)),
                };
                let uid: Option<&str> = row.get(0);
                let email: Option<&str> = row.get(1);
                let name: Option<&str> = row.get(2);
                let theme: Option<&str> = row.get(3);
                let role: Option<&str> = row.get(4);
                out.push((
                    uid.map(str::to_string).unwrap_or_default(),
                    email.map(str::to_string).unwrap_or_default(),
                    name.map(str::to_string).unwrap_or_default(),
                    theme.map(str::to_string).unwrap_or_default(),
                    role.map(str::to_string).unwrap_or_default(),
                ));
            }
            out
        };
        Ok(rows
            .into_iter()
            .map(|(id, email, name, theme, role)| ManagedUser {
                user: User {
                    id,
                    email,
                    name,
                    theme,
                },
                role: match role.as_str() {
                    "admin" => Role::Admin,
                    _ => Role::Member,
                },
            })
            .collect())
    }

    async fn set_role(&self, org: &str, user_id: &str, role: &Role) -> Result<(), ManagementError> {
        // The app-layer shape check (failure mode #8 belt — a
        // non-UUID-shaped user id is by definition not a known user →
        // Scope, CR-3; never a conversion-error 500).
        if !is_uuid_shaped(user_id) {
            return Err(ManagementError::Scope);
        }
        let role_sql = match role {
            Role::Admin => "admin",
            Role::Member => "member",
        }
        .to_string();
        let uid = user_id.to_string();
        let oid = org.to_string();
        // UPDATE-first → 0-row → Scope (the T002 cross-org battery parity;
        // a known user without the membership and an unknown user id are
        // BOTH Scope — never NotFound; the 404 row is org-ids-only, FR-010).
        // NO bare INSERT leg (module doc — the plan's upsert 2601/2627
        // idempotent-success lives on the create_user membership leg).
        let n = {
            let mut g = self.client.lock().await;
            let g = g.as_mut().ok_or(ManagementError::Unavailable)?;
            g.execute(
                "UPDATE dbo.user_organizations SET role = @P3
                 WHERE user_id = CAST(@P1 AS UNIQUEIDENTIFIER)
                   AND organization_id = CAST(@P2 AS UNIQUEIDENTIFIER)",
                &[
                    &uid as &dyn ToSql,
                    &oid as &dyn ToSql,
                    &role_sql as &dyn ToSql,
                ],
            )
            .await
            .map_err(map_duplicate)?
            .rows_affected()
            .iter()
            .sum::<u64>()
        };
        if n == 1 {
            Ok(())
        } else {
            Err(ManagementError::Scope)
        }
    }

    async fn remove_membership(&self, org: &str, user_id: &str) -> Result<(), ManagementError> {
        if !is_uuid_shaped(user_id) {
            return Err(ManagementError::Scope);
        }
        let uid = user_id.to_string();
        let oid = org.to_string();
        // FR-004: the membership row only — the user row survives.
        let n = {
            let mut g = self.client.lock().await;
            let g = g.as_mut().ok_or(ManagementError::Unavailable)?;
            g.execute(
                "DELETE FROM dbo.user_organizations
                 WHERE user_id = CAST(@P1 AS UNIQUEIDENTIFIER)
                   AND organization_id = CAST(@P2 AS UNIQUEIDENTIFIER)",
                &[&uid as &dyn ToSql, &oid as &dyn ToSql],
            )
            .await
            .map_err(map_duplicate)?
            .rows_affected()
            .iter()
            .sum::<u64>()
        };
        if n == 1 {
            Ok(())
        } else {
            Err(ManagementError::Scope)
        }
    }

    async fn create_org(&self, name: &str) -> Result<Org, ManagementError> {
        if name.is_empty() || name.len() > 200 {
            return Err(ManagementError::Validation {
                field: "name",
                message: "org name must be non-empty and ≤ 200 chars".into(),
            });
        }
        let name_s = name.to_string();
        let id = {
            let mut g = self.client.lock().await;
            let g = g.as_mut().ok_or(ManagementError::Unavailable)?;
            let stream = g
                .query(
                    "DECLARE @id NVARCHAR(40) = CAST(NEWID() AS NVARCHAR(40));
                     INSERT INTO dbo.organizations (id, name) VALUES (CAST(@id AS UNIQUEIDENTIFIER), @P1);
                     SELECT @id;",
                    &[&name_s as &dyn ToSql],
                )
                .await
                .map_err(map_duplicate)?;
            let row = stream.into_row().await.map_err(map_duplicate)?;
            row.and_then(|r| {
                let id: Option<&str> = r.get(0);
                id.map(str::to_string)
            })
        }
        .ok_or(ManagementError::Unavailable)?;
        Ok(Org { id, name: name_s })
    }

    async fn list_orgs(&self) -> Result<Vec<(Org, u32)>, ManagementError> {
        let rows = {
            let mut g = self.client.lock().await;
            let g = g.as_mut().ok_or(ManagementError::Unavailable)?;
            let mut stream = g
                .query(
                    "SELECT CAST(o.id AS NVARCHAR(40)) AS oid, o.name, COUNT(mo.user_id)
                     FROM dbo.organizations o
                     LEFT JOIN dbo.user_organizations mo ON mo.organization_id = o.id
                     GROUP BY o.id, o.name
                     ORDER BY o.name",
                    &[],
                )
                .await
                .map_err(map_duplicate)?;
            let mut out = Vec::new();
            while let Some(item) = stream.next().await {
                let row = match item {
                    Ok(tiberius::QueryItem::Row(r)) => r,
                    Ok(_) => continue,
                    Err(e) => return Err(map_duplicate(e)),
                };
                let oid: Option<&str> = row.get(0);
                let name: Option<&str> = row.get(1);
                let count: Option<i32> = row.get(2);
                out.push((
                    oid.map(str::to_string).unwrap_or_default(),
                    name.map(str::to_string).unwrap_or_default(),
                    count.unwrap_or(0),
                ));
            }
            out
        };
        Ok(rows
            .into_iter()
            .map(|(id, name, count)| (Org { id, name }, count as u32))
            .collect())
    }

    async fn rename_org(&self, org_id: &str, name: &str) -> Result<(), ManagementError> {
        if name.is_empty() || name.len() > 200 {
            return Err(ManagementError::Validation {
                field: "name",
                message: "org name must be non-empty and ≤ 200 chars".into(),
            });
        }
        // A non-UUID-shaped org id can't match a row → NotFound (the 404
        // row IS org-ids-only — this is that row).
        if !is_uuid_shaped(org_id) {
            return Err(ManagementError::NotFound);
        }
        let oid = org_id.to_string();
        let name_s = name.to_string();
        let n = {
            let mut g = self.client.lock().await;
            let g = g.as_mut().ok_or(ManagementError::Unavailable)?;
            g.execute(
                "UPDATE dbo.organizations SET name = @P2 WHERE id = CAST(@P1 AS UNIQUEIDENTIFIER)",
                &[&oid as &dyn ToSql, &name_s as &dyn ToSql],
            )
            .await
            .map_err(map_duplicate)?
            .rows_affected()
            .iter()
            .sum::<u64>()
        };
        if n == 1 {
            Ok(())
        } else {
            Err(ManagementError::NotFound)
        }
    }

    async fn delete_org(&self, org_id: &str) -> Result<(), ManagementError> {
        if !is_uuid_shaped(org_id) {
            return Err(ManagementError::NotFound);
        }
        let oid = org_id.to_string();
        // 1 — existence (the 404 row, org-ids-only).
        let exists = {
            let mut g = self.client.lock().await;
            let g = g.as_mut().ok_or(ManagementError::Unavailable)?;
            let stream = g
                .query(
                    "SELECT 1 FROM dbo.organizations WHERE id = CAST(@P1 AS UNIQUEIDENTIFIER)",
                    &[&oid as &dyn ToSql],
                )
                .await
                .map_err(map_duplicate)?;
            stream.into_row().await.map_err(map_duplicate)?.is_some()
        };
        if !exists {
            return Err(ManagementError::NotFound);
        }
        // 2 — the 409 not-empty guard on MEMBERSHIPS (FR-009; no silent
        // cascade of membership rows).
        let members = {
            let mut g = self.client.lock().await;
            let g = g.as_mut().ok_or(ManagementError::Unavailable)?;
            let stream = g
                .query(
                    "SELECT COUNT(*) FROM dbo.user_organizations WHERE organization_id = CAST(@P1 AS UNIQUEIDENTIFIER)",
                    &[&oid as &dyn ToSql],
                )
                .await
                .map_err(map_duplicate)?;
            let row = stream.into_row().await.map_err(map_duplicate)?;
            row.map(|r| {
                let count: Option<i32> = r.get(0);
                count.unwrap_or(0)
            })
            .unwrap_or(0)
        };
        if members > 0 {
            return Err(ManagementError::NotEmpty);
        }
        // 3 — CR-2b: the session cascade, ordered BEFORE the org row delete
        // (the sessions FK to organizations requires it; other orgs'
        // sessions are untouched by the WHERE clause; the real-engine proof
        // is T013(c)).
        {
            let mut g = self.client.lock().await;
            let g = g.as_mut().ok_or(ManagementError::Unavailable)?;
            g.execute(
                "DELETE FROM dbo.sessions WHERE organization_id = CAST(@P1 AS UNIQUEIDENTIFIER)",
                &[&oid as &dyn ToSql],
            )
            .await
            .map_err(map_duplicate)?;
        }
        // 4 — the org row.
        let n = {
            let mut g = self.client.lock().await;
            let g = g.as_mut().ok_or(ManagementError::Unavailable)?;
            g.execute(
                "DELETE FROM dbo.organizations WHERE id = CAST(@P1 AS UNIQUEIDENTIFIER)",
                &[&oid as &dyn ToSql],
            )
            .await
            .map_err(map_duplicate)?
            .rows_affected()
            .iter()
            .sum::<u64>()
        };
        if n == 1 {
            Ok(())
        } else {
            Err(ManagementError::NotFound)
        }
    }
}
