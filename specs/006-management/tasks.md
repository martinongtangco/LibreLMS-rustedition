# Tasks: Organization & Learner Management (006)

**Input**: [plan.md](plan.md) | **Spec**: [spec.md](spec.md)
**Branch**: `story/006-management` | Red-first (Principle VII): every test
battery committed RED against its stub before its green.

## Phase 0 — Setup

- [x] **T001** [P] Declare `crates/management/Cargo.toml` deps (contracts, shared, tiberius, tokio, serde/serde_json, async-trait — all workspace) and wire `management` into `crates/host/Cargo.toml`. *Check: workspace builds; boundary script clean.*

## Phase 1 — US1: learner management (P1)

- [x] **T002** [P] [US1] RED: `crates/management/tests/repo.rs` — create-in-org persists (email, name, role, placeholder-hash convention); list-own-org returns only the actor-org's users with roles; re-role persists (admin→member); remove deletes ONLY the membership (user row survives, still present in other orgs); duplicate email (sequential) → `DuplicateEmail` (LOGIC-LAYER label); cross-org user op → `Scope` (CR-3: unknown user id ALSO → `Scope`, never `NotFound`); junk role → `Validation` (CR-1).
- [ ] **T003** [P] [US1] RED: `crates/host/tests/management_flow.rs` — `POST /admin/users` 401 without session; 201 in-scope admin; 403 `forbidden` member; 403 `scope` cross-org; 409 `duplicate-email`; 400 naming the field (incl. role — CR-1); `PUT /admin/users/{id}/role` 200 + `GET /admin/users` reflects it; `DELETE /admin/users/{id}` 204 (membership gone, user survives); 503 on `Unavailable` (never 500, never silent).
- [x] **T004** [US1] Implement `ManagementRepo` trait + `ManagementError` (Scope/Forbidden/NotFound/DuplicateEmail/NotEmpty/Validation/Unavailable) + `InMemoryManagementRepo` (double + fault injection) + validation (FR-012 incl. role — CR-1) until T002/T003 green.
- [ ] **T005** [US1] Implement `TiberiusManagementRepo` (wire-edge CASTs per 001 convention; 2601 **and** 2627 → `DuplicateEmail` on BOTH error legs; CHECK-violation 2627-class → `Validation` (CR-1), never 500; re-role upsert; remove = membership row only) until the repo double/real parity holds.
- [ ] **T006** [US1] Wire `management::routes` (learner endpoints) into the host (one-line-mount convention, docs 767d690) with role-gate + org-scope-gate + the error→envelope mapping until T003 green.

## Phase 2 — US2: organization management (P2)

- [ ] **T007** [P] [US2] RED: `crates/management/tests/repo.rs` — create-org 201-shape (id, name; 001 (id,name) only — no description); list with user counts; rename persists; delete of a non-empty org → `NotEmpty`, nothing deleted; delete of an empty org succeeds AND cascade-expires the org's sessions (CR-2b: session rows gone, other orgs' sessions untouched); unknown org id → `NotFound` (the 404 row is org-ids-only — CR-3).
- [ ] **T008** [P] [US2] RED: `crates/host/tests/management_flow.rs` — `GET/POST/PUT/DELETE /admin/organizations[...]` 401/403 `forbidden` (role-gate, BEFORE any id check)/201/200/204/409 `not-empty`/404 `not-found`/503.
- [ ] **T009** [US2] Implement the organization methods on both repos (list-with-counts; delete guard on MEMBERSHIPS; session cascade ordered before the org row delete — CR-2b) until T007 green.
- [ ] **T010** [US2] Wire the organization routes (role-gated; the 404 row org-ids-only) until T008 green.

## Phase 3 — US3: robustness (P3) + real engine

- [ ] **T011** [P] [US3] RED: `crates/management/tests/validation.rs` — the FR-012 battery (email ≤320, name ≤200, org-name ≤200, role ∈ {admin,member} — CR-1; 400 naming the field); injection-shaped email/name/org-name stored and compared as plain text; 503 fault-injection battery (never a silent empty list).
- [ ] **T012** [US3] Implement the validation wiring (app-layer FIRST; the engine CHECK/UNIQUE are the belt — CR-1/CR-2 mapping never a 500) until T011 green.
- [ ] **T013** Gated full-environment test `crates/host/tests/management_full_env.rs` (self-skips visibly; NEVER a silent pass): (a) the duplicate-email RACE PROOF — N concurrent duplicate creates over N INDEPENDENT `TiberiusManagementRepo` instances (one connection each) asserting exactly ONE user row + losers' stable `duplicate-email` codes (the 004 CR-1 lesson); (b) the empty-manifest run is a no-op (the runner's `&[]` contract on the real engine); (c) the org-delete session cascade on the real engine (a live 001 session for the deleted org is gone; a foreign org's session survives); (d) the never-500 battery (CHECK/UNIQUE violations → stable codes); (e) durability across a fresh client reconnect.
- [ ] **T014** Full gate sweep on a clean checkout: `cargo fmt --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo build --workspace`, `cargo test --workspace --no-fail-fast`, `check-boundaries.sh` — all green; the full-env test's deciding run is the reviewing machine's fresh-container session (Principle XVI — a SKIP is not a pass).
- [ ] **T015** Update `docs/work-split.md` status (Phase 3 Management done) + PR body with the Principle XIII evidence (gate outputs + the fresh-container full-env run result + the duplicate-email race proof).

## Dependencies & Parallelism

- **Phase 1**: T001 first; T002/T003 [P] red-first → T004 → T005 → T006
- **Phase 2**: T007/T008 [P] red-first → T009 → T010; independent of US1 after T001
- **Phase 3**: T011 [P] red-first → T012 → T013 → T014 → T015
- [P] tasks: T001, T002, T003, T007, T008, T011 (distinct files)

## Compliance Notes

1. Red-first per task (Principle VII): RED commit → green commit, per battery.
2. `contracts`/`shared` byte-identical before/after (frozen post-001).
3. No new dependencies (Principle XVIII) — workspace only.
4. The full-env test self-skips visibly; the deciding run is the reviewing machine's (Principle XVI/XVII).
5. Polish → full gate evidence + the duplicate-email race proof on the real engine → PR (Agent B reviews per Principle XVI; the deciding full-env run is theirs to perform).
