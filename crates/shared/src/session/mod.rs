//! Ephemeral session state (Principle VI: Valkey holds only high-churn,
//! non-durable state — the live SCORM bag per session).

use async_trait::async_trait;

/// Session store failures. `Unavailable` maps to the 503 path (spec US1.4).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StoreError {
    /// Store unreachable (degraded health / 503).
    Unavailable(String),
    /// Other error (message must never contain secrets — FR-009).
    Other(String),
}

impl std::fmt::Display for StoreError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let (kind, msg) = match self {
            StoreError::Unavailable(m) => ("unavailable", m),
            StoreError::Other(m) => ("error", m),
        };
        write!(f, "session store {kind}: {msg}")
    }
}

impl std::error::Error for StoreError {}

/// Key shape per data-model.md: `lms:session:{token_hash}:bag`.
pub fn bag_key(token_hash: &str) -> String {
    format!("lms:session:{token_hash}:bag")
}

/// Ephemeral per-session state (the SCORM `cmi.*` bag once the SCORM slice
/// lands). Values are JSON strings.
#[async_trait]
pub trait SessionStore: Send + Sync {
    /// Verify the store is reachable.
    async fn verify(&self) -> Result<(), StoreError>;

    /// Store a bag with TTL (PRINCIPLE: TTL = session lifetime, refreshed with
    /// the sliding expiry).
    async fn set_bag(&self, token_hash: &str, value: &str, ttl_secs: u64)
        -> Result<(), StoreError>;

    /// Read a bag; `None` if absent or expired.
    async fn get_bag(&self, token_hash: &str) -> Result<Option<String>, StoreError>;

    /// Extend the bag's TTL (sliding refresh).
    async fn refresh_ttl(&self, token_hash: &str, ttl_secs: u64) -> Result<(), StoreError>;
}

mod memory;
mod valkey;

pub use memory::MemorySessionStore;
pub use valkey::ValkeySessionStore;
