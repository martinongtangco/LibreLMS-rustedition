# Feature Specification: Course Enrollment & Completion

**Feature Branch**: `story/004-enrollment`

**Created**: 2026-07-07

**Status**: Draft

**Input**: User description: "Enrollment slice (spec 004, Agent B lane per docs/work-split.md): a
learner (member role) enrolls in a course of their organization, views their
enrollments, and completion is recorded against an enrollment — on the shared
foundation (spec 001) with the shared contract shapes (Enrollment, Completion) and
the module persistence seam (in-crate repository + deterministic double + own
migrations)."

## User Scenarios & Testing *(mandatory)*

### User Story 1 - A learner enrolls in a course of their organization (Priority: P1)

A learner who belongs to an organization picks a course of that organization and
enrolls. The enrollment is recorded and survives restarts; the learner cannot
enroll in a course they don't belong to, and can't enroll twice in the same
course.

**Why this priority**: "Browse and enroll in a course" is the core learner
capability of the LMS (Principle VII vertical slice). Without it, the SCORM slice
(spec 005) has nothing to attach attempts to, and the Catalog slice (spec 003)
has no consumer to prove the org-scoped course model end-to-end.

**Independent Test**: With a valid session and a known course of the session's
organization, create an enrollment and confirm it exists with the right shape;
attempt a second enrollment of the same course and confirm the rejection; attempt
an enrollment of an out-of-organization course and confirm the scope rejection —
all verifiable with no other module implemented.

**Acceptance Scenarios**:

1. **Given** an authenticated member of organization A and a course that exists in
   organization A, **When** the member enrolls in that course, **Then** an
   enrollment with status "enrolled" for (member, course, organization A) is
   created and durable.
2. **Given** the same member, **When** they enroll in the same course a second
   time, **Then** the request is rejected with a stable error code and no
   duplicate is created — the first enrollment remains the single source of truth.
3. **Given** a member of organization A, **When** they attempt to enroll in a
   course that belongs to organization B, **Then** the request is rejected with a
   scope error (403-class) and no enrollment is created.
4. **Given** an unauthenticated request, **When** an enrollment is attempted,
   **Then** it is rejected with 401 and no enrollment is created.

---

### User Story 2 - The learner sees their enrollments; an admin sees the organization's (Priority: P2)

A learner sees the list of their own enrollments (course + status) for their
active organization. An organization admin sees the enrollments of the whole
organization. The views are organization-scoped: nothing from another
organization is ever visible.

**Why this priority**: Enrolling without being able to see the result is not a
viable capability; the admin view is what makes the admin|member role model
observable (foundation auth scope mechanism, spec 001 US3).

**Independent Test**: Create enrollments for two members of one organization and a
member of a second organization; as each role, list and confirm exactly the
expected rows are visible (and no other organization's rows appear).

**Acceptance Scenarios**:

1. **Given** a member with existing enrollments, **When** they list their own
   enrollments, **Then** they see their own enrollments with course identifier and
   status, and nothing from other organizations.
2. **Given** a member with no enrollments, **When** they list, **Then** an empty
   list is returned (not an error).
3. **Given** an admin of organization A and at least one member enrollment in
   organization A, **When** the admin lists the organization's enrollments,
   **Then** the member enrollments of organization A are visible and no
   enrollment of any other organization is visible.
4. **Given** a member (non-admin) of organization A, **When** they request the
   organization-wide enrollment view, **Then** the request is rejected with a
   403-class scope/permission error.
5. **Given** an unauthenticated request, **When** either view is requested,
   **Then** it is rejected with 401.

---

### User Story 3 - Completion is recorded and the enrollment becomes completed (Priority: P3)

When a learner's completion of an enrolled course is recorded, the enrollment
transitions to "completed" and a completion record (with a timestamp) is stored
with it. Completion is recorded once — the first completion is the source of
truth.

**Why this priority**: The enrolled→completed transition is the terminal state of
the learner journey and the hook the SCORM slice (spec 005) will drive in a later
slice; it must exist and be correct before that slice is built on it.

**Independent Test**: Create an enrollment, record completion, and verify the
status transition and the completion record; attempt a second completion and
verify the rejection with the original record unchanged.

**Acceptance Scenarios**:

1. **Given** an enrollment with status "enrolled", **When** completion is
   recorded for it, **Then** the enrollment's status is "completed" and a
   completion record with a timestamp is associated with it.
2. **Given** an enrollment that is already "completed", **When** completion is
   recorded again, **Then** the request is rejected with a stable error code and
   the original completion (timestamp and all) is unchanged.
3. **Given** a completion recorded against an enrollment that does not exist or
   is outside the session's organization scope, **When** it is attempted,
   **Then** it is rejected with a stable error code and nothing is persisted.
4. **Given** a recorded enrollment and completion, **When** the session store is
   flushed or the process restarts, **Then** both remain (durable in the system
   of record).

---

### Edge Cases

- Enrollment in a course identifier that does not exist: rejected with a
  404-class stable error (not 403, not a server error).
- A user who is a member of no organization attempts to enroll: rejected with a
  scope error (403-class).
- Concurrent duplicate enrollments (the same learner enrolls twice in a tight
  window): exactly one enrollment exists afterwards — duplicate detection is safe
  under concurrency, not just sequential.
- Completion recorded for an enrollment that is not "enrolled": rejected with a
  stable error; the first completion's timestamp is preserved.
- Listing enrollments while the session store is unavailable: 503 (foundation
  behavior), the process keeps serving non-session-dependent requests.
- Any enrollment operation while the system of record is unreachable: 503 with a
  stable code — never a 500, never a silent 401.
- Errors never leak backend internals (connection strings, storage details) in
  responses or logs (foundation FR-009 posture).

## Requirements *(mandatory)*

### Functional Requirements

- **FR-001**: The system MUST allow an authenticated organization member to create
  an enrollment for a course of their organization; the enrollment starts with
  status "enrolled".
- **FR-002**: The system MUST reject a duplicate enrollment (same learner, same
  course) with a stable error code, and exactly one enrollment per (learner,
  course) MUST exist — including under concurrent attempts.
- **FR-003**: The system MUST enforce organization scope on every enrollment
  operation: a session scoped to organization A MUST NOT be able to create, read,
  or complete enrollments tied to organization B, in either direction.
- **FR-004**: The system MUST allow a member to list their own enrollments
  (course identifier + status) and an organization admin to list the
  organization's enrollments; the organization-wide view MUST NOT be
  accessible to non-admins. Both views MUST be organization-scoped.
- **FR-005**: The system MUST allow completion to be recorded for an enrollment
  with status "enrolled", transitioning it to "completed" and persisting a
  completion record with a timestamp.
- **FR-006**: The system MUST reject a completion for an enrollment that is not in
  status "enrolled" with a stable error code; the first completion MUST be
  preserved unchanged.
- **FR-007**: Enrollments and completions MUST be durable in the system of
  record (survive process restart and session-store flush).
- **FR-008**: All enrollment responses MUST use the foundation error envelope with
  stable codes (401/403/404/503 mapped to the failure class), and MUST never
  return a 500 for a handled failure or leak backend internals.
- **FR-009**: The slice MUST introduce no module-to-module crate dependencies —
  the module-boundary check MUST stay green, proven by the negative test.
- **FR-010**: The slice MUST be provable without external services (deterministic
  tests) and MUST include a full-environment test that runs against live services
  only when they are present (a SKIP is not a pass).

### Key Entities *(include if feature involves data)*

- **Enrollment**: a learner's enrollment in a course; attributes: identifier,
  learner, course, organization scope, status (enrolled | completed). At most one
  per (learner, course).
- **Completion**: the completed event of an enrollment; attributes: the enrollment
  it completes, the completed-at timestamp. At most one per enrollment; the first
  is the source of truth.
- **Course** (consumed, owned by spec 003): existence, organization ownership,
  and title as needed to create and display enrollments.
- **User / Organization / Membership role** (consumed, owned by spec 001):
  identity, organization scope, and the admin|member role driving FR-004.

## Success Criteria *(mandatory)*

### Measurable Outcomes

- **SC-001**: 100% of the US1–US3 acceptance scenarios are verified by test, not
  by manual observation.
- **SC-002**: 100% of cross-organization attempts in the test matrix are rejected
  (scope holds in both directions — no cross-org enrollment creation, read, or
  completion).
- **SC-003**: A simulated concurrent duplicate enrollment leaves exactly one
  enrollment (race-safety proven by test).
- **SC-004**: The deterministic gate (build + clippy warnings-as-errors + fmt +
  tests + module-boundary check) is green on a clean checkout on both machines;
  the full-environment slice test runs green against live services on the machine
  that has them (Principle XIII/XVI evidence).
- **SC-005**: Zero module→module crate dependencies — the boundary check passes on
  the clean tree and its negative test detects an intentionally introduced
  violation.

## Assumptions

- Course data (existence, organization ownership, title) is provided by the
  Catalog slice (spec 003). This slice consumes course data through a port that
  the host wires; deterministic tests use a fixture course source, and the
  production wiring is resolved at Phase 4 integration. This slice does NOT
  depend on the catalog crate (Principle III; CR-2 seam from spec 001).
- The enrollment lifecycle is two-state in this slice (enrolled → completed).
  Dropping/withdrawing an enrollment, re-enrollment after completion, and
  waitlists are out of scope (later slices).
- Completion in this slice is recorded through the slice's own interface; the
  SCORM-driven completion flow (spec 005) drives the same transition in a later
  slice and does not change it.
- The authentication, session, and organization-scope mechanisms are inherited
  from the foundation (spec 001); this slice adds no new auth surface. The
  seeder development user (admin of the development organization) is used for
  manual smoke testing.
- The learner-facing view is a minimal server-rendered page plus JSON endpoints,
  on the foundation's rendering/error mechanisms; no client-side JS framework.
- This is a teaching exercise: correctness, boundary discipline, and the two-agent
  protocol are the goals — no throughput or scale targets.
