# Work Split — the contract between the two agents

Per Principle V of the constitution. This file is the single source of truth for lane
ownership. Amended only via PR.

## The two agents

| | Agent A (Linux) | Agent B (Windows / "redmi local") |
|---|---|---|
| Machine | Bazzite Linux x86_64, `~/Development/LibreLMS-rustedition` | Windows 11, `C:\Development\LibreLMS-rustedition` |
| Lane (modules) | **Foundation** (host, shared, contracts, auth), **Catalog**, **Management** | **Enrollment**, **SCORM** |
| Reviews | First-reviews Agent B's PRs (runs gates on Linux) | First-reviews Agent A's PRs (runs gates on Windows) |

## Sequence

1. **Phase 0 — Setup** (Agent A): repo, spec-kit, constitution, CI. Agent B clones and
   verifies tooling (`cargo --version`, `uv`/`specify`, `git`, `gh`).
2. **Phase 1 — Foundation spec** (Agent A drafts on `main`, Agent B reviews):
   spec 001 = workspace layout, contracts crate, host (axum), tiberius pool, Valkey client,
   auth skeleton, askama, CI. `/speckit.specify → .plan → .tasks`.
3. **Phase 2 — Foundation implementation** (Agent A, doloop): build the foundation,
   PR → Agent B reviews (first full cross-review; also validates the Windows build
   environment) → merge. Both agents pull.
4. **Phase 3 — Parallel modules**:
   - Agent A: spec 002 (Catalog) + spec 003 (Management), doloop per spec, PRs.
   - Agent B: spec 004 (Enrollment) + spec 005 (SCORM), doloop per spec, PRs.
   - Independent specs proceed concurrently; each PR reviewed by the *other* agent
     before merge (Principle XVI).
5. **Phase 4 — Integration**: host wiring for all modules, full-environment gate on both
   machines + CI, README/quickstart update.

## Rules of the road

- Sequential dependencies are real: nothing module-level starts before the foundation
  PR is merged.
- `contracts`/`shared` changes after the foundation: PR, reviewed by the *other* agent.
- Blocked > one doloop ceiling? Diagnose (Principle XIV) and ping the other agent via the
  agent network with the diagnosis — do not silently work around it.
- When both agents touch `docs/` (ADRs, this file): file-level ownership by PR, no
  simultaneous edits.

## Status (Phase 3)

- **Agent B — Enrollment (spec 004): MERGED** as `49d6c94 story 004: enrollment (#3)`
  (2026-11-07). Spec/plan/tasks on `main` (CRs folded, Agent A APPROVE
  12/12); implementation 18 tasks, all gates green; deciding evidence:
  fresh-container full-env run — 8-connection SC-003 race proof (1 winner +
  7 stable 409 codes via 2601/2627 mapping), idempotency, first-completion-
  wins, durability. Rebased onto `46027ae` (PR #2) with 003+004 host-mount
  coexistence (7-arg `run_with_stop`); 90/90 deterministic on main.
- **Agent A — Catalog (spec 003): MERGED** as `46027ae Spec 003: Course
  Catalog (#2)` (2026-11-07). Five review rounds (Agent B), each a deeper
  class — invalid DDL → production wiring → impossible assertion → wire-edge
  materialization (Guid/DATETIME2 as &str) → client-side self-deadlock →
  test-literal no-ops → fmt — every one closed and engine-proven on fresh
  containers before merge.
- **Agent B — SCORM (spec 005): MERGED** as `535d8db story 005: SCORM (#4)`
  (2026-11-07). Spec (CRs folded `2851ccf`), plan + 18 tasks (plan-level
  wire discipline verified against the implementation by Agent A), 18 tasks
  all gates green. Deciding evidence: fresh-container full-env run on the
  exact pushed commit — 5-slice migration manifest (applied run-1 + no-op
  run-2), N=8 independent-connection launch race (1 winner + 7
  AttemptExists via 2601/2627 mapping, never a 500, never a second row),
  commit durability across reconnect, finished_at engine belt, 4,000-char
  injection-shaped suspend round-trip, post-state exactly 5
  `schema_migrations` rows (independent sqlcmd probe: 3 scorm rows, 1
  enrollment, 0 courses, seed user). Host is now a 10-arg
  `run_with_stop` (documented allow); 107/107 deterministic on main.
  Design NOTE on record (peer review): the finish ordering
  (finalize → record_completion) is intentional — do NOT re-order in
  Phase 4; the outage case self-heals via session scoping.
- **Next (main 107/107 green):** Agent A: spec 006 (Management) doloop —
  4/15 tasks at last sync; identity-upsert design decision documented
  (same (email,name) idempotent join, name-conflict 409, the engine's
  UNIQUE as concurrency arbiter); PR expected, then Agent B reviews
  (standing checklist: guard-scoping, re-role upsert INSERT-leg PK →
  idempotent success, sessions-before-org-row delete cascade, 403-scope
  for unknown user, N=8 duplicate-email race). **Phase 4 (Agent B, ONE PR
  after 006 merges):** both cross-slice adapters (004 CourseSource ← 003
  catalog with the 404/403 decision procedure; 005 EnrollmentPort ← 004
  repo with idempotent first-wins record_completion) + the cross-slice
  E2E (enroll via 004 → scorm launch → finish passed → completion visible
  in the 004 list) + the all-slice full_env battery (pre-state 4060,
  post-state across all six migrations).
