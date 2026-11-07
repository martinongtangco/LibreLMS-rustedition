# ADR 0002: sqlx (MSSQL), not SeaORM

- Status: accepted
- Date: 2026-07-22

## Context

The original uses EF Core against MSSQL. The natural "Rust ORM" candidates are SeaORM,
sqlx, and Diesel.

## Decision

**sqlx with the MSSQL driver** (plus `sqlx` migrations). SeaORM is excluded because it
does not support MSSQL (PostgreSQL/MySQL/SQLite only) and MSSQL is the system of record
(Principle VI). Diesel is viable but its async story is newer; sqlx is mature, async
first, compile-time-checkable queries, and first-class MSSQL support.

## Consequences

- Data access is explicit and SQL-shaped rather than an ORM abstraction (fits Principle
  II: no wrapping layer around the data-access abstraction).
- No change tracking / lazy loading — the application layer owns transaction scope.
- Deterministic tests (Principle XIII/XVII) use SQLite in-memory or mocks where a full
  MSSQL environment is unavailable; full-environment integration runs in CI or on the
  reviewing agent's machine.
