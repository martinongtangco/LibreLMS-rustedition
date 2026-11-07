//! System-of-record access (Principle VI: MSSQL is the system of record).
//!
//! `Storage` is an async trait with two implementations:
//! - `TiberiusStorage` (tiberius against MSSQL) — full environment
//! - `MemoryStorage` — deterministic test double (Principle XIII/XVII)
//!
//! No ORM wrapper layer (Principle II): the data client *is* the data-access
//! abstraction.

use async_trait::async_trait;

/// A session row (data-model.md). Timestamps are unix seconds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionRow {
    pub token_hash: String,
    pub user_id: String,
    pub organization_id: String,
    pub created_at: i64,
    pub last_activity: i64,
    pub expires_at: i64,
}

/// Inputs for creating a session row.
#[derive(Debug, Clone)]
pub struct NewSession {
    pub token_hash: String,
    pub user_id: String,
    pub organization_id: String,
    pub created_at: i64,
    pub last_activity: i64,
    pub expires_at: i64,
}

/// Schema readiness for the health check (contracts/api.md).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SchemaStatus {
    Ready,
    NotReady,
}

/// Storage failures. `Unavailable` maps to a degraded health / 503 path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StorageError {
    /// Backend unreachable or not ready (maps to `degraded` / 503).
    Unavailable(String),
    /// A required row was not found (maps to 401/404 semantics at the call site).
    NotFound(String),
    /// Anything else (message must never contain secrets — FR-009).
    Other(String),
}

impl std::fmt::Display for StorageError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let (kind, msg) = match self {
            StorageError::Unavailable(m) => ("unavailable", m),
            StorageError::NotFound(m) => ("not found", m),
            StorageError::Other(m) => ("error", m),
        };
        write!(f, "storage {kind}: {msg}")
    }
}

impl std::error::Error for StorageError {}

/// System-of-record access (users, organizations, memberships, sessions, migrations).
#[async_trait]
pub trait Storage: Send + Sync {
    /// Verify the backend is reachable.
    async fn verify(&self) -> Result<(), StorageError>;

    /// Schema readiness for the health check.
    async fn schema_status(&self) -> Result<SchemaStatus, StorageError>;

    /// Apply all pending migrations (idempotent — research R1).
    /// Returns the names of scripts applied during this call.
    async fn run_migrations(&self) -> Result<Vec<String>, StorageError>;

    async fn create_session(&self, s: &NewSession) -> Result<(), StorageError>;
    async fn get_session(&self, token_hash: &str) -> Result<Option<SessionRow>, StorageError>;
    /// Sliding-expiry touch (research R4). Returns false if the session is gone.
    async fn touch_session(
        &self,
        token_hash: &str,
        now: i64,
        new_expiry: i64,
    ) -> Result<bool, StorageError>;
    async fn delete_session(&self, token_hash: &str) -> Result<(), StorageError>;

    async fn get_user(&self, id: &str) -> Result<Option<contracts::User>, StorageError>;
    async fn get_user_by_email(&self, email: &str)
        -> Result<Option<contracts::User>, StorageError>;
    /// The user's role within the organization, if any (scope enforcement, R5).
    async fn org_role(
        &self,
        user_id: &str,
        organization_id: &str,
    ) -> Result<Option<contracts::Role>, StorageError>;
}

mod memory;
mod mssql;

pub use memory::MemoryStorage;
pub use mssql::TiberiusStorage;
