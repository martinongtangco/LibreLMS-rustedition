# Tasks: Course Enrollment & Completion (004)

**Input**: Design documents from `/specs/004-enrollment/`

**Prerequisites**: spec.md ✅, plan.md ✅, checklists/ ✅

**Organization**: Tasks grouped by user story; each story independently testable.
Tests are mandatory (spec SC-001: "verified by test, not by manual
observation") and are written first — red, then green.

## Format: `[ID] [P?] [Story] Description`

- **[P]**: Can run in parallel (different files, no dependencies)
- **[Story]**: US1/US2/US3 from spec.md

## Phase 1: Setup

- [ ] **T001** Declare crate dependencies in `crates/enrollment/Cargo.toml` (tiberius, tokio, serde/serde_json, async-trait — versions from workspace `Cargo.toml`; no new dependencies) and wire `enrollment` into the host (`crates/host/Cargo.toml`)
- [ ] **T002** [P] Create `crates/enrollment/src/migrations/0005_enrollment.sql` (dbo.enrollments with `UNIQUE (learner_id, course_id)` constraint + dbo.completions with `enrollment_id` PK — plan Data Model) and expose `pub const MIGRATIONS: &[(&str, &str)]` (spec 001 manifest-generic runner; whole-script batches per CR-E)

## Phase 2: Foundational (blocks all stories)

- [ ] **T003** [P] Define `EnrollmentRepo` trait, `CourseSource` trait (the agreed PQ-2 port shape — plan Cross-Slice Contract: the provider's 2-method minimal set + 404/403 decision procedure), and `EnrollmentError` (Scope/Forbidden/NotFound/AlreadyEnrolled/NotEnrolled/Unavailable) in `crates/enrollment/src/repo.rs` with the plan's error→envelope mapping table
- [ ] **T004** [P] Implement `InMemoryEnrollmentRepo` (deterministic double; the sequential duplicate check is labeled LOGIC-LAYER) + `FixtureCourseSource` in `crates/enrollment/src/memory.rs`
- [ ] **T005** Implement `TiberiusEnrollmentRepo` in `crates/enrollment/src/mssql.rs`: create (mapping the unique-constraint violation — MSSQL 2627 / tiberius unique-violation — to `AlreadyEnrolled`, never a 500 — PQ-1), list own/org-wide (org-scoped query), complete (INSERT-into-completions so the PK enforces first-completion-wins in the engine), status transition enrolled→completed; `Unavailable` on storage outage
- [ ] **T006** Wire `enrollment::routes` into the host router (one-line-mount convention, docs 767d690) with the host adapting `FixtureCourseSource` for now; `CourseSource` catalog→port adaptation is a Phase 4 task (this slice adds the adapter SEAM + a test with the fixture, not the catalog wiring)

**Checkpoint**: `cargo build --workspace` + `cargo clippy` green; foundation compiles, stories can begin

## Phase 3: User Story 1 — Enroll (P1) 🎯 MVP

**Goal**: create enrollment — the scoped, duplicate-safe, durable core.
**Independent Test**: in-scope create → row exists (status enrolled, durable);
duplicate → 409 stable code, one row; cross-org → 403, nothing written;
unauthenticated → 401; unknown course id → 404 (not 403).

### Tests (write first, must FAIL)

- [ ] **T007** [P] [US1] `crates/enrollment/tests/repo.rs`: create-in-scope persists with shape (learner, course, org, status enrolled); duplicate (sequential) → `AlreadyEnrolled`, single row (logic layer — the race proof is the full-env test, PQ-1); cross-org create → `Scope`, nothing written (SC-002 both directions)
- [ ] **T008** [P] [US1] `crates/host/tests/enrollment_flow.rs`: `POST /enrollments` — 401 envelope without session; 201 + visible for in-scope member; 403 `scope` for other-org course (content-identical to any other 403 — no existence oracle); 404 `not-found` for unknown course id; 409 `already-enrolled` for duplicate; 503 envelope when the repository reports `Unavailable` (never 500, never silent)

### Implementation

- [ ] **T009** [US1] Implement `POST /enrollments` route (auth extractor → `CourseSource::get` → 404/403 split → `repo.create` → 409 on `AlreadyEnrolled`) until T007/T008 green

**Checkpoint**: US1 independently functional — enroll is provable without any other module (spec US1 Independent Test)

## Phase 4: User Story 2 — Views by role (P2)

**Goal**: member sees own enrollments; admin sees the org-wide view; role + scope gates.
**Independent Test**: two members (one org) + a member of a second org + an admin: list per role, assert exact visibility (SC-002); empty list for a member with none; non-admin on the org-wide view → 403.

### Tests (write first, must FAIL)

- [ ] **T010** [P] [US2] `crates/enrollment/tests/repo.rs`: list-own returns only the actor's rows (with course identifier + status via the port's `course_title`); empty → empty (not an error); org-wide returns all org rows for admin, and member rows of other orgs never appear
- [ ] **T011** [P] [US2] `crates/host/tests/enrollment_flow.rs`: `GET /enrollments` — 200 own list for member; `GET /enrollments?scope=org` (or the plan's org-wide endpoint) 200 for admin, 403 `forbidden` for non-admin; 401 without session; org A never sees org B rows (SC-002)

### Implementation

- [ ] **T012** [US2] Implement `GET /enrollments` (own) + the org-wide admin view (role gate → `Forbidden`) until T010/T011 green

## Phase 5: User Story 3 — Completion (P3)

**Goal**: the enrolled→completed transition with first-completion-wins.
**Independent Test**: complete an enrolled enrollment → status completed + timestamped completion record; second completion → rejected, original unchanged; completion of a non-enrolled enrollment → rejected; out-of-scope completion → rejected, nothing persisted; after a "restart" (fresh double over the same rows) both survive (durable, spec US3.4).

### Tests (write first, must FAIL)

- [ ] **T013** [P] [US3] `crates/enrollment/tests/repo.rs`: complete transitions enrolled→completed + stores exactly one completion with a timestamp; second completion → rejected, original timestamp preserved (the `enrollment_id` PK is the engine anchor — assert in the double + it holds in the real engine by construction); completion of non-enrolled → `NotEnrolled`; out-of-scope completion → `Scope`, nothing persisted
- [ ] **T014** [P] [US3] `crates/host/tests/enrollment_flow.rs`: `POST /enrollments/{id}/completion` — 200 + status `completed` on GET; 409 `not-enrolled` for non-enrolled; 409 for double-completion with the original record unchanged; 404 unknown id; 403 out-of-scope; 503 on `Unavailable`

### Implementation

- [ ] **T015** [US3] Implement the completion route (status gate → INSERT completion → transition) until T013/T014 green

## Phase 6: Cross-Cutting & Polish

- [ ] **T016** [P] Gated full-environment test `crates/host/tests/enrollment_full_env.rs` (gated on live services — self-skips visibly, NEVER a silent pass): (a) the SC-003 RACE PROOF — N=8 concurrent duplicate create-attempts for one (learner, course) on live MSSQL assert exactly ONE row + losers all got the stable `already-enrolled` code (PQ-1); (b) migration idempotency on the real engine (second run no-op); (c) the US3.4 durability assertion (rows survive a fresh client reconnect)
- [ ] **T017** Full gate sweep on a clean checkout: `cargo fmt --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo build --workspace`, `cargo test --workspace --no-fail-fast`, `check-boundaries.sh` — all green (SC-004/SC-005 local half); the full-env test's deciding run is Agent B's fresh-container machine (Principle XVI — a SKIP is not a pass)
- [ ] **T018** Update `docs/work-split.md` status (Phase 3 Enrollment done) + PR body with the Principle XIII evidence (gate outputs + the fresh-container full-env run result + SC-003 race proof)

## Dependencies & Execution Order

- **Phase 1**: T001, T002 [P] — no dependencies
- **Phase 2**: T003/T004 [P] independent; T005 ← T003; T006 ← T003 (routes need the error mapping). BLOCKS all stories.
- **Phase 3 (US1)**: tests T007/T008 [P] first (red) → T009
- **Phase 4 (US2)**: tests T010/T011 [P] first (red) → T012; independent of US1 after Phase 2
- **Phase 5 (US3)**: tests T013/T014 [P] first (red) → T015
- **Phase 6**: after all stories

### Parallel Opportunities

- T002, T003, T004, T007, T008, T010, T011, T013, T014, T016 are [P] (distinct files)
- Single implementer per branch (Agent B): [P] tasks run within the same doloop session; the full-env half is Agent B's own machine (deterministic check per iteration, ceiling per Principle XIV)

## Implementation Strategy

1. Setup + Foundational → checkpoint (compiles, port + double + real repo exist)
2. US1 → checkpoint = **MVP** (durable, scoped, duplicate-safe enrollment)
3. US2 → checkpoint (role-scoped views proven)
4. US3 → checkpoint (completion transition proven)
5. Polish → full gate evidence + SC-003 race proof on the real engine → PR (Agent A reviews per Principle XVI; the deciding full-env run is mine to perform — I am the reviewing-machine owner)
