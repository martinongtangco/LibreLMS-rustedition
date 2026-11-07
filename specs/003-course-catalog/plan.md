# Implementation Plan: Course Catalog (003)

**Input**: `specs/003-course-catalog/spec.md`

## Summary

The Catalog slice exposes the course catalog over the spec 001 foundation: an in-crate
repository trait + tiberius implementation + in-memory double (the spec 001 Module
Persistence Seam), its own `NNNN_*.sql` migrations exposed as `pub const MIGRATIONS`,
and axum routes on the host — list/filter, course CRUD, org-scope + role enforcement —
with 401/403/404/503 envelope behavior and no new shared surface.

## Technical Context

**Language/Version**: Rust (pinned 1.99.1, `rust-toolchain.toml`)
**Primary Dependencies**: tiberius 0.13 (ADR 0003), axum 0.8, askama 0.12 (manual
`IntoResponse` adapter — spec 001 R3), tokio 1, serde/serde_json, tracing, rand 0.9
(no new dependencies beyond the workspace; Principle XVIII)
**Storage**: MSSQL (system of record) via the spec 001 `Storage`/tiberius seam for
foundation tables; the Catalog slice owns its **own** `dbo.courses` table + repository
(in-crate trait + real impl + in-memory double), per the spec 001 Module Persistence
Seam decision. Deterministic tests = the in-memory double; real-SQL verification = the
gated full-environment test (Principle XIII/XVI, Agent B's machine).
**Session/Auth**: the spec 001 `Authed` extractor (cookie token → identity + org scope +
role); resource-level scope check: `course.organization_id != session.organization_id →
403 envelope`.
**Target Platform**: Linux (machine A) + Windows (machine B, gnu override) — gates
toolchain-agnostic.
**Performance Goals**: 1,000-course list + substring filter < 2 s (SC-001/SC-002).
**Constraint**: no cross-module dependencies (boundary check, spec 001 US2); no new
shared surface (shared frozen post-001); no secrets in logs (spec 001 FR-009).

## Constitution Check

*Gate: pass/fail against the constitution (v1.0.1).*

| Principle | Verdict |
|---|---|
| I. Spec-Driven Development | **PASS** — spec + checklist committed on main before planning; tasks trace to FRs. |
| II. Simplicity / YAGNI | **PASS** — one table, one repository trait, no pagination/caching/locking layer; last-write-wins. |
| III. Module Boundaries via Contracts | **PASS** — catalog consumes `contracts` + `shared` only; the repository trait lives in the `catalog` crate; boundary check extended negative-test style. |
| IV. ADRs for decisions | **PASS** — no new architecture decisions beyond the spec 001 seam; any deviation gets an ADR. |
| V. Parallel Two-Agent Protocol | **PASS** — Catalog = Agent A lane; `story/003-course-catalog` branch; planning on main. |
| VI. MSSQL system of record / Valkey ephemeral | **PASS** — courses persist to MSSQL only; no catalog state in Valkey. |
| VII. Tests First | **PASS** — tasks are red-first (double + repository + HTTP behavior before implementation). |
| VIII. Deterministic Gates | **PASS** — `cargo test -p catalog` + host route tests are container-free; full-env test gated. |
| IX. Fail-Fast Configuration | **PASS** — reuses spec 001 `Config`. |
| X. Structured Logging, No Secrets | **PASS** — reuses spec 001 error envelopes; catalog error paths name fields, not backends. |
| XI. Migrations Idempotent + Bookkept | **PASS** — `0003_catalog.sql` through the manifest-generic runner; idempotency test on the real engine in the full-env run. |
| XII. Graceful Shutdown | **PASS** — no new long-lived state beyond the repository (no pool layer). |
| XIII. Full-Environment Verification | **PASS** — `catalog_full_env` gated test; Agent B's fresh-container run is the deciding one. |
| XIV. Spec/Plan/Task Traceability | **PASS** — FR-001..FR-009 → tasks; SC-001..SC-005 → acceptance tests. |
| XV. Spec-Kit on Main, Code on Branches | **PASS** — spec/plan/tasks on main; code on `story/003-course-catalog`. |
| XVI. Cross-Agent Review for Merge | **PASS** — Agent B reviews spec/plan then the PR; author does not self-approve. |
| XVII. No Faked Passes | **PASS** — full-env test self-skips visibly when env absent; the deciding run must be a real run (Agent B). |
| XVIII. Rust Hygiene | **PASS** — no `unwrap`/`expect` outside tests; clippy `-D warnings`; fmt; boundary script. |

**Result: PASS (18/18)** — no amendments required.

## Project Structure

```
crates/catalog/
├── Cargo.toml          # depends: contracts, shared (no other modules)
└── src/
    ├── lib.rs          # pub routes(Router) + module migration manifest + repo
    ├── repo.rs         # CourseRepo trait (in-crate), CatalogError, scope/role rules
    ├── memory.rs       # InMemoryCourseRepo (deterministic double)
    ├── mssql.rs         # TiberiusCourseRepo (real dbo.courses)
    └── migrations/     # 0003_catalog.sql (manifest-exposed const)
crates/host/
└── src/lib.rs          # + /courses routes wired to catalog::routes (state carries repo)
tests (crates/catalog/tests/)   # repo + scope/permission matrix (in-memory double)
crates/host/tests/catalog_flow.rs  # 401/403/404/503/200 HTTP surface
tests/catalog_full_env.rs        # gated: real tiberius repo + migrations on live MSSQL
```

**Structure Type**: single module crate + host wiring (modular monolith, ADR 0001).

## Data Model

```
dbo.courses (module-owned, 0003_catalog.sql)
  id              UNIQUEIDENTIFIER PK
  organization_id UNIQUEIDENTIFIER NOT NULL  (FK→ organizations)
  title           NVARCHAR(200)  NOT NULL
  description     NVARCHAR(5000) NULL
  created_at      DATETIME2 NOT NULL
  updated_at      DATETIME2 NOT NULL
  INDEX idx_courses_org (organization_id)

CatalogError = NotFound | Scope | Forbidden | Validation{field} | Unavailable{—}
   (→ 404 / 403 / 403 / 422-class 400 with field-named message / 503 envelopes)
```

Contracts consumed, none added: `Course` shape already in `contracts` (title,
description, organization, id) — the slice populates it.

## Contracts

- `GET /courses?title=<substring>` → 200 `[Course]` (scoped to session org; empty
  allowed) — `X-Organization` from the session; 401 envelope unauthenticated.
- `POST /courses` (admin only) → 201 `Course` / 400 field-named / 403 non-admin.
- `PUT /courses/{id}` (admin only) → 200 `Course` / 404 / 403 scope / 403 role.
- `DELETE /courses/{id}` (admin only) → 204 / 404 / 403 scope / 403 role.
- Scope rule (resource-level): `course.organization_id != session.organization_id → 403`
  envelope (spec 001 contracts); `course id not found → 404` envelope (no existence
  oracle: out-of-scope reads answer 403, not 404 — spec US3/003 story 3.2).
- Role rule: `role == Admin` for write; read = any in-scope member.

## Quickstart (deterministic half)

```bash
cargo test -p catalog --tests            # red-first: double + matrix, no containers
cargo test -p host --test catalog_flow   # HTTP surface via the in-memory double
cargo clippy --workspace --all-targets -- -D warnings && cargo fmt --check
bash .github/scripts/check-boundaries.sh # catalog stays module-clean
```
Full-environment half (Agent B, fresh containers): `cargo test --test catalog_full_env`.

## Complexity Tracking

No constitution violations. No new dependencies (Principle XVIII satisfied: nothing added
beyond the workspace set).
