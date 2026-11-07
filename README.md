# Libre LMS — Rustedition

A Rust port of [LibreLMS](https://github.com/martinongtangco/LibreLMS) (course catalog,
SCORM launch, per-user completion) — built as a hands-on exercise in two things:

1. **Spec-driven development** with [GitHub's spec-kit](https://github.com/github/spec-kit)
   (`/speckit.specify → /speckit.plan → /speckit.tasks → /speckit.implement`) on a Rust
   rewrite.
2. **Two-agent parallel implementation** — two coding agents on two machines (Linux +
   Windows) implement the codebase in parallel and keep each other honest through
   **alternating PR reviews** (Principle XVI of the constitution).

Read `.specify/memory/constitution.md` first — it's the source of truth for *why* things
are structured the way they are. The short version and the reasoning are in `docs/adr/`.

## Architecture at a glance

- **Modular monolith**: one binary (`crates/host`, axum) composed of independent module
  crates (`catalog`, `enrollment`, `scorm`, `management`) plus a `contracts` crate
  (shared DTOs/interfaces) and a `shared` crate (tiberius MSSQL pool, Valkey client, mail,
  auth core).
- **Clean Architecture inside each module** (domain → application → infrastructure),
  deliberately free of ORM/mediator frameworks.
- **Boundaries are compiled, not conventional**: CI fails on module-boundary violations
  (Principle III).
- **Storage**: MSSQL (via tiberius) is the system of record; Valkey holds only the live
  SCORM session bag for an in-progress attempt.

## The work split

See `docs/work-split.md` — the contract between the two agents (Principle V).

## Running this locally

### 1. Toolchain

`rust-toolchain.toml` pins the stable toolchain; `rustup` handles the rest:

```bash
rustup show   # picks up rust-toolchain.toml automatically
```

### 2. Build and test (deterministic subset)

```bash
cargo build --workspace
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
```

### 3. Full environment (MSSQL + Valkey)

`docker compose up` brings up `mssql` and `valkey`; the host reads its database
connection from `DATABASE_URL` and Valkey from `VALKEY_URL`. No connection strings are
committed to the repo — see `.env.example`.

### 4. Run the host

```bash
cargo run -p host -- --port 5000
```

## Agent workflow (both machines)

1. Read the constitution.
2. `/speckit.specify` → `/speckit.plan` → `/speckit.tasks` on `main`.
3. `story/` or `bug/` branch, implement with `doloop` (act → verify → record).
4. PR → the **other** agent reviews (runs the gates on their own machine) → author merges.

## Foundation (spec 001) — status: complete, pending cross-agent review

| Piece | Where | Notes |
|---|---|---|
| Workspace | `Cargo.toml` | 7 crates: host + 4 module stubs + shared + contracts |
| Config | `shared::config` | fail-fast, names every invalid item, no secrets logged |
| Storage | `shared::storage` | `Storage` trait + `TiberiusStorage` (MSSQL, ADR 0003) + `MemoryStorage` test double |
| Migrations | `shared::migrate` + `migrations/*.sql` | idempotent, `schema_migrations` bookkeeping, checksummed |
| Sessions | `shared::session` | `SessionStore` trait + `ValkeySessionStore` + `MemorySessionStore` |
| Auth | `shared::auth` | opaque cookie tokens, hashed in DB, org scope, sliding expiry (bounded) |
| Host | `crates/host` | axum 0.8: `/`, `/healthz`, `/me`; 401/403/503 envelope; graceful shutdown |

**Run** (full environment, see `specs/001-foundation/quickstart.md`):
```bash
cp .env.example .env          # set real DATABASE_URL / VALKEY_URL
docker compose up -d          # MSSQL + Valkey (dev only)
cargo run -p host
# → GET /healthz 200 {"status":"ok"} once the schema is ready
```
**Deterministic suite** (no containers): `cargo test --workspace` — 40+ tests, all mock-backed.

**Gates (CI + doloop check)**: build · clippy `-D warnings` · fmt · boundary check (module crates must not depend on each other — enforced by `.github/scripts/check-boundaries.sh`, negative-tested in `crates/host/tests/boundary.rs`).

**Constitution**: `.specify/memory/constitution.md` (v1.0.1 — v1.0.1: ADR 0003 replaces the sqlx entry).
**Work split**: `docs/work-split.md`. **ADRs**: `docs/adr/`.
