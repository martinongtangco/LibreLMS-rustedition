//! The management repository seam (in-crate per the 001 Module Persistence
//! Seam) + error → envelope mapping. T002 (red): the US1 battery defines the
//! contract; the stub below fails it on purpose until T004.

use std::sync::Arc;

use contracts::{Role, User};

/// Management failures → envelope mapping (host): 403 / 403 / 404 / 409 /
/// 409 / 400(field) / 503.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ManagementError {
    /// Cross-org user operation, or an unknown user id (spec 006 CR-3:
    /// user-side answers 403 `scope` — NEVER 404; the 404 row is
    /// org-ids-only per FR-010).
    Scope,
    /// The session's role does not allow the action (403).
    Forbidden,
    /// Unknown organization id (404 — org-ids-only, FR-010).
    NotFound,
    /// Duplicate email (409 — the engine's UNIQUE constraint is the final
    /// arbiter under concurrency; 2601/2627 both mapped, never a 500).
    DuplicateEmail,
    /// Deletion of a non-empty organization (409 — no silent cascade of
    /// membership rows; sessions cascade per spec 006 CR-2, option b).
    NotEmpty,
    /// Invalid input; the message names the offending field (400 — FR-012
    /// incl. role ∈ {admin, member}, CR-1: the CHECK is the belt).
    Validation {
        field: &'static str,
        message: String,
    },
    /// The system of record (or session store) is unreachable (503 — never
    /// 500, never a silent empty list).
    Unavailable,
}

impl std::fmt::Display for ManagementError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Scope => write!(f, "out of scope"),
            Self::Forbidden => write!(f, "forbidden"),
            Self::NotFound => write!(f, "not found"),
            Self::DuplicateEmail => write!(f, "duplicate email"),
            Self::NotEmpty => write!(f, "organization not empty"),
            Self::Validation { field, message } => write!(f, "validation: {field}: {message}"),
            Self::Unavailable => write!(f, "storage unavailable"),
        }
    }
}

/// A user as seen from an organization (the user row + the org's role).
#[derive(Debug, Clone, PartialEq)]
pub struct ManagedUser {
    pub user: User,
    pub role: Role,
}

/// An organization (001's `dbo.organizations` is (id, name) only — plan
/// Assumption: flat in this slice).
#[derive(Debug, Clone, PartialEq)]
pub struct Org {
    pub id: String,
    pub name: String,
}

/// The management repository seam. T002 (red): signatures defined by the US1
/// battery; the stub implementation fails on purpose until T004.
#[async_trait::async_trait]
pub trait ManagementRepo: Send + Sync {
    /// US1: create a user in the org with a role (FR-001; the 001
    /// placeholder-hash convention; duplicate email → DuplicateEmail).
    async fn create_user(
        &self,
        org: &str,
        email: &str,
        name: &str,
        role: &Role,
    ) -> Result<ManagedUser, ManagementError>;

    /// US1: list the org's users with roles (FR-003 — other orgs' users
    /// never appear).
    async fn list_users(&self, org: &str) -> Result<Vec<ManagedUser>, ManagementError>;

    /// US1: change the user's role in the org (FR-004; re-role is an upsert —
    /// plan NOTE-2: the INSERT-leg PK violation maps to idempotent success).
    async fn set_role(&self, org: &str, user_id: &str, role: &Role) -> Result<(), ManagementError>;

    /// US1: remove the user from the org (FR-004 — the membership row only;
    /// the user row survives, multi-org users).
    async fn remove_membership(&self, org: &str, user_id: &str) -> Result<(), ManagementError>;

    /// US2: create an organization (FR-006).
    async fn create_org(&self, name: &str) -> Result<Org, ManagementError>;

    /// US2: list organizations with member counts (FR-007).
    async fn list_orgs(&self) -> Result<Vec<(Org, u32)>, ManagementError>;

    /// US2: rename an organization (FR-008).
    async fn rename_org(&self, org_id: &str, name: &str) -> Result<(), ManagementError>;

    /// US2: delete an organization (FR-008/FR-009 — 409 not-empty guard on
    /// memberships; a successful delete cascade-expires the org's sessions,
    /// CR-2 option b, ordered sessions-before-org-row).
    async fn delete_org(&self, org_id: &str) -> Result<(), ManagementError>;
}

/// Test-only fault injection + deterministic id generation (the 003 double
/// pattern).
#[derive(Debug, Default)]
pub struct InMemoryManagementRepo {
    users: std::sync::Mutex<std::collections::HashMap<String, User>>,
    memberships: std::sync::Mutex<std::collections::HashMap<(String, String), Role>>,
    orgs: std::sync::Mutex<std::collections::HashMap<String, Org>>,
    /// Test-only fault injection (the 001 MemorySessionStore pattern).
    down: Arc<std::sync::atomic::AtomicBool>,
}

impl InMemoryManagementRepo {
    pub fn new() -> Self {
        Self::default()
    }

    /// Test-only fault injection (the 003 double pattern).
    pub fn set_unavailable_for_test(&self, down: bool) {
        self.down.store(down, std::sync::atomic::Ordering::SeqCst);
    }

    /// Test-only: ensure the org row exists (the 001
    /// `create_user_for_test` pattern — the session's org has a
    /// membership without an org row; the delete guard is on
    /// memberships, but the org-row existence check is 404).
    pub fn ensure_org_for_test(&self, id: &str, name: &str) {
        self.orgs
            .lock()
            .unwrap()
            .entry(id.to_string())
            .or_insert_with(|| Org {
                id: id.to_string(),
                name: name.to_string(),
            });
    }

    fn is_down(&self) -> bool {
        self.down.load(std::sync::atomic::Ordering::SeqCst)
    }
}

#[async_trait::async_trait]
impl ManagementRepo for InMemoryManagementRepo {
    async fn create_user(
        &self,
        org: &str,
        email: &str,
        name: &str,
        role: &Role,
    ) -> Result<ManagedUser, ManagementError> {
        if self.is_down() {
            return Err(ManagementError::Unavailable);
        }
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
        // Duplicate-email semantics (documented — Principle X): the user row
        // is GLOBAL with a UNIQUE email. Same identity (email AND name) →
        // idempotent: the user row is not re-created; the target-org
        // membership is upserted (the multi-org "join" path). Different name
        // on the same email → DuplicateEmail (identity conflict). The engine
        // belt (T013(a)): CONCURRENT identical creates race the pre-check —
        // both see "absent", both INSERT, the loser's 2601/2627 maps to
        // DuplicateEmail; the engine's UNIQUE constraint is the final
        // arbiter, the logic layer is best-effort.
        let users = self.users.lock().unwrap();
        if let Some(existing) = users.values().find(|u| u.email == email) {
            if existing.name != name {
                return Err(ManagementError::DuplicateEmail);
            }
            let user = existing.clone();
            drop(users);
            self.memberships
                .lock()
                .unwrap()
                .insert((org.to_string(), user.id.clone()), *role);
            return Ok(ManagedUser { user, role: *role });
        }
        let id = format!("mu-{}", users.len() + 1);
        let user = User {
            id: id.clone(),
            email: email.into(),
            name: name.into(),
            theme: "default".into(),
        };
        drop(users);
        self.users.lock().unwrap().insert(id.clone(), user.clone());
        self.memberships
            .lock()
            .unwrap()
            .insert((org.to_string(), id.clone()), *role);
        Ok(ManagedUser { user, role: *role })
    }

    async fn list_users(&self, org: &str) -> Result<Vec<ManagedUser>, ManagementError> {
        if self.is_down() {
            return Err(ManagementError::Unavailable);
        }
        let users = self.users.lock().unwrap();
        let memberships = self.memberships.lock().unwrap();
        let mut out: Vec<ManagedUser> = memberships
            .iter()
            .filter(|((o, _), _)| o == org)
            .filter_map(|((_, uid), role)| {
                users.get(uid).map(|user| ManagedUser {
                    user: user.clone(),
                    role: *role,
                })
            })
            .collect();
        out.sort_by(|a, b| a.user.id.cmp(&b.user.id));
        Ok(out)
    }

    async fn set_role(&self, org: &str, user_id: &str, role: &Role) -> Result<(), ManagementError> {
        if self.is_down() {
            return Err(ManagementError::Unavailable);
        }
        // CR-3: user-side answers Scope (403), never NotFound — a known user
        // without a membership in `org`, and an unknown user id, are
        // indistinguishable. The engine's upsert INSERT-leg 2601/2627
        // (concurrent create_user already wrote the membership) maps to
        // idempotent success (plan NOTE-2); the double is sequential, so the
        // INSERT leg is the not-yet-written case → Scope here.
        let known = self
            .users
            .lock()
            .unwrap()
            .iter()
            .any(|(_, u)| u.id == user_id);
        if !known {
            return Err(ManagementError::Scope);
        }
        let mut memberships = self.memberships.lock().unwrap();
        match memberships.get_mut(&(org.to_string(), user_id.to_string())) {
            Some(r) => {
                *r = *role;
                Ok(())
            }
            None => Err(ManagementError::Scope),
        }
    }

    async fn remove_membership(&self, org: &str, user_id: &str) -> Result<(), ManagementError> {
        if self.is_down() {
            return Err(ManagementError::Unavailable);
        }
        let known = self
            .users
            .lock()
            .unwrap()
            .iter()
            .any(|(_, u)| u.id == user_id);
        if !known {
            return Err(ManagementError::Scope);
        }
        // FR-004: the membership row only — the user row survives.
        let mut memberships = self.memberships.lock().unwrap();
        match memberships.remove(&(org.to_string(), user_id.to_string())) {
            Some(_) => Ok(()),
            None => Err(ManagementError::Scope),
        }
    }

    async fn create_org(&self, name: &str) -> Result<Org, ManagementError> {
        if self.is_down() {
            return Err(ManagementError::Unavailable);
        }
        if name.is_empty() || name.len() > 200 {
            return Err(ManagementError::Validation {
                field: "name",
                message: "org name must be non-empty and ≤ 200 chars".into(),
            });
        }
        let mut orgs = self.orgs.lock().unwrap();
        let id = format!("org-{}", orgs.len() + 1);
        let org = Org {
            id: id.clone(),
            name: name.into(),
        };
        orgs.insert(id, org.clone());
        Ok(org)
    }

    async fn list_orgs(&self) -> Result<Vec<(Org, u32)>, ManagementError> {
        if self.is_down() {
            return Err(ManagementError::Unavailable);
        }
        let orgs = self.orgs.lock().unwrap();
        let memberships = self.memberships.lock().unwrap();
        let mut out: Vec<(Org, u32)> = orgs
            .iter()
            .map(|(id, org)| {
                let count = memberships.iter().filter(|((o, _), _)| o == id).count() as u32;
                (org.clone(), count)
            })
            .collect();
        out.sort_by(|a, b| a.0.id.cmp(&b.0.id));
        Ok(out)
    }

    async fn rename_org(&self, org_id: &str, name: &str) -> Result<(), ManagementError> {
        if self.is_down() {
            return Err(ManagementError::Unavailable);
        }
        if name.is_empty() || name.len() > 200 {
            return Err(ManagementError::Validation {
                field: "name",
                message: "org name must be non-empty and ≤ 200 chars".into(),
            });
        }
        let mut orgs = self.orgs.lock().unwrap();
        match orgs.get_mut(org_id) {
            Some(o) => {
                o.name = name.into();
                Ok(())
            }
            None => Err(ManagementError::NotFound),
        }
    }

    async fn delete_org(&self, org_id: &str) -> Result<(), ManagementError> {
        if self.is_down() {
            return Err(ManagementError::Unavailable);
        }
        let orgs = self.orgs.lock().unwrap();
        if !orgs.contains_key(org_id) {
            return Err(ManagementError::NotFound);
        }
        drop(orgs);
        // FR-009: 409 not-empty guard on MEMBERSHIPS (no silent cascade); the
        // session cascade (CR-2b) is ordered before the org row delete — the
        // double holds no session rows (001 ground truth), so the guard is the
        // full behavior here; T013(c) proves the cascade on the real engine.
        let count = self
            .memberships
            .lock()
            .unwrap()
            .iter()
            .filter(|((o, _), _)| o == org_id)
            .count();
        if count > 0 {
            return Err(ManagementError::NotEmpty);
        }
        self.orgs.lock().unwrap().remove(org_id);
        Ok(())
    }
}

/// Role validation at the wire edge (FR-012, CR-1 — the engine CHECK is the
/// belt; this parse is the app-layer first line; the route 400 names the
/// field).
pub fn parse_role(raw: &str) -> Result<Role, ManagementError> {
    match raw {
        "admin" => Ok(Role::Admin),
        "member" => Ok(Role::Member),
        _ => Err(ManagementError::Validation {
            field: "role",
            message: "role must be 'admin' or 'member'".into(),
        }),
    }
}
