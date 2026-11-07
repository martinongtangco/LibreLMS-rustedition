# Tasks: Application Foundation

**Input**: Design documents from `/specs/001-foundation/`

**Prerequisites**: plan.md ✅, spec.md ✅, research.md ✅, data-model.md ✅, contracts/ ✅

**Organization**: Tasks grouped by user story; each story independently testable.
Tests are mandatory for this spec (spec SC-004: "verified by test, not by manual
observation") and are written first — red, then green.

## Format: `[ID] [P?] [Story] Description`

- **[P]**: Can run in parallel (different files, no dependencies)
- **[Story]**: US1/US2/US3 from spec.md

## Phase 1: Setup

- [x] T001 Declare crate dependencies in `crates/shared/Cargo.toml` (tiberius, redis, cookie, tracing, serde, sha2, hex — versions from workspace `Cargo.toml`) and `crates/host/Cargo.toml` (axum, tower-http, askama, tokio)
- [x] T002 [P] Create `docker-compose.yml` (mssql 2022 + valkey 8, healthchecks, env from `.env`) and extend `.env.example` with `MSSQL_SA_PASSWORD`

## Phase 2: Foundational (blocks all stories)

- [x] T003 [P] Implement `shared/src/config.rs`: env parsing (`DATABASE_URL`, `VALKEY_URL`, `PORT`, `SESSION_TTL_SECS`, `SESSION_MAX_TTL_SECS`, `LOG_LEVEL`) with fail-fast validation naming the missing/malformed item (FR-001)
- [x] T004 [P] Define `Storage` and `SessionStore` async traits in `crates/shared/src/storage/mod.rs` and `crates/shared/src/session/mod.rs` (session rows, ephemeral bag with TTL, user/org lookups)
- [x] T005 [P] Implement `MemoryStorage` + `MemorySessionStore` (test doubles, `Arc<Mutex<...>>`) in `crates/shared/src/storage/memory.rs` and `crates/shared/src/session/memory.rs`
- [x] T006 Implement `TiberiusStorage` (tiberius pool, connection verification, session/bag queries) in `crates/shared/src/storage/mssql.rs` (depends on T004)
- [x] T007 Implement migration runner in `crates/shared/src/migrate/mod.rs` (ordered `migrations/*.sql`, `schema_migrations` bookkeeping, idempotent, transactional where supported — R1)
- [x] T008 [P] Write `migrations/0001_base.sql` (users, organizations, user_organizations, sessions, schema_migrations) + `migrations/0002_seed.sql` (dev user `dev@example.com`, dev org, admin membership — idempotent) per data-model.md
- [x] T009 [P] Implement `ValkeySessionStore` (redis-rs tokio-comp, `lms:session:{hash}:bag` key, TTL refresh, graceful error when unreachable) in `crates/shared/src/session/valkey.rs`
- [x] T010 [P] Implement `shared/src/mail/mod.rs`: `Mailer` trait + `DevOutbox` (in-process queue, drainable for the dev page — FR-010)
- [x] T011 Implement contract shapes in `crates/contracts/src/lib.rs` (User, Organization, Membership, Course, Enrollment, Completion, ScormAttemptStatus — serde, no other deps) per data-model.md

**Checkpoint**: `cargo build --workspace` + `cargo clippy` green; foundation compiles, stories can begin

## Phase 3: User Story 1 — Lifecycle (P1) 🎯 MVP

**Goal**: start → health → degraded → graceful shutdown, all proven by test.
**Independent Test**: start with valid config → `/healthz` ok; remove each required item → fail-fast naming it; kill signal → clean exit; store down → 503 path alive.

### Tests (write first, must FAIL)

- [x] T012 [P] [US1] Lifecycle tests in `crates/host/tests/lifecycle.rs`: startup-with-valid-config → `/healthz` 200 ok (MemoryStorage + MemorySessionStore); degraded-when-session-store-down → 503 `degraded`; port-in-use → fail-fast naming the port
- [x] T013 [P] [US1] Fail-fast tests in `crates/host/tests/failfast.rs`: each of the 3 missing-`*_URL`/config variants exits < 5 s, non-zero, message names the item (SC-002)
- [x] T014 [P] [US1] Graceful shutdown test in `crates/host/tests/shutdown.rs`: signal → stops accepting, exits ≤ 10 s

### Implementation

- [x] T015 [US1] Implement `crates/host/src/error.rs`: error envelope `{"error","message"}`, mapping 404/401/403/503, no stack traces/internal paths (contracts/api.md)
- [x] T016 [US1] Implement `crates/host/src/health.rs`: `GET /healthz` JSON per contracts/api.md, no auth
- [x] T017 [US1] Implement `crates/host/src/pages/mod.rs`: askama base layout + home page, `GET /` 200 public (R3)
- [x] T018 [US1] Wire `crates/host/src/main.rs`: config → storage/session/migrate startup → axum router → `tokio::signal` graceful shutdown (R7); unknown route → 404 envelope
- [x] T019 [US1] Structured logging in `main.rs` (startup/shutdown/error, tracing-subscriber; secrets redacted — FR-009)

**Checkpoint**: US1 independently functional — `cargo test -p host` green, quickstart §2–§3 validated

## Phase 4: User Story 2 — Contracts & Boundary (P2)

**Goal**: shared contract surface + mechanical boundary enforcement + idempotent migrations.
**Independent Test**: introduce an intentional module→module dependency → boundary check fails naming the pair; remove → passes. Run migrations twice on a fresh DB → second run changes nothing.

### Tests (write first, must FAIL)

- [ ] T020 [P] [US2] Boundary negative test in `tests/boundary.rs` (or a script target): a scratch dependency `catalog → enrollment` makes `check-boundaries.sh` exit non-zero naming the pair; clean tree exits 0 (SC-003)
- [ ] T021 [P] [US2] Migration idempotency test in `crates/shared/tests/migrate.rs`: apply all `migrations/` twice against a fresh `MemoryStorage` DB → second apply is a no-op, bookkeeping rows match checksums

### Implementation

- [ ] T022 [US2] Harden `.github/scripts/check-boundaries.sh` if the negative test (T020) exposes gaps (e.g. path-deps / `path =` entries)
- [ ] T023 [US2] Full-environment integration test in `tests/full_env.rs` (gated: skipped — not faked — when `DATABASE_URL` absent; runs migrations + healthz against live MSSQL/Valkey)

**Checkpoint**: `check-boundaries.sh` proves negative detection; migrations idempotent on fresh DB

## Phase 5: User Story 3 — Auth & Session Store (P3)

**Goal**: cookie sessions with org scope, sliding expiry, ephemeral bag — mechanism only.
**Independent Test**: create session → two scoped requests succeed; cross-org request → 403; expired/unknown token → 401; bag survives DB restart only in the store; store flush → bag gone, session alive.

### Tests (write first, must FAIL)

- [ ] T024 [P] [US3] Session tests in `crates/shared/tests/auth.rs` (MemoryStorage): establish → verify across requests; scope mismatch → 403 (story 3.2); sliding extension on activity; expiry after TTL with no activity; unknown token → 401 never 500
- [ ] T025 [P] [US3] Ephemeral bag tests in `crates/shared/tests/session_bag.rs`: set/get with TTL; flush store → only bag lost, session row intact (story 3.4)

### Implementation

- [ ] T026 [US3] Implement `shared/src/auth/mod.rs`: token generation (32-byte, base64url), `token_hash` session row, cookie attributes per contracts/api.md (R4), sliding-expiry logic
- [ ] T027 [US3] Implement `AuthenticatedSession` extractor + `ScopeGuard` (401/403, FR-007 — R5) in `crates/shared/src/auth/`
- [ ] T028 [US3] Implement bag API (set/get/refresh-TTL; `SessionStoreUnavailable` → 503 path, story 1.4) in `crates/shared/src/session/`

**Checkpoint**: all three stories independently functional; full deterministic gate green

## Phase 6: Polish & Cross-Cutting

- [ ] T029 [P] Validate quickstart.md §1–§5 end-to-end on the Linux machine (deterministic path)
- [ ] T030 Full gate sweep on clean checkout: `cargo fmt --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo build --workspace`, `cargo test --workspace`, `check-boundaries.sh` — all green (SC-005, local half; peer runs the Windows half)
- [ ] T031 Update `docs/work-split.md` status (Phase 2 done) + close spec 001 with the completion evidence in the PR body (Principle XIII proof)

## Dependencies & Execution Order

- **Phase 1 (Setup)**: no dependencies — start immediately
- **Phase 2 (Foundational)**: T003–T005 independent; T006 ← T004; T007, T008, T009, T010, T011 independent of each other. BLOCKS all stories.
- **Phase 3 (US1)**: tests T012–T014 first (red); implementation T015→T016/T017 (parallel ok, different files) → T018 → T019
- **Phase 4 (US2)**: independent of US1 after Phase 2; T020/T021 parallel
- **Phase 5 (US3)**: T024/T025 first; T026 → T027/T028 (parallel ok)
- **Phase 6**: after all stories

### Parallel Opportunities

- T003, T004, T005, T008, T009, T010 are [P] (distinct files)
- T012–T014 [P]; T020–T021 [P]; T024–T025 [P]; T027–T028 [P]
- Single implementer per branch (Agent A for this slice): [P] tasks run within the same doloop session; cross-machine parallelism starts at Phase 3 of specs 002–005.

## Implementation Strategy

1. Setup + Foundational → checkpoint (compiles)
2. US1 → checkpoint = **MVP** (health-proven application) → doloop validate
3. US2 → checkpoint (boundary proven negative)
4. US3 → checkpoint (auth mechanism proven)
5. Polish → full gate evidence → PR (Agent B reviews per Principle XVI)
