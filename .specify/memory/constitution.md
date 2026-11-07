# LibreLMS-Rustedition Constitution

This project is a teaching exercise, not a product. Its purpose is to learn (a) spec-driven
development with spec-kit on a Rust rewrite of LibreLMS, and (b) how two coding agents can
implement the same codebase in parallel on two machines and keep each other's work honest
through alternating PR reviews. Every principle below exists to serve one or both of those
goals — if a rule doesn't serve either, it doesn't belong here.

## ⚠️ Before You Touch Code

Before editing ANY file, you MUST do the following. No exceptions.

1. **Check your branch**: Run `git branch --show-current`. You must be on a `bug/` or `story/`
   branch. If you are on `main` or any other branch, STOP and create the correct branch first.
2. **Check for a spec**: A spec must exist for this change. If it doesn't, create one via
   `/speckit.specify` on `main` before branching and coding.
3. **Check your lane** (Principle V): confirm the files you intend to edit belong to your
   work split. If they belong to the other agent's lane, coordinate via the agent network
   or a PR comment before touching them.
4. **Declare before editing**: Before every file edit, state:
   - The branch you are on
   - The spec or issue this change belongs to
   - Which constitution principles apply

If any step fails, STOP. Do not proceed to edit files.

## Core Principles

### I. Modular Monolith, Not Microservices (Yet)
The system ships as one deployable Rust binary (`crates/host`) built on **axum**, composed of
independent module crates (`catalog`, `enrollment`, `scorm`, `management`). This gets the
organizational benefit of module boundaries without paying the operational cost of network
calls, service discovery, or distributed transactions before there's a real reason to. Each
module's public API (its `lib.rs` re-exports) is the rehearsal for what would become a
network API boundary if a module ever needed to split out — the seam is designed in from
day one, even though it isn't a network seam yet.

### II. Clean Architecture, Applied Simply
Inside each module crate, dependencies point inward: `domain` knows nothing about
`application`, `application` knows nothing about `infrastructure`. That's the whole rule.
Do not add an ORM abstraction layer, an event bus, or a repository pattern wrapping sqlx
unless a specific, current problem requires it — sqlx already *is* the data-access
abstraction; wrapping it again just adds a layer a human has to read through for no
behavioral gain. Every abstraction that *does* get introduced must be explainable in one
plain sentence to someone who knows Rust but not this codebase. If it can't be, simplify it.

### III. Module Boundaries Are Compiled, Not Conventional
A module crate may only be used by other crates through its public API — never through
`pub(crate)` internals or `#[doc(hidden)]` escape hatches. The `host` crate wires modules
together; module crates never reference each other's internals directly. This is enforced by
a boundary check in CI (`.github/scripts/check-boundaries.sh`, run via `cargo metadata` and
the public-API surface) that fails the build on violation — the point is that an AI agent
(or a human) *cannot* accidentally cross a module boundary and have it silently compile. A
convention that relies on memory or code review isn't a boundary; a failing build is.

### IV. Human-Legible AI-Authored Code
Every non-obvious structural decision — a module boundary, a storage choice, a framework
choice, the parallel-work protocol — gets a short ADR in `docs/adr/` (context → decision →
consequences, one page or less). Code favors explicit, straightforward control flow over
clever generalization. The target reader is someone with solid general Rust knowledge but no
prior exposure to this repo. If a reviewer has to ask "why did the agent do it this way,"
that question should already be answered in an ADR, not just in the agent's now-gone
reasoning.

### V. Parallel Two-Agent Protocol (NON-NEGOTIABLE)
Two coding agents implement this codebase in parallel on two machines (one Linux, one
Windows). The protocol:

1. **Contracts-first**: the shared `contracts` crate and the `shared` infrastructure crate
   (DB pool, Valkey client, mail, auth core) are the foundation. They are written once,
   reviewed by both agents, and merged before module work starts. After merge they are
   treated as shared read-only ground truth; changes to them go through a PR reviewed by
   the *other* agent.
2. **Lane discipline**: the work split is recorded in `docs/work-split.md`. An agent never
   edits files in the other agent's lane. If a task genuinely requires crossing lanes, the
   agent coordinates via the agent network (or a PR comment) first, and the change lands
   through the normal PR flow — never by silently editing the other lane.
3. **Synchronize through git, not chat**: `main` is the single source of truth. Agents pull
   `main` before starting work and open a PR when a slice is ready. Chat is for
   coordination, not for code transfer.
4. **Bounded loops**: long implementation work uses the `doloop` loop-engineering session
   (act → verify → record) with a deterministic check and an iteration ceiling, per
   Principles XIII–XV.

### VI. Polyglot Storage With a Reason, Not by Default
MSSQL is the system of record for everything durable — users, courses, enrollments, final
completion status — accessed via **sqlx (mssql)**. The Redis-protocol store (**Valkey**,
via `redis-rs`) is used *only* for ephemeral, high-churn state that doesn't need relational
guarantees: specifically, the live SCORM `cmi.*` runtime bag during an in-progress attempt,
written on every `LMSSetValue` call and persisted to MSSQL on `LMSCommit`/`LMSFinish`.
Nothing lives permanently in Valkey that isn't either derived from or eventually committed
to SQL. If a future slice wants to put something else in Valkey, the question to answer
first is "would losing this on a cache flush actually be fine?" — if not, it belongs in SQL.

### VII. Spec-Driven, Sliced Thin
No code gets written before its slice has gone through `/speckit.specify` →
`/speckit.plan` → `/speckit.tasks` → `/speckit.implement`. Slices are vertical — a whole
user-visible capability (e.g. "browse and enroll in a course") — not horizontal ("build the
whole domain layer for every module first"). A module only gets built when a slice
currently needs it; no module is scaffolded ahead of demand.

### VIII. Branching Discipline
Every coding task — whether planned or discovered ad-hoc — MUST run on a dedicated Git
branch from `main` so that agentic work can proceed in parallel without interfering with
each other or the integration branch. No commits land on `main` outside of a merged PR.
Branch names follow a strict convention:

- **Prefix**: `bug/` for defect work, `story/` for feature or enhancement work.
- **Format**: `<prefix>/<task-id>-<short-description>` where `task-id` is the numeric or
  alphanumeric identifier from the spec (e.g. `001`, `FEAT-42`) and `short-description` is
  a concise kebab-case phrase (e.g. `story/001-course-catalog-browse`).
- **Lifecycle**: The branch is created before code changes begin, work is committed there,
  and the branch is opened as a PR only after implementation completes and all validation
  checks pass.

### IX. Plan On Main Only
Any SpecKit command that performs planning or creates user stories (`/speckit.specify`,
`/speckit.plan`, `/speckit.tasks`) MUST verify the active Git branch is `main` before
proceeding. If the current branch is anything other than `main` (e.g. a `story/` or `bug/`
working branch), the command MUST stop immediately and report:

- The current branch name
- That planning commands must run on `main`
- The instruction to switch back: `git checkout main`

This prevents specs, plans, and task lists from being authored while already inside an
implementation branch — they belong on the integration branch where the full project state
is visible. Implementation work on feature branches follows *after* planning is complete on
`main`.

### X. No Ad-Hoc Fixes — Document Before You Code
When an issue is discovered through conversation (outside a planned slice), the fix MUST
still follow the SpecKit workflow — no direct code edits on `main` or any branch without
documentation.

The required flow for an ad-hoc issue:

1. **Root cause first**: Before touching code, identify and state the root cause.
2. **Branch off**: Create a `bug/<id>-<desc>` branch from `main` (Principle VIII) before
   any code changes begin.
3. **Document via SpecKit**: Run `/speckit.specify` to capture the issue, root cause, and
   proposed fix in a spec. Then `/speckit.plan` → `/speckit.tasks` →
   `/speckit.implement`.
4. **Structural changes get an ADR**: If root cause analysis reveals the issue stems from
   a design decision, record that finding as an ADR under `docs/adr/` before proceeding.

There is no "too small to document" exemption for code changes.

### XI. Two-Machine Parallelism
When a slice's tasks are independent, the two agents split them per `docs/work-split.md`
and implement concurrently on their respective machines, each running `doloop` sessions
with a deterministic verification check. Sequential dependencies (foundation before modules;
contracts before anything) are handled by merging the dependency first and both agents
pulling `main` before starting. Neither agent waits for the other's *unmerged* work —
the only shared state is `main`.

### XII. Return To Main After Implementation
After an implementation slice completes — whether the PR is merged or still open — the
agent MUST switch the working branch back to `main` (`git checkout main`) and pull. This
is mandatory; a session MUST NOT remain on a `story/` or `bug/` branch after the
implementation session ends.

### XIII. Verification Before Claim
An agent MUST NOT claim a feature is complete until all three verification gates pass
**with concrete proof** from each:

1. **Compiles and lints clean**: `cargo build` succeeds and `cargo clippy --all-targets
   -- -D warnings` passes. Show the output — do not claim "it compiles" without running it.
2. **Tests validate the change**: `cargo test` passes for the touched crates. If no test
   covers the changed behavior, the agent MUST write one before claiming completion. Show
   test output — do not claim "tests pass" without running them.
3. **Runs**: the `host` binary builds and starts, and a smoke check (a `curl` against the
   wired routes, or a documented manual step when the environment lacks the DB/Valkey
   services) responds correctly. Show the evidence.

A fix that compiles but has no test is unverified. Claiming a gate is passed without showing
evidence from running that gate is a violation. When the full environment (MSSQL/Valkey)
is not available on the implementing machine, the gate runs against the deterministic
substitute (SQLite in-memory / mocks) and the PR body MUST state that the full
environment verification is deferred to CI or the reviewing agent's machine (Principle XVI).

### XIV. Bounded Retry — Every Loop Has a Ceiling
An agent MUST NOT attempt the same verification gate more than 2 times against the same
failure without a change in approach. On the second consecutive failure, the agent MUST
STOP and produce a written diagnosis:

- What was tried (the specific fix attempted each time)
- Why each attempt failed (the actual error, not a restatement of the gate)
- Whether the blocker is inside the current spec's scope or reveals a scope/spec problem

The diagnosis goes in the PR body and the session ends there, pending review. `doloop`
enforces this mechanically (iteration ceiling, stop-on-no-progress). Silently retrying a
fourth, fifth, or Nth time is a constitution violation.

### XV. Triage Before Retry
Before any second attempt at a failing gate, the agent MUST classify the failure as one of:

1. **Recoverable — local**: compile error, missing dependency, wrong config value, test
   fixture mismatch. Fix and retry once.
2. **Recoverable — design**: the approach was reasonable but wrong; a different
   implementation strategy is needed. Requires stating the new strategy in one sentence
   before retrying — a retry with no stated change of approach is a repeat, and is
   disallowed under Principle XIV.
3. **Blocking**: missing credentials, undefined/contradictory requirement, environment not
   available, or a failure that recurs identically after a design change. Blocking
   failures go straight to the Principle XIV diagnosis-and-stop.

### XVI. Cross-Agent Review for Merge (NON-NEGOTIABLE)
The agent that authored an implementation MUST NOT be the sole verifier before a PR merges.
The *other* agent MUST review the PR, and the review MUST include running the gates on the
reviewer's own machine:

- `cargo build` + `cargo test` + `cargo clippy --all-targets -- -D warnings` from a clean
  checkout of the PR branch, plus
- a read-through of the diff for lane violations (Principle V) and boundary violations
  (Principle III), and
- where the author's environment couldn't run the full stack, the reviewer MUST run the
  full-environment gate (MSSQL/Valkey) before approving.

Approval is recorded as a GitHub PR review (approve / changes requested). The *author*
merges on approval. Reviews alternate: each agent first-reviews the other's PRs — the
author never self-approves, and a human may still review any PR.

### XVII. Verify Against a Disposable Environment
Evidence for a Principle XIII gate MUST come from a run against state created from scratch
for that run — never long-lived development data.

1. **Tests create their own data.** A test may only assert on rows it created, or rows a
   seeder creates. A test depending on data a human built by hand is unverified.
2. **A step that cannot fail is not evidence.** Any command used as proof MUST exit
   non-zero when it fails. A green step that did nothing is worse than a red one.
3. **Tests read configuration, not hardcoded hosts** — the same test must run on the
   developer machine, the peer agent's machine, and the CI runner.

### XVIII. Rust Hygiene
- `rustfmt` clean (`cargo fmt --check`) and `clippy -- -D warnings` clean are part of
  every gate, not an optional polish step.
- `unwrap()`/`expect()` outside `#[cfg(test)]` code requires a comment justifying why the
  invariant is guaranteed, or a refactor to a proper error type.
- Dependencies are added deliberately: every new crate in a `Cargo.toml` must be named in
  the plan or an ADR. The toolchain is pinned via `rust-toolchain.toml` to a released
  stable band.

## Technology & Scope Constraints

- **Rust stable**, pinned via `rust-toolchain.toml` — never a nightly band, even though
  this is a learning project; a stable toolchain removes one variable from an
  already-experimental exercise.
- **axum** for HTTP (routing, middleware, extraction) with `tower`/`tower-http`.
- **sqlx with the MSSQL driver** for the system of record. (SeaORM is not an option — it
  does not support MSSQL. This decision is recorded in `docs/adr/0002`.)
- **redis-rs** client against a **Valkey** server (Redis-protocol-compatible) for the
  ephemeral SCORM session bag.
- **askama** for server-rendered templates (the Razor Pages replacement), chosen for
  compile-time-checked templates; a slice may opt into plain JSON endpoints where
  server-rendering adds nothing.
- **Auth**: cookie-based with `axum` extractors + `cookie` crate, porting the original's
  claims + org-scoped authorization semantics.
- **SCORM support is deliberately SCORM 1.2, simplified**: manifest parsing, static content
  serving, and a JS API shim covering `LMSInitialize/LMSFinish/LMSGetValue/LMSSetValue/
  LMSCommit` plus the CMI fields real authoring-tool output actually needs
  (`cmi.core.student_id`, `student_name`, `lesson_status`, `credit`, `entry`, `exit`,
  `score.raw`, `session_time`, and `cmi.suspend_data`). SCORM 2004, multi-SCO
  sequencing, and `cmi.interactions` are explicitly out of scope.

## Development Workflow

- The deterministic per-slice loop (used by `doloop`) is:
  `cargo test -p <crate> && cargo clippy -p <crate> --all-targets -- -D warnings` —
  fast enough to run every cycle.
- `cargo build --workspace`, `cargo test --workspace`, `cargo clippy --workspace --all-targets
  -- -D warnings`, and `cargo fmt --check` must pass before a slice is considered done.
- The CI workflow (`.github/workflows/ci.yml`) is the authoritative gate run: clean
  checkout, all gates, plus the module-boundary check (Principle III) and, when CI
  provides MSSQL/Valkey services, the full-environment integration suite. A green local
  run is supporting evidence, not proof (Principle XVII).
- Any decision that took real discussion to reach gets a short ADR under `docs/adr/`,
  numbered sequentially.

## Governance

This constitution supersedes ad hoc choices made mid-slice. It is also the document both
coding agents are expected to read before running `/speckit.plan` or `/speckit.implement` —
if an instruction here is ambiguous, the fix is to simplify the instruction, not to add
more words. Amendments require updating this file, bumping the version below, and — if the
amendment reverses a prior ADR — recording that reversal as a new ADR rather than editing
the old one. Both agents must acknowledge an amendment via a PR before it is considered in
force for in-flight slices.

**Version**: 1.0.0 | **Ratified**: 2026-07-22 | **Last Amended**: 2026-07-22
