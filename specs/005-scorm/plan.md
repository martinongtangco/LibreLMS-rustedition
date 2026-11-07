# Implementation Plan: SCORM Attempt Lifecycle (005)

**Branch**: `story/005-scorm` | **Date**: 2026-11-07 | **Spec**: [spec.md](spec.md)

**Input**: Feature specification from `specs/005-scorm/spec.md` (Agent A
APPROVE + 2 CRs folded, `2851ccf`: FR-005 LMSFinish is an implicit final
LMSCommit; FR-001 the launch gate runs BEFORE the idempotent resume).

## Summary

The SCORM slice exposes the thin standard subset — LMSInitialize /
LMSGetValue / LMSSetValue / LMSCommit / LMSFinish over the `cmi` data
subset — on the spec 001 foundation: a live `cmi` bag at the 001-reserved
Valkey key (ephemeral by design), committed state in `dbo.scorm_attempts`
(MSSQL, the system of record, written on LMSCommit/LMSFinish — Principle
VI), an in-crate repository trait + tiberius implementation + in-memory
double (the spec 001 Module Persistence Seam), the module-owned bag-store
port, a host-wired 2-method enrollment port (the 004 `CourseSource` parity,
driving the 004 `enrolled → completed` transition — no enrollment-crate
dependency), its own `0006_scorm` migration manifest, and axum routes on the
host — with 401/403/404/409×3/400/503 envelope behavior, the engine-arbitrated
launch race (FR-001/US1.6), and no new shared surface.

## Technical Context

**Language/Version**: Rust 1.99.1, pinned via `rust-toolchain.toml` (gates
toolchain-agnostic; machine B runs the per-directory gnu override — spec 001
CR-3).

**Primary Dependencies**: tiberius (workspace, ADR 0003), axum 0.8, tokio,
serde/serde_json, tracing. **No new dependencies beyond the workspace**
(Principle XVIII). No Valkey client beyond the shared session store (the bag
store wraps it — Principle VI: the bag is the only ephemeral state, and it
lives in the 001-reserved key `lms:session:{token_hash}:bag`).

**Storage split (Principle VI — polyglot with a reason)**: the `cmi` bag is
high-churn ephemeral → Valkey via the module-owned `BagStore` port (production
wraps the shared session store; deterministic tests use an in-memory double);
the committed state (state, suspend_data, score_raw, session_time, finished_at)
is durable → MSSQL `dbo.scorm_attempts` via the in-crate repository (trait +
real impl + in-memory double). A store flush loses only the bag — the session
row and the committed state survive (FR-006 rehydrates the bag from the last
commit).

**Session/Auth**: the spec 001 session mechanism (identity + org scope via the
host auth extractor); the learner and organization come from the session — no
org parameter in the request (spec Assumptions).

**Wire discipline (Agent A plan-level requirement, verbatim)**: `0006` DDL
follows the 003/004 wire discipline — `id` as `NVARCHAR(40)` NEWID string,
**no `UNIQUEIDENTIFIER` columns read as `&str`** (this table has no
UNIQUEIDENTIFIER columns at all — the learner is implicit in the session),
**2601/2627 mapped on both error legs** (tiberius can surface the
unique-violation on the `query()` leg or the `into_row()` leg — spec 004
lesson), and the **N-launch race is N independent connections** (one
`TiberiusScormRepo`/connection each — the 004 PQ-1 discipline; a
single-client mutex pattern would serialize the race and prove nothing).

**Target Platform**: Linux (machine A) + Windows (machine B, gnu override);
gates toolchain-agnostic.

**Performance Goals**: teaching slice — every API call is ≤ 2 store round-trips
(bag or row, plus the commit write); no throughput targets (spec Assumptions).

**Constraints**: no cross-module crate dependencies (boundary check + negative
test, spec 001 US2); no new shared surface (shared frozen post-001); no
secrets in logs (spec 001 FR-009); whole-script migration batches (spec 001
CR-E — the runner is manifest-generic and executes each script as ONE batch);
`dbo.completions` is untouched (004 owns it — the 005 slice writes it only
through the port); app-level `suspend_data ≤ 4,000` chars (the column is
`NVARCHAR(MAX)` — the engine cap is 4,000 on non-MAX NVARCHAR, spec 002 CR-1
lesson).

## Constitution Check

*GATE: must pass before implementation. Checked against constitution v1.0.1 (actual principle names/numbers).*

| # | Principle (v1.0.1) | Status | Notes |
|---|---|---|---|
| I | Modular Monolith, Not Microservices (Yet) | ✅ PASS | one `host` binary; the scorm module composes via its public API + host-wired port |
| II | Clean Architecture, Applied Simply | ✅ PASS | the repository trait + bag-store port are the only seams; no further pattern wrapping |
| III | Module Boundaries Are Compiled | ✅ PASS | depends on `contracts` + `shared` only; the enrollment port lives IN this crate (host wires the 004 repository to it); boundary negative test unchanged |
| IV | Human-Legible AI-Authored Code | ✅ PASS | the gate-before-resume ordering, the implicit final commit, and the `finished_at` column decision are recorded below with one-sentence rationale |
| V | Parallel Two-Agent Protocol (NON-NEGOTIABLE) | ✅ PASS | SCORM = Agent B lane; `story/005-scorm`; planning on main; `contracts`/`shared` untouched |
| VI | Polyglot Storage With a Reason | ✅ PASS | the `cmi` bag is high-churn ephemeral → Valkey (001-reserved key); the committed attempt state is durable → MSSQL only |
| VII | Spec-Driven, Sliced Thin | ✅ PASS | spec 005 on `main` (CRs folded, `2851ccf`) before planning; thin vertical slice (launch → work → commit → finish → completion) |
| VIII | Branching Discipline | ✅ PASS | implementation on `story/005-scorm` |
| IX | Plan On Main Only | ✅ PASS | this plan authored on `main` |
| X | No Ad-Hoc Fixes — Document Before You Code | ✅ N/A | new slice, not a fix; all decisions documented here first |
| XI | Two-Machine Parallelism | ✅ PASS | host mount via the documented one-line-mount + rebase convention (docs 767d690); the 003+004 rebase is the worked example |
| XII | Return To Main After Implementation | ✅ PASS | `story/005-scorm` → PR → `main` (the slice ends merged or abandoned, never parked) |
| XIII | Verification Before Claim | ✅ PASS | red-first tasks; the real-engine proof (N-launch race, commit durability) is a real run on the reviewing machine (never a self-skip) |
| XIV | Bounded Retry — Every Loop Has a Ceiling | ✅ PASS | doloop iteration ceiling (state file per slice) |
| XV | Triage Before Retry | ✅ PASS | each failed cycle is triaged (diagnosis before re-run) per the doloop protocol |
| XVI | Cross-Agent Review for Merge (NON-NEGOTIABLE) | ✅ PASS | Agent A reviews spec (done — `2851ccf`)/plan/tasks, then the PR; the full-env gate run is the reviewing machine's |
| XVII | Verify Against a Disposable Environment | ✅ PASS | tests create their own rows; the gated full-env test self-skips visibly, the deciding run is real |
| XVIII | Rust Hygiene | ✅ PASS | `rust-toolchain.toml` pin, clippy `-D warnings`, fmt, no `unwrap`/`expect` outside tests |

**Gate result: PASS — no violations; Complexity Tracking not required.**

## Cross-Slice Contract (enrollment port — 2-method, host-wired)

`EnrollmentPort` — a port defined IN the `scorm` crate (the host wires the 004
enrollment repository + 003 course checks to it at Phase 4; deterministic tests
use an in-memory double with fault injection; no enrollment-crate dependency).
The error type carries the 404/403/503 distinction and the `Ok` payload the
409 distinction, so the launch gate maps 1:1 to the envelope:

```rust
pub enum LaunchGate { Enrolled, NotEnrolled, Completed }

#[async_trait]
pub trait EnrollmentPort: Send + Sync {
    /// `Err(NotFound)` = the course does not exist (→ 404-class, FR-002).
    /// `Err(Scope)` = the course exists but is not in `org_id` (→ 403-class —
    /// content-identical to any other 403, no existence oracle).
    /// `Ok(NotEnrolled)` = 409 `not-enrolled` (stable code — never a 404).
    /// `Ok(Completed)` = 409 `already-completed` (the 004 first-completion-wins
    /// guarantee; the prior committed attempt state remains the record).
    /// `Err(Unavailable)` = 503 (N1 discipline — an outage is never conflated
    /// with absence: no `Ok(NotEnrolled)`-on-outage, no fake 404/200).
    async fn enrollment_status(
        &self, learner_id: &str, course_id: &str, org_id: &str,
    ) -> Result<LaunchGate, ScormError>;
    /// Drives the 004 `enrolled → completed` transition. First-completion-wins
    /// is enforced by the 004 engine (`dbo.completions` PK) — a second call
    /// is an idempotent no-op, never an error and never a second timestamp.
    /// `Err(Unavailable)` = 503 — a completion outage is recorded as an outage,
    /// never as a fake success.
    async fn record_completion(
        &self, learner_id: &str, course_id: &str, org_id: &str,
    ) -> Result<(), ScormError>;
}
```

**Launch procedure (spec FR-001 CR-2 — the gate runs BEFORE the idempotent
resume)**:
1. `enrollment_status(learner, course, session_org)` → `Err(NotFound)` → 404;
   `Err(Scope)` → 403; `Err(Unavailable)` → 503; `Ok(NotEnrolled)` → 409
   `not-enrolled`; `Ok(Completed)` → 409 `already-completed` (US1.2 — a
   re-launch of a completed enrollment is NEVER a resume of the final
   attempt).
2. Idempotent resume: `find_by_session_course(session, course)` → `Some(row)`
   → 200 with the attempt state. If the row is **finalized** (`finished_at`
   set) → 409 `attempt-final` (the race-window belt: a finalized row with a
   still-`Enrolled` gate can only exist inside the completion race; a final
   attempt is never resumed). If the live bag is missing (store flush) →
   rehydrate the bag from the row's committed state (FR-006).
3. `None` → `create` the attempt (the `UNIQUE (session, course)` constraint is
   the engine arbiter under concurrent launches). A 2601/2627 unique-violation
   (either error leg) → read the winner's row by the UNIQUE key → 200 with the
   winner's state — never a 409, never a 500, never a second row (US1.6).
4. Initialize the live bag (`in_progress`, no score, empty suspend, 0s).

**Bag-store port (FR-003 — module-owned)**:

```rust
#[async_trait]
pub trait BagStore: Send + Sync {
    /// The 001-reserved key `lms:session:{token_hash}:bag` — the production
    /// implementation wraps the shared session store (the key itself stays in
    /// `shared` ground truth; the port never names it).
    async fn get_bag(&self, token_hash: &str) -> Result<Option<String>, ScormError>;
    async fn set_bag(&self, token_hash: &str, bag: &str, ttl_secs: u64) -> Result<(), ScormError>;
}
```

The `cmi` bag JSON shape (all four fields always present — init defaults
`in_progress` / `null` / `""` / `0`):
`{"lesson_status": <five-value>, "score": <0-100 int | null>,
"suspend_data": <text ≤ 4,000>, "session_time": <seconds>}`.

## Data Model

### `dbo.scorm_attempts` (owned by this slice; migration `0006_scorm`)

| column | type | notes |
|---|---|---|
| id | NVARCHAR(40) PK | **NEWID string — 003/004 wire discipline** (Agent A plan-level requirement); never a UNIQUEIDENTIFIER |
| session_id | NVARCHAR(256) NOT NULL | the 001 session `token_hash` — session-scoped id (one attempt per (session, course)) |
| course_id | NVARCHAR(40) NOT NULL | **1:1 with 003/004 `dbo.courses.id`** (NEWID string; no implicit CAST at the wire edge) |
| organization_id | NVARCHAR(40) NOT NULL | scope column (denormalized; enforced at the app layer — the session org) |
| state | NVARCHAR(20) NOT NULL | the five-value `cmi.core.lesson_status` subset (`in_progress` \| `passed` \| `failed` \| `completed` \| `incomplete`) |
| suspend_data | NVARCHAR(MAX) | opaque text; **app-level ≤ 4,000 chars** (the engine cap on non-MAX NVARCHAR is 4,000 — spec 002 CR-1 lesson); injection-shaped values stored/compared as plain text (FR-012) |
| score_raw | INT NULL | raw 0–100; scaled = raw/100 is computed, not stored (FR-004) |
| session_time | BIGINT NOT NULL | accumulated seconds |
| finished_at | BIGINT NULL | **plan decision (Principle X/IV)**: `NULL` = live attempt, set = final — the app-layer `attempt-final` discriminator (FR-005) without a boolean column |
| created_at / updated_at | BIGINT NOT NULL | unix seconds — **005's own convention** (001 is DATETIME2; no parity claim — the 004 N-1 lesson) |
| UNIQUE (session_id, course_id) | **constraint** | the US1.6 launch-race engine anchor (2601/2627 on both error legs) |

Note: no cross-module FKs (004 decision parity — `dbo.courses` is 003-owned,
`dbo.completions` is 004-owned); **no UNIQUEIDENTIFIER columns at all** — the
learner is implicit in the 001 session, so nothing here is read as `&str` from
a Guid column (the 003 CR-7/CR-8 class is impossible by construction).
`dbo.completions` is untouched — 004 owns it, and the 005 slice reaches it
only through `record_completion`. One-sentence explanation (Principle IV):
*the attempt row carries its own scope column and the learner comes from the
session, so the slice's DDL references no other module's tables.*

### `ScormError` → envelope mapping (host)

| variant | status | stable code |
|---|---|---|
| (absent session — extractor) | 401 | `unauthenticated` (foundation) |
| `Scope` (cross-org course) | 403 | `scope` (content-identical to any other 403 — no existence oracle) |
| `NotFound` (unknown course) | 404 | `not-found` |
| `NotEnrolled` | 409 | `not-enrolled` |
| `AlreadyCompleted` | 409 | `already-completed` |
| `AttemptFinal` (operation on a finished attempt) | 409 | `attempt-final` |
| `Validation(field)` (invalid `cmi` value — score/status/suspend_data/element) | 400 | `validation` (field named) |
| `Unavailable` (system-of-record OR session-store outage) | 503 | `storage-unavailable` |

`AttemptExists` (the 2601/2627 mapping, internal) is never an error response —
it is the loser path that reads the winner's row (US1.6). Never a 500, never
a silent state loss, never a fake success (FR-010).

## US1.6 Launch-Race Design (real-engine proof required)

The in-memory double's mutex serializes inserts, so a deterministic test proves
the LOGIC but not the RACE (Principle XIII/XVII: a stand-in that cannot race
is not evidence for a race-safety claim). Therefore:

1. **Engine anchor**: the `UNIQUE (session_id, course_id)` constraint in
   `0006_scorm` makes the engine the final arbiter under concurrency.
2. **Violation mapping**: `TiberiusScormRepo::create` maps the
   unique-constraint violation (MSSQL **2601 and 2627** — the engine surfaces
   either) on **both error legs** (the `query()` leg and the `into_row()` leg
   — tiberius can hide the batch error in the stream; spec 004 lesson) to the
   internal `AttemptExists` — never a 500, never a second row.
3. **The real proof (gated full-env test, `scorm_full_env`)**: fires N=8
   concurrent launches of the same (session, course) against live MSSQL with
   **N independent `TiberiusScormRepo` instances (one connection each)** —
   `tokio::join!` over them — and asserts exactly ONE attempt row exists,
   every caller ends with the same attempt (200), and no caller saw a 500 or a
   409. **Why N instances**: a single-client repo (the 001/003
   `Client`-behind-`Mutex` pattern — a fine teaching-slice choice for normal
   operations; the 003 CR-9 self-deadlock lesson applies to any
   write-then-sibling-call here, so every write is scoped under its own guard
   block) would serialize 8 joins on its own mutex, so the UNIQUE constraint
   would never see a real contest and the gate would pass while proving
   nothing (the 004 PQ-1 discipline). The race test needs genuine engine-side
   concurrency. Gated on live services (SKIP is not a pass); the deciding run
   is Agent B's fresh-container machine (Principle XVI).
4. **Commit durability across reconnect**: the full-env test opens a second
   repo instance (fresh connection) after an LMSCommit and asserts the
   committed state (state/suspend_data/score_raw/session_time) is readable —
   the system-of-record survives the session-store's ephemerality (FR-005/US1.3
   parity).
5. The deterministic test keeps its sequential launch-idempotency assertions
   (logic layer); it is labeled as such in the test name.

## Project Structure

```text
crates/scorm/
├── Cargo.toml          # depends: contracts, shared, tiberius (no other modules)
└── src/
    ├── lib.rs          # pub routes(Router) + MIGRATIONS manifest + port re-exports
    ├── repo.rs         # ScormRepo trait, EnrollmentPort + LaunchGate, BagStore, ScormError, the cmi validation battery
    ├── memory.rs       # InMemoryScormRepo + InMemoryBagStore + DoubleEnrollmentPort (fault-injecting)
    ├── tiberius.rs     # TiberiusScormRepo (real dbo.scorm_attempts; 2601/2627 both legs; guard-scoped writes — 003 CR-9 lesson)
    ├── routes.rs       # the thin SCORM API (initialize/get/set/commit/finish) + gate procedure + envelope mapping
    └── migrations/     # 0006_scorm.sql (manifest-exposed pub const)
host: scorm::routes mounted per the documented one-line-mount convention (docs 767d690);
     production wiring = real TiberiusScormRepo + BagStore wrapping the shared
     session store + the honest UnwiredEnrollmentPort (503 — never a fake
     404/200/completion, 004 N1 discipline) until Phase 4
tests: crates/scorm/tests/{repo.rs (doubles: launch idempotency, gate battery, finish discipline),
       bag.rs (resume-after-bag-loss + validation battery + injection)};
       crates/host/tests/{scorm_flow.rs, scorm_full_env.rs (gated: N=8 race + migration idempotency + commit durability)}
```

**Structure Decision**: the 7-crate workspace is kept exactly (work-split
contract, Principle V); this slice fills `scorm` only — `contracts`/`shared`
untouched (frozen post-001), no other module touched. The `finished_at`
column is the one DDL addition beyond the spec's column list — documented
here (Principle X) with its one-sentence rationale (the app-layer
`attempt-final` discriminator without a boolean column).

## Complexity Tracking

Not required — Constitution Check passed with no violations.
