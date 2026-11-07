# Feature Specification: SCORM Attempt Lifecycle (005)

**Branch**: `main` (spec authored on main per Principle IX; implementation on
`story/005-scorm`) | **Date**: 2026-07-22 | **Lane**: Agent B (Phase 3)

## User Scenarios & Acceptance (testable)

A learner works through SCORM content for a course they are enrolled in. The
module owns a thin, testable slice of the SCORM standard: a session-scoped
attempt with a live `cmi.*` bag in the session store, committed state in the
system of record, and the finish that drives the 004
`enrolled → completed` transition (the 004 engine's first-completion-wins
guarantee, consumed through a host-wired port — no crate dependency on the
enrollment module). The frozen 001 ground truth this slice must respect:
the live bag lives in Valkey at the reserved key
`lms:session:{token_hash}:bag` (ephemeral by design — a store flush loses
only the bag; the session row survives), and the **committed** state
(final status, score, suspend data, session time) persists to MSSQL on
LMSCommit/LMSFinish (Principle VI); the `ScormAttemptStatus` shape is the
frozen `contracts` placeholder.

### User Story 1 - A learner launches, works, commits, and finishes a SCORM attempt (Priority: P1)

**FR-001**: The system MUST allow a learner (a member of organization O,
enrolled in course C of O) to launch a SCORM attempt for C (LMSInitialize),
returning 200 with the attempt state. Launch is **idempotent per
(session, course)**: a second launch in the same session resumes the
existing attempt — no second attempt is ever created. Concurrent launches of
the same (session, course) are arbitrated by the engine's UNIQUE constraint:
exactly one attempt row exists, the losing inserter reads the winner's row
and proceeds with it (200, never a 409, never a 500, never a second row —
the 004 PQ-1 discipline; the deterministic duplicate test is labeled
LOGIC-LAYER, the real-engine proof rides the gated full-env test with N
independent connections).

**FR-002**: Launch is gated on the enrollment port (the 004 `CourseSource`
parity, host-wired): unknown course → 404 `not-found`; the course exists
but not in the learner's organization → 403 `scope` (content-identical to
any other 403 — no existence oracle, 003 US3.2 parity); the learner is not
enrolled in C → 409 `not-enrolled`; the enrollment is already `completed` →
409 `already-completed` (a new completion can never be recorded — the 004
first-completion-wins guarantee; the prior committed attempt state remains
the record). An enrollment-port outage → 503, never `Ok(None)`-as-404 (the
004 N1 discipline).

**FR-003**: The live `cmi.*` bag MUST live in the session store at the
001-reserved key (`lms:session:{token_hash}:bag`) and be ephemeral by
design. The module MUST own the bag-store port (trait; the production
implementation wraps the shared session store, deterministic tests use an
in-memory double).

**FR-004**: The module MUST support the thin SCORM API subset —
LMSInitialize (launch/resume), LMSGetValue, LMSSetValue, LMSCommit,
LMSFinish — over the `cmi` data subset: `cmi.core.lesson_status`
(`in_progress` | `passed` | `failed` | `completed` | `incomplete`),
`cmi.core.score` (raw 0–100; scaled = raw/100, computed not stored),
`cmi.suspend_data` (opaque text), and session time (seconds, accumulated).
Values outside the subset are rejected with 400 naming the element.

**FR-005**: LMSCommit MUST persist the committed state (state,
suspend_data, score_raw, session_time) to the system of record
(`dbo.scorm_attempts`); the bag remains live in the session store.
LMSFinish MUST finalize the attempt: `passed`/`completed` final states drive
the 004 `enrolled → completed` transition (first completion wins; the
original `completed_at` is preserved unchanged — the 004 engine's
`dbo.completions` PK guarantee, consumed through the port) and persist the
final attempt state; `failed`/`incomplete` final states persist the attempt
without recording a completion. After LMSFinish the attempt is final:
further LMSSetValue/LMSCommit on it are rejected with 409
`attempt-final`.

**FR-006**: Resume after bag loss: an LMSInitialize after the session-store
bag was lost (flush/outage recovery) rehydrates the bag from the last
LMSCommit in the system of record (state, suspend_data, score, session
time — the 001 story-3 scenario-4 parity); if no commit exists, a fresh
`in_progress` attempt begins.

**Acceptance Scenarios**

1. **Given** an enrolled learner, **When** they launch course C, **When**
   they set `cmi.core.lesson_status=in_progress` and LMSCommit, **When**
   they LMSFinish with `passed`, **Then** the attempt row is `passed` with
   the score/suspend_data/session time persisted, and the enrollment is
   `completed` with a completion timestamp.
2. **Given** the same session, **When** they launch C again after the
   finish, **Then** 409 `already-completed`.
3. **Given** a launched attempt that was committed, **When** the session
   store flushes the bag and the learner re-launches, **Then** the bag is
   rehydrated from the last commit (suspend_data and score intact).
4. **Given** an enrollment that is already `completed` (any prior path),
   **When** the learner launches C, **Then** 409 `already-completed` — no
   new attempt, no second completion.
5. **Given** a learner not enrolled in C, **When** they launch C, **Then**
   409 `not-enrolled` (stable code; never a 404 — no existence oracle).
6. **Given** N concurrent launches of the same (session, course) — N
   independent connections on the real engine (gated full-env), **When**
   they race, **Then** exactly one attempt row exists; every caller ends
   with the same attempt; no 500, no 409. (Deterministic analog:
   LOGIC-LAYER.)
7. **Given** a finished attempt, **When** LMSSetValue is attempted, **Then**
   409 `attempt-final`.

### User Story 2 - The module seam (Priority: P2)

**FR-007**: The module MUST own its persistence per the 001 Module
Persistence Seam: an in-crate repository trait + tiberius implementation +
in-memory double, the bag-store port, and its own `0006_scorm` migration
manifest. The module MUST depend on `contracts` + `shared` only (boundary
check, proven by the negative test); it consumes the frozen
`ScormAttemptStatus` shape from `contracts`.

**FR-008**: Enrollment interaction MUST go through a host-wired port
(2-method, the 004 `CourseSource` parity): `enrollment_status(learner_id,
course_id, org_id) -> Result<Option<EnrollmentStatus>, ScormError>` and
`record_completion(learner_id, course_id, org_id) -> Result<...>` —
deterministic tests use an in-memory double; the production wiring (Phase 4)
uses the honest `Unwired` style (outage → 503, never a fake
404/200/completion). No crate dependency on the enrollment module.

**FR-009**: The slice's DDL (`0006_scorm`) creates `dbo.scorm_attempts`
(session-scoped id, course_id, organization_id, state, suspend_data,
score_raw, session_time) with a UNIQUE (session, course) constraint; no
cross-module FKs (004 decision parity). `dbo.completions` is untouched —
004 owns it.

### User Story 3 - Error semantics and robustness (Priority: P3)

**FR-010**: All endpoints return the 001-style JSON envelope
(`{"error": <stable code>}` + status); 401 `unauthenticated` without a
session; 403 `scope` (cross-org course); 404 `not-found` (unknown course);
409 `not-enrolled` / `already-completed` / `attempt-final`; 400 `validation`
(invalid `cmi` value, field named); 503 `storage-unavailable` on
system-of-record OR session-store outage — NEVER a 500, never a silent
state loss, never a fake success.

**FR-011**: Validation battery: `cmi.core.score` raw MUST be an integer in
0–100 (out-of-range/non-numeric → 400 naming the element);
`cmi.core.lesson_status` MUST be one of the five subset values (anything
else → 400); `cmi.suspend_data` MUST be ≤ 4,000 chars (app-level; the
column is `NVARCHAR(MAX)`); unknown `cmi` elements → 400.

**FR-012**: Injection-shaped values (suspend_data containing SQL/JSON
metacharacters) MUST be handled parameterized and stored/compared as plain
text.

**FR-013**: Deterministic + full-environment: the slice MUST be provable
without external services (deterministic tests: the in-memory doubles
exercise launch idempotency, the finish→completion path, resume-after-bag-loss,
and the validation battery), and MUST include a full-environment test that
runs against live services only when present (a SKIP is not a pass —
Principle XVII). The real-engine proof: N independent-connection concurrent
launch (FR-001/US1.6) + migration idempotency + commit durability across
reconnect.

## Assumptions & Out of Scope

- **Thin SCORM subset**: LMSInitialize/LMSGetValue/LMSSetValue/LMSCommit/
  LMSFinish over the five-value `lesson_status`, integer score, suspend_data,
  session time. The full SCORM 2004 API surface, SCORM 1.2 compatibility,
  and `cmi.interactions`/`cmi.objectives` are OUT OF SCOPE.
- **No content packaging**: no .zip/manifest parsing, no content hosting —
  the `cmi` bag IS the content interface in this slice (a follow-up spec
  candidate if the original's package handling is in scope for the port).
- **One attempt per (session, course)**: a new login (new session) is a new
  attempt; the same session never starts a second attempt for a course.
  No pass/fail threshold computation — the content package reports the
  final status; the LMS records it.
- **Session/actor model**: the SCORM endpoints are 001-session-scoped — the
  learner and the organization come from the session (the 001 middleware);
  no org parameter in the request.
- **No module-to-module dependencies**: the enrollment data is consumed
  through the host-wired port (FR-008); the `dbo.completions` row is written
  only by the 004-owned path the port exposes.
- **The 004 port is 2-method** (status + record_completion), mirroring the
  004 `CourseSource` shape; the Phase-4 adapter is ~10 lines over the 004
  repository.

## Error Semantics (envelope mapping)

| condition | status | stable code |
|---|---|---|
| absent session | 401 | `unauthenticated` (foundation) |
| cross-org course | 403 | `scope` (content-identical to any other 403) |
| unknown course | 404 | `not-found` |
| not enrolled in the course | 409 | `not-enrolled` |
| enrollment already completed | 409 | `already-completed` |
| operation on a finished attempt | 409 | `attempt-final` |
| invalid `cmi` value (score/status/suspend_data/element) | 400 | `validation` (field named) |
| storage OR session-store outage | 503 | `storage-unavailable` |
