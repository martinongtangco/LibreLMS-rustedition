//! Management module (spec 006 — Organization & Learner Management).
//! T001: crate scaffold; T002 (red): the US1 repository battery defines the
//! `ManagementRepo` contract; T004 makes it green.

pub mod repo;

/// The module's migration manifest — EMPTY BY CONSTRUCTION (spec 006 FR-013,
/// plan decision #1): every table this slice writes is foundation-owned DDL
/// (0001) and a module must not ALTER foundation tables. The manifest-generic
/// runner handles `&[]` by construction (001 CR-2).
pub const MIGRATIONS: &[(&str, &str)] = &[];
