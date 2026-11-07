# Research: Application Foundation (Phase 0)

Resolves the unknowns from the plan's Technical Context. No open NEEDS CLARIFICATION
markers — all findings below are decisions or documented risks.

## R1. sqlx migrations with MSSQL

**Unknown**: does sqlx's built-in migrator support MSSQL?

**Finding**: No — `sqlx migrate` supports PostgreSQL, MySQL, and SQLite only. MSSQL has
no first-class migrator in sqlx 0.8.

**Decision**: a small startup migration runner in `shared::migrate`: ordered `.sql`
scripts from `migrations/`, applied at startup inside a transaction where the engine
supports it (MSSQL DDL is transactional for most statements), with a `schema_migrations`
bookkeeping table (script name, checksum, applied_at). Idempotent by construction: a
second run skips already-applied scripts (spec User Story 2, scenario 3). Scripts are
checked in; the runner never auto-generates DDL.

**Risk**: DDL batches that are not transactional on MSSQL (e.g. some `ALTER` cases)
must be written to complete atomically; migration scripts are reviewed like code.

## R2. Deterministic test substitute for MSSQL

**Unknown**: how to run the same tests without MSSQL (this machine has no docker).

**Decision**: `shared::storage::Storage` is an async trait with two implementations:
`MssqlStorage` (sqlx) and `MemoryStorage` (in-process, `Arc<Mutex<...>>`). Tests and
the deterministic doloop check use `MemoryStorage`; the full-environment suite
(`tests/`, gated on `DATABASE_URL` presence) uses `MssqlStorage`. Contract tests assert
on the trait, so both implementations are proven. This is the Principle XIII/XVII
substitute; the full-environment run happens on the peer machine (docker available) or
CI.

## R3. askama + axum integration

**Unknown**: template rendering path for axum 0.8.

**Decision**: `askama` 0.12 with its `axum` feature — `Template` implements axum's
`IntoResponse`. One base layout + home page in this slice; module pages are added by
their slices. No client-side JS framework.

## R4. Session model

**Unknown**: where the session token lives.

**Decision**: opaque random token (32 bytes, base64url) set as an `HttpOnly` cookie
(`__Host-`-style attributes, `SameSite=Lax`). The session row (token hash, user id,
org scope, created/last-activity, expiry) lives in **MSSQL** (system of record —
Principle VI). The Valkey store holds only *ephemeral* per-session state
(`lms:session:{token_hash}` → JSON bag, TTL = session lifetime) — spec FR-008. If the
store is down at runtime, session-dependent requests 503; the process keeps serving
health (spec User Story 1, scenario 4). Sliding expiry: `last_activity` refreshed on
authenticated requests, bounded by an absolute max lifetime.

**Rationale**: token in the DB (not the cache) means a cache flush never kills active
logins — only the ephemeral bag is lost (spec story 3, scenario 4).

## R5. Auth scope enforcement

**Unknown**: how organization scope is enforced without a framework authorization layer.

**Decision**: an axum extractor (`AuthenticatedSession`) that (a) reads the cookie, (b)
loads the session from `Storage`, (c) checks expiry (sliding update), (d) returns the
identity + scope. Routes declare the required scope; a `#[require_scope]`-style helper
(or a `ScopeGuard` extractor) returns 403 on mismatch (spec story 3, scenarios 2–3).
No framework — an extractor is the axum-idiomatic equivalent (Principle II: no extra
layers).

## R6. Dependency versions

**Finding**: versions are already pinned in the workspace `Cargo.toml`
(axum 0.8, sqlx 0.8 mssql, redis 0.27 tokio-comp, askama 0.12, cookie 0.18, tokio 1,
serde 1). No new dependency is added by this plan — if one becomes necessary during
implementation it gets a plan/ADR entry first (Principle XVIII).

## R7. Graceful shutdown

**Decision**: `tokio::signal` for SIGINT/SIGTERM (and `ctrl_c` on Windows) → axum
`Server::graceful_shutdown` → stop accepting, drain in-flight work, close pools, exit
within the 10 s budget (spec story 1, scenario 3). Port-in-use → bind error surfaced
fail-fast with the port named (spec edge case).

## Open items (none blocking)

- None. All unknowns resolved by decision above.
