# Implementation Plan: Application Foundation

**Branch**: `story/001-foundation` | **Date**: 2026-07-22 | **Spec**: [spec.md](spec.md)

**Input**: Feature specification from `/specs/001-foundation/spec.md`

## Summary

The foundation slice delivers the application lifecycle (config → startup → health →
shutdown), the system-of-record access (tiberius against MSSQL — ADR 0003), the ephemeral session store
(redis-rs against Valkey), the auth mechanism (cookie sessions with organization scope),
the shared contract crate, the boundary check, and the full CI gate set. Technical
approach: an axum host crate composing a `shared` infrastructure crate (storage traits +
tiberius/in-memory implementations, session store, mail, auth) and a `contracts` crate
(typed shapes). No module business logic.

## Technical Context

**Language/Version**: Rust 1.99.1 stable, pinned via `rust-toolchain.toml` (Principle XVIII)

**Primary Dependencies** (all already declared in the workspace `Cargo.toml`):
`axum` 0.8, `tower`/`tower-http`, `tiberius` 0.13 (tokio feature — ADR 0003), `redis` 0.27 (tokio-comp),
`askama` 0.12, `cookie` 0.18, `tokio`, `serde`/`serde_json`, `tracing`/`tracing-subscriber`

**Storage**: MSSQL (system of record, via tiberius — ADR 0002/0003); Valkey (Redis-protocol, via
redis-rs) for ephemeral per-session state only (Principle VI); in-memory implementation
behind the storage trait for deterministic local/CI tests (Principle XIII/XVII)

**Testing**: built-in `#[test]` + `tokio::test`, `mockall` where needed; deterministic
check = `cargo test -p <crate> && cargo clippy -p <crate> --all-targets -- -D warnings`;
full-environment (MSSQL/Valkey) integration suite runs on a machine with the services or
in CI (Principle XIII/XVI)

**Target Platform**: Linux (dev machine A) and Windows (dev machine B — per-directory
rustup override `1.99.1-x86_64-pc-windows-gnu`; MSVC is unusable on that box: partial
VS 2022/BuildTools installs, missing toolset lib/x64, no Win10 SDK. The gates —
build/test/clippy/fmt — are toolchain-agnostic, and the `1.99.1` pin in
`rust-toolchain.toml` is honored by the override.) ; no
cross-compilation in this slice

**Project Type**: web service (modular monolith — ADR 0001)

**Performance Goals**: not a performance slice; startup < 30 s, fail-fast startup < 5 s
(spec SC-001/SC-002); no throughput targets

**Constraints**: no `unwrap`/`expect` outside tests (Principle XVIII); no new dependency
without a plan/ADR entry (Principle XVIII); secrets never logged (FR-009); clean build on
both machines (SC-005)

**Scale/Scope**: 7-crate workspace; base schema = users, organizations, memberships,
sessions; 1 seeder user; no business behavior

## Constitution Check

*GATE: Must pass before Phase 0 research. Re-check after Phase 1 design.*

| # | Principle | Status | Notes |
|---|-----------|--------|-------|
| VII | Spec-Driven, Sliced Thin | ✅ PASS | spec 001 exists on `main` before planning |
| VI | Polyglot Storage With a Reason | ✅ PASS | MSSQL = system of record; Valkey = ephemeral session state only |
| I | Modular Monolith | ✅ PASS | one `host` binary; module crates independent (ADR 0001) |
| II | Clean Architecture, Applied Simply | ✅ PASS | `shared` holds the storage traits; no ORM wrapper layers |
| III | Module Boundaries Compiled | ✅ PASS | `.github/scripts/check-boundaries.sh` in CI + locally |
| XVIII | Rust Hygiene | ✅ PASS | `rust-toolchain.toml` pin, `clippy -D warnings`, `fmt --check` in gates |
| VIII | Branching Discipline | ✅ PASS | implementation on `story/001-foundation` |

**Gate result: PASS — no violations; Complexity Tracking not required.**

Post-design re-check (after Phase 1 artifacts): still PASS — the in-memory storage
implementation is a test double behind the `Storage` trait, not a new storage system
(Principle VI respected: nothing durable lives outside MSSQL).

## Module Persistence Seam (decided at foundation baseline — CR-2)

Module slices persist their own rows and must do so without new shared surface (shared
is read-only ground truth after the foundation merge):

1. Each module crate owns: (a) a **domain repository trait defined in the module crate**,
   (b) the real `tiberius` implementation against its own `dbo.*` rows, (c) an
   **in-memory test double** in the same crate, (d) its **migrations `NNNN_*.sql`**
   exposed as `pub const MIGRATIONS: &[(&str, &str)]`.
2. The startup migration runner is **manifest-generic**: `Storage::run_migrations` takes
   a manifest; the host concatenates `shared::migrate::MIGRATIONS` with every module's
   `MIGRATIONS` (name-ordered). This signature change lands in the foundation, pre-merge.
3. **Deterministic tests use the in-memory doubles.** Real-SQL verification runs through
   **gated full-environment tests** (the `full_env.rs` pattern, per module), executed on
   Agent B's Docker machine (Principle XIII) and in CI when services are added.
4. A sqlx SQLite substitute is **not available under ADR 0003** (tiberius is MSSQL-only;
   sqlx 0.8 has no MSSQL feature) — the repository pattern replaces it. No new shared
   surface is introduced.

## Project Structure

### Documentation (this feature)

```text
specs/001-foundation/
├── plan.md              # This file
├── research.md          # Phase 0 output
├── data-model.md        # Phase 1 output
├── quickstart.md        # Phase 1 output
├── contracts/           # Phase 1 output
│   └── api.md           # health/auth/contract surface contracts
└── tasks.md             # Phase 2 output (/speckit.tasks — NOT created here)
```

### Source Code (repository root)

```text
crates/
├── contracts/            # shared typed shapes (User, Organization, Course, Enrollment,
│   └── src/lib.rs      # Completion, ScormAttemptStatus) — no dependencies beyond serde
├── shared/
│   └── src/
│       ├── lib.rs        # re-exports
│       ├── config.rs     # env config parsing + fail-fast validation (FR-001)
│       ├── storage/
│       │   ├── mod.rs    # Storage trait (async) + SessionStore trait
│       │   ├── mssql.rs # tiberius MSSQL implementation + startup migration runner
│       │   └── memory.rs # in-memory implementation (deterministic tests)
│       ├── session/
│       │   └── mod.rs    # Valkey session store (ephemeral, TTL, FR-008)
│       ├── auth/
│       │   └── mod.rs    # cookie session establishment, org-scope enforcement (FR-007)
│       ├── mail/
│       │   └── mod.rs    # Mailer trait + dev outbox (FR-010)
│       └── migrate/
│           └── mod.rs    # ordered SQL scripts, idempotent runner (FR-003)
├── catalog/  enrollment/  scorm/  management/   # unchanged stubs (later slices)
└── host/
    └── src/
        ├── main.rs       # config → startup → axum router → graceful shutdown (FR-001)
        ├── health.rs     # JSON health: alive + db + session store (FR-002)
        ├── pages/
        │   └── mod.rs    # askama layout + home page (FR-002)
        └── error.rs      # unified error → 404/401/403/503 mapping, no stack traces (spec edge cases)
migrations/               # ordered .sql scripts (idempotent; run at startup)
tests/                    # full-environment integration (marked, run when services present)
docker-compose.yml        # mssql + valkey for full-environment runs (peer machine / CI)
```

**Structure Decision**: the existing 7-crate workspace is kept exactly (work-split contract,
Principle V); this slice fills `host` + `shared` + `contracts` and leaves the four module
crates as stubs for Phases 3–4.

## Complexity Tracking

Not required — Constitution Check passed with no violations.

## Amendments
- 2026-07-24: `rand 0.9` added (US3 token generation — CSPRNG-quality session tokens; std has no RNG). Listed per Principle XVIII.
