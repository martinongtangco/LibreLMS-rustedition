//! SCORM slice (spec 005, Module Persistence Seam).
//!
//! The repository trait + error types + both ports are defined IN THIS crate
//! (the spec 001 CR-2 seam decision); `InMemoryScormRepo` / `InMemoryBagStore`
//! (memory.rs) are the deterministic doubles, `TiberiusScormRepo`
//! (tiberius.rs) is the real engine. Enrollment data arrives through the
//! `EnrollmentPort` (2-method, host-wired — the 004 `CourseSource` parity);
//! no enrollment-crate dependency (Principle III). The live `cmi` bag lives
//! in the session store at the 001-reserved key (Principle VI: the bag is
//! the only ephemeral state); the committed state persists to
//! `dbo.scorm_attempts` (the system of record) on LMSCommit/LMSFinish.

pub mod memory;
pub mod migrations;
pub mod repo;
pub mod routes;
pub mod tiberius;
