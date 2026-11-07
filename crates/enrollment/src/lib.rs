//! Enrollment slice (spec 004, Module Persistence Seam).
//!
//! The repository trait + error types are defined IN THIS crate (the spec 001
//! CR-2 seam decision); `InMemoryEnrollmentRepo` (memory.rs) is the
//! deterministic double, `TiberiusEnrollmentRepo` (tiberius.rs) is the real
//! engine. Course data arrives through the `CourseSource` port (PQ-2 agreed
//! shape — the provider's 2-method minimal set); the host wires the catalog
//! adapter at Phase 4, tests use the in-crate fixture. No catalog-crate
//! dependency (Principle III).

pub mod memory;
pub mod migrations;
pub mod repo;
pub mod routes;
pub mod tiberius;
