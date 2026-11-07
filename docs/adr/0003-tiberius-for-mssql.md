# ADR 0003: tiberius for MSSQL (amends ADR 0002)

- Status: accepted (amends the driver choice in ADR 0002; the exclusion of
  SeaORM in ADR 0002 stands)
- Date: 2026-07-22

## Context

ADR 0002 selected sqlx "with the MSSQL driver" because SeaORM does not support
MSSQL. During implementation (doloop cycle 1, spec 001), the resolver proved this
premise wrong: **sqlx 0.8 has no `mssql` feature at all** — sqlx's MSSQL support
was removed in the 0.7 line. The candidates were re-evaluated:

- sqlx 0.8: PostgreSQL / MySQL / SQLite only — excluded (no MSSQL).
- SeaORM: same three, no MSSQL — excluded (ADR 0002 stands).
- diesel: has an experimental `mssql_backend`, but its async layer (diesel-async)
  does not expose MSSQL — excluded for an async-first design.
- **tiberius**: dedicated TDS (MSSQL) client, tokio-native, built-in connection
  pool, mature (used in production by several Rust services). Raw, explicit SQL —
  no ORM, which fits Principle II.

## Decision

**tiberius** (tokio feature) as the system-of-record client (Principle VI). Data
access stays explicit raw SQL (same shape ADR 0002 intended); the
`migrations/` runner executes DDL through the same client (R1 unchanged).

## Consequences

- One less abstraction layer than ADR 0002 promised (no sqlx query-builder) —
  queries are plain SQL strings, reviewed like code.
- `Storage` trait (research R2) is unchanged: `TiberiusStorage` + `MemoryStorage`;
  deterministic tests still use the in-memory implementation (Principle XIII/XVII).
- ADR 0002's exclusion of SeaORM remains valid and is re-confirmed here.
