# Feature Specification: Course Catalog

**Feature Branch**: `story/003-course-catalog`

**Created**: 2026-07-24

**Status**: Draft

**Input**: Work-split assignment: Agent A (Linux) owns the Catalog lane — the course catalog: listing, search/filter, course CRUD for the signed-in organization, with organization-scoped visibility and role-based (admin|member) permissions.

## User Scenarios & Testing *(mandatory)*

### User Story 1 - A learner sees their organization's courses (Priority: P1)

An authenticated member of an organization signs in and opens the catalog. They see a list of the courses that exist *for their organization* — nothing from any other organization. They can narrow the list by typing part of a course title.

**Why this priority**: Without a browsable, scoped catalog, no downstream feature (enrollment, learning, SCORM) has anything to act on. This is the smallest independently valuable slice of the Catalog lane.

**Independent Test**: Sign in as the seeded development member (spec 001), open the catalog, and verify: the list shows exactly the courses belonging to the session's organization; typing a title substring filters the list; a member from a different (hypothetical) organization sees zero of these courses.

**Acceptance Scenarios**:

1. **Given** an organization with 3 courses, **When** a member of that organization opens the catalog, **Then** all 3 courses appear, each with its title (and description, if set), and no course from any other organization appears.
2. **Given** the 3 courses above, **When** the member searches for a substring that matches 2 titles, **Then** exactly those 2 appear (case-insensitive substring match on title).
3. **Given** a search that matches nothing, **When** the member submits it, **Then** an empty result is shown with a clear "no courses found" indication — not an error.
4. **Given** a signed-out visitor, **When** they request the catalog, **Then** they are asked to sign in (401-class response), never shown any courses.

---

### User Story 2 - An admin manages their organization's courses (Priority: P2)

An administrator of an organization creates, updates, and deletes courses for that organization. Creation requires a title; description is optional. Every change is visible to members of the same organization immediately.

**Why this priority**: Catalog content must be creatable and correctable for the P1 read experience to stay true; but P1 delivers value on its own (seeded content), so P2 is independently deployable.

**Independent Test**: Sign in as the development admin, create a course via the catalog, verify it appears in the member list; edit its title, verify the update is visible; delete it, verify it disappears and a subsequent detail request reports "not found".

**Acceptance Scenarios**:

1. **Given** an authenticated admin, **When** they create a course with a valid title (non-empty, ≤ 200 characters) and an optional description (≤ 5,000 characters), **Then** the course exists and appears in the organization's list.
2. **Given** a create attempt with an empty or > 200-character title (or > 5,000-character description), **When** it is submitted, **Then** it is rejected with a message that names the offending field — nothing is stored.
3. **Given** an existing course, **When** the admin updates its title or description, **Then** the new values are returned by the list and detail views.
4. **Given** an existing course, **When** the admin deletes it, **Then** it no longer appears in the list, and requesting it by identifier reports "not found" (404-class).
5. **Given** an admin of organization A, **When** they attempt to create/update/delete a course of organization B, **Then** the request is rejected as out of scope (403-class); nothing in organization B changes.

---

### User Story 3 - Permissions and scope are enforced on every course path (Priority: P3)

Every catalog action honors the session's organization scope and role: members may read, admins may read and write, and no course from another organization is ever readable, writable, or visible to a given session.

**Why this priority**: Isolation is the security property of the whole catalog; it is testable independently against the P1/P2 surfaces, and its absence would make the other two stories unsafe to release.

**Independent Test**: With two organizations and known course sets, run the full matrix: member-A on A-courses (read OK), member-A on B-courses (denied/hidden), admin-A creating into B (denied), non-admin writing into A (denied), unknown course id (404-class, no leakage).

**Acceptance Scenarios**:

1. **Given** a member (non-admin) of organization A, **When** they attempt to create, update, or delete any course, **Then** the request is rejected (403-class) and no data changes.
2. **Given** an authenticated session for organization A, **When** they request by identifier a course that belongs to organization B, **Then** the response is an out-of-scope rejection (403-class) — indistinguishable in content from any other out-of-scope attempt (no existence oracle).
3. **Given** any authenticated session, **When** they request a course identifier that does not exist, **Then** a plain "not found" (404-class) with no internal details is returned.

---

### Edge Cases

- Empty organization (no courses): the list shows an empty state, never an error.
- Title at exactly 200 characters (boundary): accepted; at 201: rejected naming the field.
- Description at exactly 5,000 characters (boundary): accepted; at 5,001: rejected naming the field.
- Course updated concurrently by two admins: last write wins; both succeed or one gets a clear retryable indication — no corruption, no 500.
- Storage backend unavailable during a catalog request: a 503-class response with the standard envelope — never a 500, never a silent empty list.
- Search term containing SQL-injection-shaped or path-shaped input: treated as plain text; no error, no injection.

## Requirements *(mandatory)*

### Functional Requirements

- **FR-001**: The system MUST list the courses belonging to the signed-in session's organization, each with its title and (if set) description.
- **FR-002**: The system MUST filter the list by a case-insensitive substring of the course title; a non-matching search returns an empty result with a clear indication, not an error.
- **FR-003**: An organization's administrator MUST be able to create a course with a title (required, non-empty, ≤ 200 characters) and an optional description (≤ 5,000 characters); invalid input MUST be rejected with a message naming the offending field, and nothing MUST be stored.
- **FR-004**: An organization's administrator MUST be able to update (title and/or description) and delete a course of their organization; updates MUST be visible to members of that organization immediately.
- **FR-005**: A non-admin member MUST be able to read (list/detail) courses of their organization and MUST be rejected (403-class) on any create/update/delete.
- **FR-006**: Any course action whose target course belongs to a different organization than the session's MUST be rejected (403-class); courses of other organizations MUST never appear in a session's list.
- **FR-007**: Requests for a non-existent course identifier MUST return a plain "not found" (404-class) with no internal details.
- **FR-008**: Catalog data MUST persist in the system of record (the module owns its tables and migrations per the spec 001 Module Persistence Seam decision); a storage outage MUST produce a 503-class response, never a 500 or a silent empty list.
- **FR-009**: All catalog responses MUST use the standard success/error envelopes (spec 001 contracts) and MUST never expose backend internals or secrets.

### Key Entities

- **Course**: a titled, optionally described learning unit that belongs to exactly one organization; its lifecycle is create → update* → delete (*title/description).
- **Organization** (from spec 001): the scope boundary; a course's visibility and writable set are defined by it.
- **Membership** (from spec 001): the role (admin|member) that gates read vs. write on a course.

## Success Criteria *(mandatory)*

### Measurable Outcomes

- **SC-001**: An authenticated member can see their organization's full course list (up to 1,000 courses) in under 2 seconds.
- **SC-002**: Title search is 100% precise and 100% recall for substring matches: every matching course appears, no non-matching course appears (verified against a known fixture set).
- **SC-003**: A course an admin creates appears in the organization's list within 3 seconds, with the exact title and description submitted.
- **SC-004**: Cross-organization isolation is 100%: in a two-organization fixture, zero courses of organization B are visible, readable, or writable from any session of organization A (and vice versa).
- **SC-005**: 100% of invalid create/update inputs are rejected with a message that names the offending field; zero invalid rows are stored (verified against a fixture battery including boundary sizes).

## Assumptions

- The spec 001 foundation (session/identity/org-scope, config, storage seam, envelopes, 503 behavior) is merged and available; the Catalog slice consumes it and adds no new shared surface.
- A course's identifying attributes are title and optional description only; richer curriculum content (modules, lessons, SCORM packages) belongs to later slices and is out of scope here.
- Read access requires an authenticated, in-scope session (spec 001); there is no anonymous catalog in this slice.
- The list is returned fully (no pagination in this slice); the 1,000-course bound of SC-001 is the performance assumption that later slices may revisit.
- Concurrency is last-write-wins (no optimistic locking) at this scale.
- Deterministic tests use the in-crate repository trait + in-memory double pattern (spec 001 Module Persistence Seam); real-SQL verification runs through the gated full-environment test pattern, exercised on Agent B's machine per Principle XIII/XVI.
