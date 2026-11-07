# Implementation Plan: Course Enrollment & Completion (004)

**Branch**: `story/004-enrollment` | **Date**: 2026-07-07 | **Spec**: [spec.md](spec.md)

**Input**: Feature specification from `specs/004-enrollment/spec.md`

## Summary

The Enrollment slice exposes enroll, list (own + admin org-wide), and
completion recording over the spec 001 foundation: an in-crate repository
trait + tiberius implementation + in-memory double (the spec 001 Module
Persistence Seam), its own `NNNN_*.sql` migrations exposed as `pub const
MIGRATIONS`, a `CourseSource` port the host wires (no catalog-crate
dependency), and axum routes on the host — with 401/403/404/503 envelope
behavior, a real-engine race-safety proof for duplicate enrollment (SC-003),
and no new shared surface.

## Technical Context

**Language/Version**: Rust 1.99.1, pinned via `rust-toolchain.toml` (gates
toolchain-agnostic; machine B runs the per-directory gnu override — spec 001
CR-3).

**Primary Dependencies**: tiberius (workspace, ADR 0003), axum 0.8, askama
0.12 (manual `IntoResponse` adapter — spec 001 R3), tokio, serde/serde_json,
tracing. **No new dependencies beyond the workspace** (Principle XVIII).

**Storage**: MSSQL (system of record) — the slice owns `dbo.enrollments` +
`dbo.completions` via the in-crate repository (trait + real impl + in-memory
double), per the spec 001 Module Persistence Seam. Deterministic tests = the
in-memory double; real-SQL verification = the gated full-environment test
(Principle XIII/XVI — the deciding run is Agent B's fresh-container machine).
No Valkey state (Principle VI: nothing here is high-churn ephemeral).

**Session/Auth**: the spec 001 session mechanism (identity + org scope + role
via the host auth extractor); resource-level scope check:
`course.organization_id != session.organization_id → 403 envelope` (no
existence oracle: out-of-scope → 403, unknown id → 404 — consistent with spec
003's rule).

**Target Platform**: Linux (machine A) + Windows (machine B, gnu override);
gates toolchain-agnostic.

**Performance Goals**: teaching slice — list of own/org enrollments is a
single query; no throughput targets (spec Assumptions).

**Constraints**: no cross-module crate dependencies (boundary check + negative
test, spec 001 US2); no new shared surface (shared frozen post-001); no
secrets in logs (spec 001 FR-009); whole-script migration batches (spec 001
CR-E — the runner is manifest-generic and executes each script as ONE batch);
fresh-environment behavior is app-guaranteed (spec 001 CR-G).

## Constitution Check

*GATE: must pass before implementation. Checked against constitution v1.0.1 (actual principle names/numbers).*

| # | Principle (v1.0.1) | Status | Notes |
|---|---|---|---|
| I | Modular Monolith, Not Microservices (Yet) | ✅ PASS | one `host` binary; the enrollment module composes via its public API + host-wired port |
| II | Clean Architecture, Applied Simply | ✅ PASS | repository trait is the single seam; no repository-pattern wrapping beyond it |
| III | Module Boundaries Are Compiled | ✅ PASS | depends on `contracts` + `shared` only; `CourseSource` port lives IN this crate (host wires catalog→port); boundary negative test extends the check |
| IV | Human-Legible AI-Authored Code | ✅ PASS | port + race-safety decision recorded below (one-sentence explainable) |
| V | Parallel Two-Agent Protocol (NON-NEGOTIABLE) | ✅ PASS | Enrollment = Agent B lane; `story/004-enrollment`; planning on main; `contracts`/`shared` untouched |
| VI | Polyglot Storage With a Reason | ✅ PASS | enrollments/completions are durable → MSSQL only; nothing in Valkey |
| VII | Spec-Driven, Sliced Thin | ✅ PASS | spec 004 on `main` before planning; thin vertical slice (enroll → list → complete) |
| VIII | Branching Discipline | ✅ PASS | implementation on `story/004-enrollment` |
| IX | Plan On Main Only | ✅ PASS | this plan authored on `main` |
| XI | Two-Machine Parallelism | ✅ PASS | runs parallel to 003; host mount via the documented one-line-mount + rebase convention (docs commit 767d690) |
| XIII | Verification Before Claim | ✅ PASS | red-first tasks; full-env proof is a real run on Agent B's machine (never a self-skip) |
| XVI | Cross-Agent Review for Merge (NON-NEGOTIABLE) | ✅ PASS | Agent A reviews spec/plan/tasks, then the PR; full-env gate run by Agent B |
| XVII | Verify Against a Disposable Environment | ✅ PASS | tests create their own rows; gated full-env test self-skips visibly, deciding run is real |
| XVIII | Rust Hygiene | ✅ PASS | `rust-toolchain.toml` pin, clippy `-D warnings`, fmt, no `unwrap`/`expect` outside tests |

**Gate result: PASS — no violations; Complexity Tracking not required.**

## Cross-Slice Contract (PQ-2 — agreed with spec 003's plan)

`CourseSource` — a port defined IN the `enrollment` crate (host wires the
catalog implementation to it at Phase 4; deterministic tests use a fixture
impl; no catalog-crate dependency):

```rust
/// A course as the enrollment slice needs it: which org owns it + its title.
pub struct CourseRef {
    pub organization_id: String,
    pub title: String,
}
#[async_trait]
pub trait CourseSource: Send + Sync {
    /// None = the course does not exist (→ 404-class).
    /// Some(ref) with ref.organization_id != session org (→ 403-class,
    /// no existence oracle — indistinguishable from any other 403).
    async fn get(&self, course_id: &str) -> Result<Option<CourseRef>, EnrollmentError>;
}
```

One call covers the minimal set (existence → 404 check; ownership → 403 check;
title → display). Satisfies the spec 003 plan's proposed surface
(`exists_in_org = get().map(|r| r.organization_id == org).unwrap_or(false)`;
`title = get().map(|r| r.title)`); spec 003's catalog repository exposes
exactly `get`, the host adapts catalog→port.

## Data Model

### `dbo.enrollments` (owned by this slice; migration `0005_enrollment`)

| column | type | notes |
|---|---|---|
| id | UNIQUEIDENTIFIER PK | |
| learner_id | UNIQUEIDENTIFIER NOT NULL | the enrolling user (no cross-module FK to `dbo.users` — see note) |
| course_id | UNIQUEIDENTIFIER NOT NULL | |
| organization_id | UNIQUEIDENTIFIER NOT NULL | scope column (denormalized; enforced at the app layer) |
| status | NVARCHAR(20) NOT NULL | `'enrolled' \| 'completed'` |
| created_at | DATETIME2 NOT NULL | |
| UNIQUE (learner_id, course_id) | **constraint** | the SC-003 race-safety anchor (PQ-1) |

### `dbo.completions` (owned by this slice; migration `0005_enrollment`)

| column | type | notes |
|---|---|---|
| enrollment_id | UNIQUEIDENTIFIER PK | PK enforces "at most one completion per enrollment" in the engine (first-completion-wins, spec US3.2) |
| completed_at | DATETIME2 NOT NULL | the first completion's timestamp is preserved (no UPDATE path) |

Note: no cross-module FKs (`dbo.courses` is owned by spec 003; an FK would
make this slice's migration order-dependent on theirs — a data-level coupling
the seam design avoids). Integrity (course exists, is in-scope) is enforced by
the `CourseSource` check at write time; the denormalized `organization_id`
makes listing scope-filterable without a join. One-sentence explanation
(Principle IV): *enrollment rows carry their own scope column and check the
course through the port, so the two modules' tables never reference each
other's DDL.*

### `EnrollmentError` → envelope mapping (host)

| variant | status | stable code |
|---|---|---|
| (absent session — extractor) | 401 | `unauthenticated` (foundation) |
| `Scope` (cross-org create/read/complete) | 403 | `scope` (foundation) |
| `Forbidden` (non-admin org-wide view) | 403 | `forbidden` |
| `NotFound` (unknown course / enrollment id) | 404 | `not-found` |
| `AlreadyEnrolled` (duplicate, incl. unique-violation) | 409 | `already-enrolled` |
| `NotEnrolled` (completion on non-enrolled) | 409 | `not-enrolled` |
| `Unavailable` (storage outage) | 503 | `storage-unavailable` |

Unknown course (404) and out-of-scope course (403) are distinct by design —
but an out-of-scope response is content-identical to any other 403 (no
existence oracle, spec 003 US3.2 parity).

## SC-003 Race-Safety Design (PQ-1 — real-engine proof required)

The in-memory double's mutex serializes inserts, so a deterministic test
proves the LOGIC but not the RACE (Principle XIII/XVII: a stand-in that
cannot race is not evidence for a race-safety claim). Therefore:

1. **Engine anchor**: the `UNIQUE (learner_id, course_id)` constraint in
   `0005_enrollment` makes the engine the final arbiter under concurrency.
2. **Violation mapping**: `TiberiusEnrollmentRepo::create` maps the
   unique-constraint violation (MSSQL 2627 / tiberius unique-violation error)
   to `EnrollmentError::AlreadyEnrolled` (→ 409 `already-enrolled`) — never a
   500, never a silent second row.
3. **The real proof (gated full-env test, `enrollment_full_env`)**: fires N=8
   concurrent duplicate create-attempts for the same (learner, course) against
   live MSSQL and asserts exactly ONE row exists and the losers all received
   the stable `already-enrolled` code. Gated on live services (SKIP is not a
   pass); the deciding run is Agent B's fresh-container machine (Principle
   XVI).
4. The deterministic test keeps its sequential duplicate-rejection assertions
   (logic layer); it is labeled as such in the test name.

## Project Structure

```text
crates/enrollment/
├── Cargo.toml          # depends: contracts, shared, tiberius (no other modules)
└── src/
    ├── lib.rs          # pub routes(Router) + MIGRATIONS manifest + CourseSource port + re-exports
    ├── repo.rs         # EnrollmentRepo trait, CourseSource trait + CourseRef, EnrollmentError, scope/role rules
    ├── memory.rs       # InMemoryEnrollmentRepo + FixtureCourseSource (deterministic double)
    ├── mssql.rs        # TiberiusEnrollmentRepo (real dbo.enrollments / dbo.completions; unique-violation mapping)
    └── migrations/     # 0005_enrollment.sql (manifest-exposed pub const)
migrations-free: the slice's migrations ride the manifest-generic runner (spec 001 CR-2 runner: Storage::run_migrations(manifest); host concatenates base + module manifests)
host: enrollment::routes mounted per the documented one-line-mount convention (docs 767d690); CourseSource adapted catalog→port at Phase 4 (fixture in this slice's tests)
tests: crates/enrollment/tests/{repo.rs, scope.rs, race.rs (logic layer)};
       crates/host/tests/{enrollment_flow.rs, enrollment_full_env.rs (gated: race proof + idempotency on the real engine)}
```

**Structure Decision**: the 7-crate workspace is kept exactly (work-split
contract, Principle V); this slice fills `enrollment` only — `contracts`/
`shared` untouched (frozen post-001), no other module touched.

## Complexity Tracking

Not required — Constitution Check passed with no violations.
