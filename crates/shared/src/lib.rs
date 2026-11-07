//! Shared infrastructure for the LibreLMS-rustedition host (ADR 0001).
//!
//! - `config`: fail-fast environment configuration (spec FR-001)
//! - `storage`: system-of-record access (MSSQL via tiberius — ADR 0003 — and
//!   the in-memory test double)
//! - `session`: ephemeral per-session state (Valkey; never durable — Principle VI)
//! - `migrate`: the ordered, checksummed migration manifest
//! - `mail`: mail-sending interface + development outbox (spec FR-010)

pub mod config;
pub mod mail;
pub mod migrate;
pub mod session;
pub mod storage;

pub use config::{Config, ConfigError};
