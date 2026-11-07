# Tasks: Course Catalog (003)

**Input**: spec + plan. Tests are written FIRST (Principle VII); each task names its
deterministic check. Loop check per iteration:
`cargo test -p catalog --tests && cargo test -p host --test catalog_flow && cargo clippy --workspace --all-targets -- -D warnings && cargo fmt --check && bash .github/scripts/check-boundaries.sh`

## Foundational (P0)

- [ ] **T001** Add `catalog` crate deps (contracts, shared; tiberius via workspace) + `pub const MIGRATIONS` in `catalog::migrations` exposing `0003_catalog.sql` (courses table + org index); host concatenates the catalog manifest (seam from spec 001 CR-2). *Check: workspace builds; boundary script clean.*
- [ ] **T002** Write `0003_catalog.sql` (idempotent `IF OBJECT_ID IS NULL CREATE TABLE` pattern, whole-script batch per spec 001 CR-E). *Check: script executes as one batch on live MSSQL in the full-env run; idempotent second run.*

## User Story 1 — scoped listing + filter (P1)

- [ ] **T003** (test-first) `crates/catalog/tests/repo.rs`: `InMemoryCourseRepo` double + tests: list returns exactly the session org's courses (SC-004 fixture); case-insensitive substring filter — 100% precision/recall on a fixture battery (SC-002); empty org → empty list, not error; empty search = full list. **RED before double exists.**
- [ ] **T004** Implement `InMemoryCourseRepo` + `CourseRepo` trait (list(title_filter), get, create, update, delete) until T003 green.
- [ ] **T005** (test-first) `crates/catalog/tests/scope.rs`: cross-org fixture (2 orgs) — zero visibility across orgs on list and get (SC-004); get of other-org course → `Scope` (403 semantics), never `NotFound` (no existence oracle, spec story 3.2).
- [ ] **T006** Implement scope rules in `repo.rs` (`CatalogError::Scope/Forbidden/NotFound`) until T005 green.
- [ ] **T007** (test-first) `crates/host/tests/catalog_flow.rs`: `GET /courses` — 401 envelope without session; 200 list for in-scope member; `?title=` filter through the HTTP surface; 503 envelope when the repository reports `Unavailable`. **RED before routes exist.**
- [ ] **T008** Wire `catalog::routes` into the host router (state carries the repo) until T007 green.

## User Story 2 — admin CRUD (P2)

- [ ] **T009** (test-first) `repo.rs` create/update/delete tests: create persists + appears in list (SC-003); update visible immediately; delete removes; delete-then-get → `NotFound`.
- [ ] **T010** Validation tests (SC-005 boundary battery): title empty / 200 / 201 chars; description 5,000 / 5,001; rejection names the field; zero invalid rows stored (double asserts).
- [ ] **T011** Implement create/update/delete + validation in the double-backed trait path until T009/T010 green.
- [ ] **T012** (test-first) `catalog_flow.rs` write-path tests: `POST /courses` 201 + visible on GET; 400 field-named for bad input; 403 non-admin on POST/PUT/DELETE; `PUT/DELETE /courses/{id}` 200/204 + 404 unknown id.
- [ ] **T013** Wire POST/PUT/DELETE routes (role + scope gates) until T012 green.

## User Story 3 — permissions & scope enforcement (P3)

- [ ] **T014** (test-first) `scope.rs` permission matrix (spec story 3): member write → `Forbidden`; admin cross-org write → `Scope`; unknown id → `NotFound`; no-leakage assertion on error bodies (no backend internals, no other-org data).
- [ ] **T015** Implement the matrix in `repo.rs` until T014 green.
- [ ] **T016** (test-first) Concurrency/robustness tests: last-write-wins on double (two updates, both succeed, no corruption); injection-shaped search terms treated as plain text (no error).

## Real-engine (Principle XIII/XVI — gated, Agent B deciding)

- [ ] **T017** Implement `TiberiusCourseRepo` (parameterized queries, `dbo.courses`, timestamp conventions per spec 001) + `tests/catalog_full_env.rs` (gated: skips visibly without env; with env — migrate `0003`, CRUD round-trip, second-migration no-op, cross-org 403 on the real engine).
- [ ] **T018** Wire the real repo into `host::main` (in-memory only in tests; real in production path).

## Final (P4)

- [ ] **T019** Boundary check: extend the negative test's pair list with `catalog` (it is a module crate — already covered by the script; verify `catalog -> enrollment` detection in `crates/host/tests/boundary.rs`-style check or the script directly). *Check: script detects an injected `catalog`→`enrollment` dep.*
- [ ] **T020** README foundation table: add the Catalog row (routes + repo + migrations); spec 003 checklist final pass.
- [ ] **T021** Full gate run (deterministic + fmt + clippy + boundary); push `story/003-course-catalog`; open PR to Agent B with the fresh-container full-env run request.

## Dependencies

P0 (T001-T002) → P1 (T003-T008) → P2 (T009-T013) → P3 (T014-T016) → real-engine (T017-T018, parallelizable with P3) → P4 (T019-T021).

**Parallel-safe**: T017/T018 may start once T001/T002 land (they depend only on the
migration + trait shape).
