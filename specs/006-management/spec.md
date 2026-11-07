# Feature Specification: Organization & Learner Management (006)

**Branch**: `main` (spec authored on main per Principle IX; implementation on
`story/006-management`) | **Date**: 2026-07-07 | **Lane**: Agent A (Phase 3)

## User Scenarios & Acceptance (testable)

### User Story 1 - An admin manages the learners of their organization (Priority: P1)

An organization admin creates, lists, re-roles, and removes the users of
their active organization. This is the port of the original app's
Admin/Learners + organization assignment surface — the user-provisioning
capability 001's seeder deferred ("real user provisioning is a later
slice").

**Acceptance Criteria**

- **FR-001**: The system MUST allow an admin of organization O to create a
  user (email + name) assigned to O with a role (`admin` | `member`),
  returning the created user + a 201. The password hash is the documented
  001 placeholder convention (login/real hashing is a later slice — 001 CR-F
  parity).
- **FR-002**: The system MUST reject a duplicate email with 409
  `duplicate-email` (the engine's UNIQUE constraint on `dbo.users.email` is
  the final arbiter — the repository maps the unique violation to
  `DuplicateEmail`, never a 500).
- **FR-003**: The system MUST allow an admin to list the users of their
  organization (id, email, name, role) — member users of other
  organizations MUST never appear.
- **FR-004**: The system MUST allow an admin to change a user's role within
  the organization (`PUT` role) and to remove a user from the organization
  (the user row itself is NOT deleted — only the membership row; a user may
  belong to multiple organizations).
- **FR-005**: All management operations MUST be admin-gated (a member of O
  receives 403 `forbidden` on every management endpoint) and
  organization-scoped (an admin of O against O' receives 403 `scope`,
  content-identical to any other 403 — the 003 US3.2 no-existence-oracle
  parity).

**Acceptance Scenarios**

1. **Given** an admin of O and a new email, **When** they create a user in
   O, **Then** 201 with the user + role; the user appears in the org list.
2. **Given** an email already existing (any org), **When** a create is
   attempted, **Then** 409 `duplicate-email` (stable code; a concurrent
   duplicate is arbitrated by the engine's UNIQUE constraint — the
   deterministic duplicate test is labeled LOGIC-LAYER, mirroring 004's
   PQ-1 discipline; the real-engine proof rides the gated full-env test).
3. **Given** an admin of O and a user of O, **When** the admin changes the
   role to `member`, **Then** 200 and the user is no longer an admin of O.
4. **Given** a member of O, **When** they call any management endpoint,
   **Then** 403 `forbidden`.
5. **Given** an admin of O, **When** they target a user of O', **Then** 403
   `scope` (never a 404 — no existence oracle).

### User Story 2 - An admin manages organizations (Priority: P2)

An admin creates, lists, renames, and deletes organizations. Flat in this
slice (the original's parent/child tree + SVG chart is OUT OF SCOPE — see
Assumptions).

**Acceptance Criteria**

- **FR-006**: The system MUST allow an admin (any org — organization
  management is a platform-level admin capability, gated by role, not by
  org scope) to create an organization (name), returning 201.
- **FR-007**: The system MUST allow listing organizations (id, name) with a
  user count each.
- **FR-008**: The system MUST allow renaming an organization (200) and
  deleting an organization (204).
- **FR-009**: The system MUST refuse to delete an organization that still
  has members with 409 `not-empty` (memberships must be removed first —
  no silent cascade of membership rows). A successful delete
  **cascade-expires the organization's active sessions** (CR-2, option b:
  `dbo.sessions` rows for the org are removed as part of the delete — a
  deleted org cannot hold a live session; this is DML through the module's
  own repository, so FR-013's no-DDL constraint holds and the
  `sessions.organization_id` FK is satisfied by construction).
- **FR-010**: Unknown organization ids MUST yield 404 `not-found`; the
  delete 409 check MUST not reveal whether the id exists to out-of-scope
  callers (role-gated: a member gets 403 before any id check). CR-3 (003
  US3.2 parity, explicit): user-side operations (re-role / remove) against an
  **unknown or other-org user id answer 403 `scope` — never 404**; the 404
  row above applies to organization ids only.

**Acceptance Scenarios**

1. **Given** an admin, **When** they create an organization, **Then** 201;
   it appears in the list with user count 0.
2. **Given** an organization with 2 members, **When** an admin deletes it,
   **Then** 409 `not-empty`; after removing both memberships, **When** the
   delete is retried, **Then** 204.
3. **Given** an admin, **When** they rename an organization, **Then** 200
   and the list reflects the new name.

### User Story 3 - Error semantics and robustness (Priority: P3)

**FR-011**: All endpoints return the 001-style JSON envelope
(`{"error": <stable code>}` + status); 401 without a session; 403
`forbidden` (non-admin) and `scope` (cross-org user operations); 404
`not-found`; 409 `duplicate-email` / `not-empty`; 503
`storage-unavailable` on storage outage — NEVER a 500, never a silent
empty list.
**FR-012**: Validation: email non-empty and ≤ 320 chars (the
`dbo.users.email` column bound); name non-empty and ≤ 200 chars; organization
name non-empty and ≤ 200 chars; **role is restricted to `admin` | `member`**
(CR-1: a junk role is a 400 `validation` at the app layer; the
`dbo.user_organizations` CHECK constraint is the belt, not the suspenders —
a 2627-class CHECK violation maps to the SAME 400, never a 500) — 400 with
the field name.
**FR-013**: The module MUST own its repository (trait + tiberius impl +
in-memory double) per the 001 Module Persistence Seam, and MUST depend on
`contracts` + `shared` only (boundary check). The module adds NO new DDL in
this slice (all touched tables are foundation-owned: `dbo.users`,
`dbo.user_organizations`, `dbo.organizations` — 001); its `MIGRATIONS`
manifest is empty by construction, documented as such.

**Acceptance Scenarios**

1. **Given** a valid request shape battery, **When** fields violate
   FR-012, **Then** 400 naming the field.
2. **Given** a storage outage, **When** any management endpoint is called,
   **Then** 503 `storage-unavailable` (never a silent empty list, never 500).
3. **Given** injection-shaped email/name values, **When** submitted,
   **Then** parameterized handling, no error, stored/compared as plain text.

## Assumptions & Out of Scope

- **Flat organizations in this slice**: the original's parent/child org tree
  (TreeLayoutService, org chart SVG, `_OrgChartSvg.cshtml`) is OUT OF
  SCOPE — a follow-up spec (candidate 007). 001's `dbo.organizations` is
  (id, name) only; no ALTER of foundation tables by a module (no DDL
  coupling — the seam principle).
- **No password hashing in this slice**: the 001 placeholder convention
  stands (001 CR-F: real argon2id lands with the login slice).
- **No pagination** (teaching slice; list endpoints return full
  lists — parity with 003's decision).
- **Organization management is role-gated, not org-scope-gated** (an admin
  of any org manages the org list — platform-level admin capability,
  matching the original's Admin portal shape). User operations ARE
  org-scope-gated (FR-005).
- **Duplicate email arbitration**: the engine's UNIQUE constraint is the
  final arbiter (the 004 PQ-1 discipline); the deterministic duplicate test
  is LOGIC-LAYER; the real-engine proof rides the gated full-env test and
  MUST use N INDEPENDENT connections (one per concurrent attempt — the 004
  CR-1 lesson: a shared single-client repo serializes the contest and turns
  the proof into a formality).
- **No cross-module dependencies**: user/org data is foundation-owned; this
  module performs DML through its own repository (like 001's auth writes to
  `dbo.sessions`) — no module-to-module coupling, no ports needed in this
  slice.

## Error Semantics (envelope mapping)

| condition | status | stable code |
|---|---|---|
| absent session | 401 | `unauthenticated` (foundation) |
| non-admin role | 403 | `forbidden` |
| cross-org user operation | 403 | `scope` (content-identical to any other 403) |
| unknown org id | 404 | `not-found` |
| duplicate email (incl. unique violation) | 409 | `duplicate-email` |
| delete non-empty org | 409 | `not-empty` |
| validation failure (incl. junk role — the CHECK constraint is the belt) | 400 | `validation` (field named in the body) |
| storage outage | 503 | `storage-unavailable` |
