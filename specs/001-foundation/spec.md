# Feature Specification: Application Foundation

**Feature Branch**: `story/001-foundation`

**Created**: 2026-07-22

**Status**: Draft

**Input**: User description: "Foundation for the Rust port: application workspace, shared contracts surface,
host process, system-of-record access, session store, auth skeleton, server-rendered pages,
and the CI gate set — so that the Catalog, Management, Enrollment, and SCORM slices can be
built on shared ground truth (see docs/work-split.md, Phase 2)."

## User Scenarios & Testing *(mandatory)*

### User Story 1 - The application starts, connects, and proves itself alive (Priority: P1)

A developer (or the CI runner) points the application at a system-of-record database and a
session store via configuration, starts it, and immediately gets unambiguous proof of the
application's state: it started, the database connection works, the session store connection
works, and the process shuts down cleanly on signal.

**Why this priority**: Nothing else — not the module slices, not the two-agent parallel
work — can start until the application lifecycle is dependable. This is the unblocking
slice for both agents (Principle V).

**Independent Test**: Start the application with a valid configuration and confirm the health
endpoint reports the three connectivity states; start it with one configuration item
removed and confirm it refuses to start with an error naming the missing item.

**Acceptance Scenarios**:

1. **Given** a valid configuration (database, session store, port), **When** the
   application starts, **Then** it completes startup within 30 seconds and the health
   endpoint reports: process alive, database reachable, session store reachable.
2. **Given** a configuration missing any required item, **When** the application starts,
   **Then** it exits within 5 seconds with a non-zero status and a message naming the
   missing item (not a stack trace, not a timeout).
3. **Given** a running application, **When** it receives a termination signal, **Then** it
   stops accepting new requests, finishes in-flight work, and exits within 10 seconds.
4. **Given** a running application, **When** its session store becomes unreachable at
   runtime, **Then** requests that require the session store fail with a clear 503 and the
   process keeps serving requests that don't need it.

---

### User Story 2 - Modules build against a shared contract, and crossing boundaries is a build failure (Priority: P2)

The foundation provides the shared data shapes (user, organization, course, enrollment,
completion, SCORM attempt status) that every module uses, plus the mechanism that makes a
module-to-module dependency a *compile/CI failure* rather than a convention.

**Why this priority**: The two-agent parallel protocol (Principle V) only works if the
shared surface is stable and the lanes are mechanically enforced — otherwise the two
agents' work silently tangles.

**Independent Test**: Introduce an intentional dependency from one module crate to another
and confirm the boundary check fails the build; remove it and confirm the check passes.

**Acceptance Scenarios**:

1. **Given** the shared contract shapes, **When** any module compiles, **Then** it can use
   every shared shape without depending on any other module crate.
2. **Given** an intentional module-to-module dependency, **When** the boundary check runs,
   **Then** it fails with a message naming the offending pair, and the CI build is red.
3. **Given** the base schema (users, organizations), **When** the migration process runs
   against a fresh database, **Then** the schema is created idempotently and a second run
   changes nothing.

---

### User Story 3 - A browser session carries identity and organization scope (Priority: P3)

A learner or administrator signs in and receives a browser session that carries *who they
are* and *which organization they are acting in*; the session survives page loads for its
configured lifetime, refreshes on activity, and expires when it should. Ephemeral,
per-attempt state (later: the live SCORM bag) is storable in the session store keyed by
session — the *live* bag is not in the database (committed SCORM state is persisted by
the owning module per Principle VI; see FR-008).

**Why this priority**: Both the learner-facing (Enrollment, SCORM) and the admin-facing
(Management) slices assume authenticated, organization-scoped actors. The *mechanism* is
foundation; the *login pages and role behavior* are later slices.

**Independent Test**: Create a session, present it across two requests with an organization
scope, verify both succeed and the scope is enforced; wait past the lifetime (or force
expiry) and verify the session is rejected.

**Acceptance Scenarios**:

1. **Given** an unauthenticated request to a protected route, **When** it is made,
   **Then** it is rejected with a 401 (or redirect to sign-in) and no organization scope
   is assumed.
2. **Given** an authenticated session scoped to organization A, **When** it requests
   something scoped to organization B, **Then** the request is rejected with a 403.
3. **Given** a live session, **When** activity occurs, **Then** the session lifetime is
   extended (sliding); **When** the lifetime passes without activity, **Then** the session
   is rejected on the next request.
4. **Given** ephemeral state stored for a session, **When** the session store is flushed,
   **Then** no data is lost from the database — only the ephemeral state is gone.

---

### Edge Cases

- Configuration with wrong *type* for a value (not just missing): same fail-fast behavior
  as missing, naming the offending item.
- Database reachable but schema migrations **pending** (startup runner not yet complete):
  startup succeeds, health check reports "schema not ready" instead of crashing.
- A startup migration **fails**: fail-fast, naming the offending script (FR-001 posture —
  a failed migration is deploy-blocking, not a degraded state).
- Unknown route: 404 with a plain, non-leaky body (no stack traces, no internal paths).
- Session token presented but the session store entry is gone: treated as expired (401),
  never a server error.
- Two processes starting with the same port: the second fails fast with a clear message.

## Requirements *(mandatory)*

### Functional Requirements

- **FR-001**: The application MUST start from environment configuration (system-of-record
  connection, session store address, port) and MUST refuse to start within 5 seconds with
  an error naming any missing or malformed required item.
- **FR-002**: The application MUST expose a health check (JSON) reporting process liveness,
  database connectivity, and session-store connectivity, and MUST serve a minimal
  server-rendered home page.
- **FR-003**: The application MUST verify database connectivity at startup and run the
  schema migration process against a fresh database idempotently.
- **FR-004**: The system of record MUST hold the base schema: users, organizations, and
  the session/identity data the auth mechanism needs.
- **FR-005**: A shared contract surface MUST provide the typed data shapes — user,
  organization, course, enrollment, completion, SCORM attempt status — usable by every
  module without inter-module dependencies.
- **FR-006**: An automated boundary check MUST fail the build when any module crate
  depends on another module crate, naming the offending pair; it runs in CI and locally.
- **FR-007**: Browser sessions MUST be established and renewed via a session token with a
  configurable lifetime (sliding on activity); the session MUST carry the actor's identity
  and organization scope, and requests outside the scope MUST be rejected.
- **FR-008**: Ephemeral per-session state MUST be storable in the session store keyed by
  session id with a configurable TTL, and MUST not *persist* to the system of record.
  (Scope: the **live** bag is ephemeral; **committed** state — e.g. SCORM
  `LMSCommit`/`LMSFinish` records — is persisted to the system of record by the owning
  module, per Principle VI.)
- **FR-009**: The application MUST log startup, shutdown, and errors in a structured way,
  and MUST never log connection strings, session tokens, or other secrets.
- **FR-010**: A mail-sending interface MUST be defined with a development outbox
  implementation (messages visible in a dev page); real delivery is out of scope for this
  slice.
- **FR-011**: The CI gate set MUST include: format check, lint with warnings-as-errors,
  full build, full test, and the module-boundary check — all green on a clean checkout on
  both a Linux machine and a Windows machine.

### Key Entities *(include if feature involves data)*

- **User**: a person who signs in; attributes: identifier, name, email, password
  credential (hashed), theme preference. Belongs to one or more organizations.
- **Organization**: the scope boundary for data and authorization; attributes: identifier,
  name. Users have a role within an organization.
- **Session**: an authenticated browser session; attributes: token, user, organization
  scope, created/last-activity times, TTL. Ephemeral state lives in the session store, not
  the database.
- **Contract shapes** (Course, Enrollment, Completion, SCORM attempt status): typed
  placeholders with the fields the later slices need — defined here, populated later.

## Success Criteria *(mandatory)*

### Measurable Outcomes

- **SC-001**: On a fresh environment with a valid configuration, the application starts and
  the health check passes within 30 seconds — on both the Linux and the Windows machine.
- **SC-002**: 100% of the 3 missing-configuration variants (database, session store, port)
  fail startup within 5 seconds with an error naming the missing item.
- **SC-003**: An intentionally introduced module-to-module dependency is detected by the
  boundary check (1 of 1 detection), and the check passes on the clean tree.
- **SC-004**: A session survives page loads within its lifetime, extends on activity, and
  is rejected after expiry — verified by test, not by manual observation.
- **SC-005**: All CI gates (SC-001's machine pair + format, lint, build, test, boundary)
  pass on a clean checkout without manual setup beyond the configuration.

## Assumptions

- Business behavior of the module slices (catalog browsing, enrollment flows, SCORM launch
  logic, admin actions) is **out of scope** for this slice — the foundation provides the
  lifecycle, storage wiring, contract, auth *mechanism*, and gates only.
- Single-tenant development environment: one database, one session store. Multi-tenancy
  and production deployment concerns are not addressed here.
- The system of record is MSSQL per the constitution (Principle VI); deterministic local
  and CI test runs use an in-memory substitute behind the same access interface, and the
  full-environment verification runs on the machine that has the services (Principle XIII
  / XVI).
- Session tokens are opaque, randomly generated, and stored server-side (the database);
  the session store holds only ephemeral state (Principle VI).
- One seeder user (development identity) is created by the migration/seeder process for
  manual smoke testing; real user provisioning is a later slice.
- Windows dev machine B builds with a per-directory rustup override
  `1.99.1-x86_64-pc-windows-gnu` (MSVC is not usable on that machine — partial VS
  installs, no Win10 SDK; see plan Technical Context). Gate commands are
  toolchain-agnostic, so `rust-toolchain.toml`'s `1.99.1` stays the single pinned
  version. No cross-compilation targets are needed in this slice.
