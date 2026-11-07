# Tasks: SCORM Attempt Lifecycle (005)

**Input**: Design documents from `/specs/005-scorm/`

**Prerequisites**: spec.md ✅ (Agent A APPROVE, CRs folded `2851ccf`), plan.md ✅, checklists/ ✅

**Organization**: Tasks grouped by user story; each story independently testable.
Tests are mandatory (spec FR-013: deterministic + full-environment, a SKIP is
not a pass) and are written first — red, then green.

## Format: `[ID] [P?] [Story] Description`

- **[P]**: Can run in parallel (different files, no dependencies)
- **[Story]**: US1/US2/US3 from spec.md

## Phase 1: Setup

- [ ] **T001** Declare crate dependencies in `crates/scorm/Cargo.toml` (tiberius, tokio, serde/serde_json, async-trait — versions from workspace `Cargo.toml`; no new dependencies) and wire `scorm` into the host (`crates/host/Cargo.toml`)
- [ ] **T002** [P] Create `crates/scorm/src/migrations/0006_scorm.sql` (`dbo.scorm_attempts` per the plan Data Model — id NVARCHAR(40) PK NEWID string, session_id, course_id NVARCHAR(40), organization_id NVARCHAR(40), state NVARCHAR(20), suspend_data NVARCHAR(MAX), score_raw INT NULL, session_time BIGINT, finished_at BIGINT NULL, created_at/updated_at BIGINT (005's own convention — no 001 parity claim), `UNIQUE (session_id, course_id)` — the US1.6 engine anchor; NO UNIQUEIDENTIFIER columns) and expose `pub const MIGRATIONS: &[(&str, &str)]` (spec 001 manifest-generic runner; whole-script batches per CR-E)

## Phase 2: Foundational (blocks all stories)

- [ ] **T003** [P] Define `ScormRepo` trait (`find_by_session_course`, `create` with the 2601/2627→`AttemptExists` mapping on BOTH error legs, `update_committed`), `EnrollmentPort` + `LaunchGate` (the plan Cross-Slice Contract: 2-method, the error type carries 404/403/503, the Ok payload carries the 409 distinction), `BagStore` (module-owned, FR-003), and `ScormError` (NotFound/Scope/NotEnrolled/AlreadyCompleted/AttemptFinal/Validation(field)/Unavailable + internal AttemptExists) in `crates/scorm/src/repo.rs` with the plan's error→envelope mapping table
- [ ] **T004** [P] Implement `InMemoryScormRepo` (deterministic double; the sequential duplicate check is labeled LOGIC-LAYER — the race proof is the full-env test, US1.6), `InMemoryBagStore` (doubles the 001-reserved-key semantics; a `flush()` test seam for the FR-006 bag-loss scenario), and `DoubleEnrollmentPort` (fault-injecting: gate outcomes + `Unavailable` injection — the N1 discipline is testable) in `crates/scorm/src/memory.rs`
- [ ] **T005** Implement `TiberiusScormRepo` in `crates/scorm/src/tiberius.rs`: `create` (the 2601 AND 2627 unique-violation mapping on both the `query()` leg and the `into_row()` leg → `AttemptExists`, never a 500, never a second row — the 004 lesson), `find_by_session_course` (SELECT string columns only — no non-string column is read as `&str`: the 003 CR-7/CR-8 class is impossible by construction), `update_committed` (parameterized; every write scoped under its own guard block — the 003 CR-9 self-deadlock lesson: never a sibling call while holding the client mutex); `Unavailable` on storage outage; `verify()` mirroring the 001/003/004 pattern
- [ ] **T006** Wire `scorm::routes` into the host router (one-line-mount convention, docs 767d690) with the production wiring: real `TiberiusScormRepo`, `BagStore` wrapping the shared session store at the 001-reserved key, and the honest `UnwiredEnrollmentPort` (outage → 503 — never a fake 404/200/completion, 004 N1 discipline); the 004-repository adapter is a Phase 4 task (this slice adds the seam + the fault-injection test, not the wiring)

**Checkpoint**: `cargo build --workspace` + `cargo clippy` green; foundation compiles, stories can begin

## Phase 3: User Story 1 — Launch, work, commit (P1) 🎯 MVP

**Goal**: the gated, idempotent, race-safe launch + the cmi bag lifecycle.
**Independent Test**: enrolled launch → 200 + bag live at the reserved key;
gate battery (404/403/409 not-enrolled/409 already-completed/503) with the
GATE BEFORE the resume (CR-2); second launch resumes (one row); the five
API methods over the cmi subset; the validation battery (400 naming the
element); commit persists to the system of record while the bag stays live.

### Tests (write first, must FAIL)

- [ ] **T007** [P] [US1] `crates/scorm/tests/repo.rs`: launch gate battery over the `DoubleEnrollmentPort` (unknown course → `NotFound`; cross-org → `Scope`; not enrolled → `NotEnrolled`; completed → `AlreadyCompleted` — and a completed gate NEVER resumes the final attempt, CR-2; port `Unavailable` → 503, never `Ok(NotEnrolled)`-as-404); launch idempotency (second launch same (session, course) → same attempt, no second row — LOGIC-LAYER); `AttemptExists` → read-winner path (the double simulates the loser)
- [ ] **T008** [P] [US1] `crates/scorm/tests/bag.rs`: LMSSetValue/LMSGetValue over the cmi subset (set/get `lesson_status` five values, `score` 0–100, `suspend_data`, session time accumulation); the validation battery (score out-of-range/non-integer → 400 naming the element; status off-subset → 400; `suspend_data` > 4,000 chars → 400; unknown element → 400); LMSCommit persists (state/suspend_data/score_raw/session_time in the repo) with the bag still live; the FR-006 resume-after-bag-loss (flush the `InMemoryBagStore`, re-launch, bag rehydrated from the last commit — suspend_data and score intact; no commit → fresh `in_progress`)
- [ ] **T009** [US1] `crates/host/tests/scorm_flow.rs`: the thin SCORM API over the host — 401 envelope without session; 200 launch + bag at the reserved key; the gate battery (404 `not-found`, 403 `scope` content-identical, 409 `not-enrolled`, 409 `already-completed` — gate before resume, CR-2); the validation battery (400 `validation` naming the element); LMSCommit durability (read-back through the repo); 503 `storage-unavailable` on repository OR bag-store outage (never 500, never silent)

### Implementation

- [ ] **T010** [US1] Implement `crates/scorm/src/routes.rs`: `LMSInitialize` (the plan Launch Procedure — gate → idempotent resume → create-with-read-winner), `LMSGetValue`, `LMSSetValue` (validation + bag update), `LMSCommit` (bag → `update_committed`; the bag stays live), the 001-style envelope mapping, and the session-scoped actor (learner + org from the 001 session; no org parameter)

**Checkpoint**: US1 independent test green; the cmi bag lifecycle is complete pre-finish

## Phase 4: User Story 1 — Finish & completion (P1)

**Goal**: the implicit final commit + the 004 transition + the final-state lock.
**Independent Test**: finish with `passed` → attempt row `passed` + the
completion recorded (first-wins through the port); finish with `failed` →
persisted, no completion; finish with NO prior commit (the common path) →
still persisted; post-finish set/commit → 409 `attempt-final`.

### Tests (write first, must FAIL)

- [ ] **T011** [P] [US1] `crates/scorm/tests/repo.rs` (extension): LMSFinish as an **implicit final LMSCommit** (FR-005 CR-1 — the bag at finish time persisted before finalization; Finish-without-Commit is the common path); `passed`/`completed` → `record_completion` invoked (the double counts calls; a port `Unavailable` → 503, never a fake success); `failed`/`incomplete` → persisted, `record_completion` NOT invoked; post-finish `LMSSetValue`/`LMSCommit` → `AttemptFinal` (409); the finished-at column set exactly once (first-wins is the 004 engine's guarantee — the double never re-records)
- [ ] **T012** [P] [US1] `crates/scorm/tests/bag.rs` (extension): the injection battery (FR-012 — `suspend_data` containing SQL/JSON metacharacters round-trips as plain text through the double; the tiberius path is parameterized by construction); the spec US1 acceptance scenarios 1–5 as end-to-end flows over the doubles

### Implementation

- [ ] **T013** [US1] Extend `routes.rs`: `LMSFinish` (implicit final commit → `update_committed` + `finished_at`; the completion port call only for `passed`/`completed`; the `AttemptFinal` lock on all post-finish mutations)

## Phase 5: User Story 2/3 — Seam + robustness (P2/P3)

**Goal**: the module seam (FR-007) is proven by the boundary negative test and
the 001 ground-truth respect; FR-010/FR-011/FR-013 are already exercised
above — this phase is the real-engine proof.

### Tests (write first, must FAIL)

- [ ] **T014** [P] [US2] `crates/host/tests/scorm_full_env.rs` (gated — SKIP is not a pass): the **N=8 independent-connection launch race** (8 `TiberiusScormRepo` instances, one connection each, `tokio::join!` over the same (session, course) → exactly ONE row, every caller 200 with the same attempt, no 500, no 409 — US1.6/FR-001); **migration idempotency** (second `run_migrations` is a no-op — the 001 US2.3 contract); **commit durability across reconnect** (a second repo instance reads the committed state — the system of record survives the bag's ephemerality, FR-005)
- [ ] **T015** [P] [US2] Boundary negative test assertion: `crates/scorm` depends on `contracts` + `shared` only (the module-boundaries check — spec 001 US2; no enrollment/catalog crate in `Cargo.toml`, proven by the check script)

### Implementation & gates

- [ ] **T016** fmt pass: `cargo fmt` clean across the slice
- [ ] **T017** Full gate sweep: `cargo test --workspace` (deterministic: doubles + flow + gated-self-skip all green) + `cargo clippy --workspace --all-targets -- -D warnings` + `cargo fmt --check` + boundary check — 100% of the workspace, red-first evidence recorded per battery
- [ ] **T018** Update `docs/work-split.md` (Phase 3 SCORM status) and push `story/005-scorm`; open the PR (Agent A review — Principle XVI; the deciding full-env run is Agent B's fresh-container machine with the pre/post state probes: pre 4060-fresh, post `schema_migrations` 4 rows + the attempt rows + the completion through the 004 path)

**Dependency order**: T001/T002 (parallel) → T003/T004 (parallel) → T005 → T006 → T007/T008 (parallel, RED) → T009 → T010 → T011/T012 (parallel, RED) → T013 → T014/T015 (parallel, RED) → T016 → T017 → T018.

**Parallelization**: [P]-marked tasks touch disjoint files; the doloop runs the red battery, triages (Principle XV), and iterates under the ceiling (Principle XIV).
