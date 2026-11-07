# Quality Checklist: 006-management

- [x] Spec is testable: every FR maps to an acceptance scenario or a
      deterministic/gated test
- [x] No implementation leakage: no framework/DB/API-shape mandates beyond
      the workspace conventions (tiberius/axum/askama per ADR 0001–0003)
- [x] Constitution alignment: thin slice (Principle VII), module boundary
      (III/IV), red-first tasks to follow, cross-agent review (XVI)
- [x] Error semantics complete: 401/403×2/404/409×2/400/503 — never 500,
      never a silent empty list
- [x] No new DDL: all touched tables are foundation-owned; the module's
      MIGRATIONS manifest is empty by construction (documented)
- [x] Out-of-scope explicit: org tree/chart, password hashing, pagination
- [x] Cross-lane contract: none needed this slice (foundation-owned data
      only); the 004/005 modules are untouched
