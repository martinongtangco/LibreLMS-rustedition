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
    memberships: std::sync::Mutex<
        std::collections::HashMap<(String, String), Role>,
    >,
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
        self.down
            .store(down, std::sync::atomic::Ordering::SeqCst);
    }

    fn is_down(&self) -> bool {
        self.down.load(std::sync::atomic::Ordering::SeqCst)
    }

    /// T002 RED: the stub fails the battery on purpose until T004.
    fn stub(&self) -> ManagementError {
        if self.is_down() {
            ManagementError::Unavailable
        } else {
            ManagementError::Validation {
                field: "stub",
                message: "T002 red: implementation lands in T004".into(),
            }
        }
    }
}

#[async_trait::async_trait]
impl ManagementRepo for InMemoryManagementRepo {
    async fn create_user(
        &self,
        _org: &str,
        _email: &str,
        _name: &str,
        _role: &Role,
    ) -> Result<ManagedUser, ManagementError> {
        Err(self.stub())
    }
    async fn list_users(&self, _org: &str) -> Result<Vec<ManagedUser>, ManagementError> {
        Err(self.stub())
    }
    async fn set_role(
        &self,
        _org: &str,
        _user_id: &str,
        _role: &Role,
    ) -> Result<(), ManagementError> {
        Err(self.stub())
    }
    async fn remove_membership(
        &self,
        _org: &str,
        _user_id: &str,
    ) -> Result<(), ManagementError> {
        Err(self.stub())
    }
    async fn create_org(&self, _name: &str) -> Result<Org, ManagementError> {
        Err(self.stub())
    }
    async fn list_orgs(&self) -> Result<Vec<(Org, u32)>, ManagementError> {
        Err(self.stub())
    }
    async fn rename_org(&self, _org_id: &str, _name: &str) -> Result<(), ManagementError> {
        Err(self.stub())
    }
    async fn delete_org(&self, _org_id: &str) -> Result<(), ManagementError> {
        Err(self.stub())
    }
}

/// T002 RED stub: role validation at the wire edge (FR-012, CR-1 — the
/// engine CHECK; the route-level 400 is the belt). Red: rejects everything;
/// T004 implements the real {admin, member} parse.
pub fn parse_role(_raw: &str) -> Result<Role, ManagementError> {
    Err(ManagementError::Validation {
        field: "stub",
        message: "T002 red: parse_role lands in T004".into(),
    })
}
