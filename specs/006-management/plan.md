# Implementation Plan: Organization & Learner Management (006)

**Branch**: `story/006-management` | **Date**: 2026-07-08 | **Spec**: [spec.md](spec.md)

## Summary

The Management slice exposes admin learner management (create / list / re-role /
remove-membership) and flat organization management (create / list+counts /
rename / delete-with-409-guard + session-cascade) over the spec 001
foundation: an in-crate repository trait + tiberius implementation +
in-memory double (the 001 Module Persistence Seam), axum routes on the host
with the 001-style envelopes, **no new DDL** (all touched tables are
foundation-owned; `MIGRATIONS` is `&[]` by construction), **no cross-module
dependencies**, and no new dependencies beyond the workspace.

## Technical Context

**Language/Version**: Rust 1.99.1, pinned via `rust-toolchain.toml`
(gates toolchain-agnostic; machine B runs the per-directory gnu override —
001 CR-3).

**Primary Dependencies**: tiberius (workspace, ADR 0003), axum 0.8, tokio,
serde/serde_json, async-trait — **all from the workspace; no new
dependencies** (Principle XVIII).

**Storage**: MSSQL only (006 touches nothing Valkey-worthy — Principle VI).
The module performs DML against foundation-owned tables (`dbo.users`,
`dbo.user_organizations`, `dbo.organizations`, and `dbo.sessions` for the
CR-2b delete-cascade) through its OWN repository — the same standing 001's
auth uses for `dbo.sessions`. **No DDL in this slice** (FR-013): `pub const
MIGRATIONS: &[(&str, &str)] = &[]`, documented; the manifest-generic runner
handles the empty manifest by construction (001 CR-2).

**Wire-edge discipline (the 003/004 hard-won rules)**:
- `dbo.users.id` / `dbo.organizations.id` / `dbo.user_organizations.*` are
  UNIQUEIDENTIFIER in 001's DDL — read via `CAST(col AS NVARCHAR(40))` at
  the SELECT edge, write via `CAST(@P AS UNIQUEIDENTIFIER)` (the 001 proven
  pattern; tiberius 0.13 cannot read a Guid column as `&str`).
- Duplicate email: map BOTH 2601 (general) and 2627 (indexed) to
  `DuplicateEmail` — and map the violation on BOTH error legs (the `query()`
  future and the `into_row()` stream consumption).
- The CHECK constraint on `dbo.user_organizations.role` is the belt: a
  2627-class CHECK violation maps to the 400 `validation`, never a 500
  (CR-1).

**Session/Auth**: the 001 session mechanism; role gate (admin-only) via the
session's role; org-scope gate for user operations (CR-3: unknown or
other-org user ids answer 403 `scope`, never 404 — the 003 US3.2
no-existence-oracle parity; 404 is org-ids-only per FR-010).

**Performance Goals**: teaching slice — single-query lists; no throughput
targets.

**Constraints**: no cross-module crate dependencies (boundary check + negative
test); `contracts`/`shared` untouched (frozen post-001); whole-script
migration batches (N/A — no DDL); no secrets in logs (001 FR-009).

## Constitution Check

*GATE: must pass before implementation. Checked against constitution v1.0.1 (actual principle names/numbers).*

| # | Principle (v1.0.1) | Status | Notes |
|---|---|---|---|
| I | Modular Monolith, Not Microservices (Yet) | ✅ PASS | one `host` binary; the management module composes via its public API |
| II | Clean Architecture, Applied Simply | ✅ PASS | the repository trait is the single seam; no further pattern wrapping |
| III | Module Boundaries Are Compiled | ✅ PASS | depends on `contracts` + `shared` only; boundary negative test unchanged |
| IV | Human-Legible AI-Authored Code | ✅ PASS | the empty-manifest decision + the session-cascade order are recorded here with one-sentence rationale |
| V | Parallel Two-Agent Protocol (NON-NEGOTIABLE) | ✅ PASS | Management = Agent A lane; `story/006-management`; planning on main; `contracts`/`shared` untouched |
| VI | Polyglot Storage With a Reason | ✅ PASS | users/memberships/orgs/sessions are durable → MSSQL only; nothing in Valkey |
| VII | Spec-Driven, Sliced Thin | ✅ PASS | spec 006 on `main` (CRs folded, 5c53ddf) before planning; thin vertical slice (learners → orgs → robustness) |
| VIII | Branching Discipline | ✅ PASS | implementation on `story/006-management` |
| IX | Plan On Main Only | ✅ PASS | this plan authored on `main` |
| XI | Two-Machine Parallelism | ✅ PASS | host mount via the documented one-line-mount + rebase convention (docs 767d690) |
| XIII | Verification Before Claim | ✅ PASS | red-first tasks; the real-engine proof is a real run on the reviewing machine (never a self-skip) |
| XVI | Cross-Agent Review for Merge (NON-NEGOTIABLE) | ✅ PASS | Agent B reviews spec (done — 5c53ddf)/plan/tasks, then the PR; the full-env gate run is the reviewing machine's |
| XVII | Verify Against a Disposable Environment | ✅ PASS | tests create their own rows; the gated full-env test self-skips visibly, the deciding run is real |
| XVIII | Rust Hygiene | ✅ PASS | `rust-toolchain.toml` pin, clippy `-D warnings`, fmt, no `unwrap`/`expect` outside tests |

**Gate result: PASS — no violations; Complexity Tracking not required.**

## Key Design Decisions (Principle IV — one sentence each)

1. **Empty manifest**: `MIGRATIONS = &[]` because every table this slice
   writes is foundation-owned DDL (0001) and a module must not ALTER
   foundation tables — the runner's manifest-generic contract makes the
   empty manifest a first-class, tested case.
2. **Delete order (CR-2b)**: a successful org delete runs
   `DELETE dbo.sessions WHERE organization_id = @org` BEFORE
   `DELETE dbo.organizations WHERE id = @org` — the sessions FK is
   satisfied by construction, and a deleted org cannot hold a live session;
   the 409 `not-empty` guard stays membership-defined (sessions are the
   delete's consequence, not its blocker).
3. **Re-role / remove semantics**: re-role is an upsert on
   `dbo.user_organizations`; remove deletes ONLY the membership row (the
   user row survives — multi-org users, FR-004).
4. **Duplicate email**: the engine's UNIQUE constraint is the final
   arbiter under concurrency; the deterministic duplicate test is
   LOGIC-LAYER; the real-engine proof (gated full-env) fires N concurrent
   duplicate creates over N INDEPENDENT connections (one per repo instance —
   the 004 CR-1 lesson) asserting exactly one user row + losers' stable
   `duplicate-email` codes.
5. **Role gate vs scope gate**: organization operations are role-gated
   (platform admin capability — the original's Admin portal shape); user
   operations are org-scope-gated (FR-005).

## Project Structure

```text
crates/management/
├── Cargo.toml          # depends: contracts, shared, tiberius (no other modules)
└── src/
    ├── lib.rs          # pub routes + MIGRATIONS (= &[], documented) + re-exports
    ├── repo.rs         # ManagementRepo trait, ManagementError, validation, scope/role rules
    ├── memory.rs       # InMemoryManagementRepo (deterministic double + fault injection)
    ├── mssql.rs        # TiberiusManagementRepo (wire-edge CASTs, 2601/2627 + CHECK mapping, session cascade)
    └── migrations.rs   # pub const MIGRATIONS: &[(&str, &str)] = &[]  (empty by construction)
host: management::routes mounted per the one-line-mount convention (docs 767d690)
tests: crates/management/tests/{repo.rs, scope.rs, validation.rs}
       crates/host/tests/{management_flow.rs, management_full_env.rs (gated: N-connection
       duplicate-email proof + idempotency + durability + never-500 battery)}
```

**Structure Decision**: the 7-crate workspace is kept exactly (work-split
contract, Principle V); this slice fills `management` only — `contracts`/
`shared` untouched (frozen post-001), no other module touched.

## Complexity Tracking

Not required — Constitution Check passed with no violations.
