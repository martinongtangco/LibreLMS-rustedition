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
  (shared DTOs/interfaces) and a `shared` crate (sqlx MSSQL pool, Valkey client, mail,
  auth core).
- **Clean Architecture inside each module** (domain → application → infrastructure),
  deliberately free of ORM/mediator frameworks.
- **Boundaries are compiled, not conventional**: CI fails on module-boundary violations
  (Principle III).
- **Storage**: MSSQL (via sqlx) is the system of record; Valkey holds only the live
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
