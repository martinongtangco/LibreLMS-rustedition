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
