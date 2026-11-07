# ADR 0001: Modular monolith, not microservices (yet)

- Status: accepted
- Date: 2026-07-22

## Context

LibreLMS (original, .NET) ships as one process with three independent modules. The Rust
port must decide: one binary with module crates, or separate services?

## Decision

One binary (`crates/host`, axum) composed of independent module crates
(`catalog`, `enrollment`, `scorm`, `management`), a `contracts` crate for shared
DTOs/interfaces, and a `shared` crate for infrastructure (tiberius MSSQL pool, Valkey
client, mail, auth core). Modules never depend on each other's internals; `host` wires
them together. The `contracts` crate is the rehearsal for a future network boundary if a
module ever splits out.

## Consequences

- Module boundaries are enforced by the CI boundary check (Principle III), not by
  network costs.
- One deployment artifact; no service discovery, no distributed transactions.
- Splitting a module later means replacing an in-process call with a network call — the
  `contracts` surface is designed to make that mechanical.
