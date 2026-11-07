# AGENTS.md

Instructions for coding agents working in this repository.

## Read this first, in order

1. `.specify/memory/constitution.md` — the rules. Non-negotiable parts are marked.
2. `docs/work-split.md` — whose lane you're in and how the two agents coordinate.
3. `docs/adr/` — *why* the non-obvious decisions were made.

## The loop

- Planning commands (`/speckit.specify|plan|tasks`) run on `main` only (Principle IX).
- Code changes run on `story/` or `bug/` branches (Principle VIII) — never on `main`.
- Implementation uses `doloop` with a deterministic check:
  `cargo test -p <crate> && cargo clippy -p <crate> --all-targets -- -D warnings`.
- Done means: build + test + clippy evidence, a running smoke check (Principle XIII),
  and a PR reviewed by the *other* agent (Principle XVI). You merge on their approval.
- After the slice: back on `main`, pulled (Principle XII).

## Coordination with the other agent

- Never edit files in the other agent's lane (Principle V.2). Coordinate via the agent
  network or a PR comment; code moves through `main` only.
- `contracts` and `shared` are shared ground truth: changes go through a PR reviewed by
  the *other* agent.
- Chat is for coordination, not code transfer.
