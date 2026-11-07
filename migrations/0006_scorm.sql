-- 0006_scorm — scorm_attempts (spec 005, Module Persistence Seam).
-- Idempotent whole-script batch (spec 001 CR-E: never split on ';').
-- No cross-module FKs (004 decision parity: an FK to dbo.courses would make
-- this slice's migration DDL-order-dependent on spec 003; integrity is the
-- EnrollmentPort check at launch time, organization_id is denormalized).
-- Wire discipline (Agent A plan-level requirement, 003/004 parity): id is a
-- NEWID string in NVARCHAR(40); NO UNIQUEIDENTIFIER columns in this table
-- (the learner is implicit in the 001 session — nothing here is read as
-- &str from a Guid column; the 003 CR-7/CR-8 class is impossible by
-- construction).
-- Timestamps: unix seconds (005's own convention — 001 uses DATETIME2; the
-- column type is a module choice, no DDL parity claim).
IF OBJECT_ID(N'dbo.scorm_attempts', N'U') IS NULL
BEGIN
    CREATE TABLE dbo.scorm_attempts (
        id              NVARCHAR(40) NOT NULL PRIMARY KEY,
        -- The 001 session token_hash (session-scoped: one attempt per
        -- (session, course); a new login is a new attempt).
        session_id      NVARCHAR(256) NOT NULL,
        course_id       NVARCHAR(40) NOT NULL,
        organization_id NVARCHAR(40) NOT NULL,
        -- The five-value cmi.core.lesson_status subset (spec 005 FR-004).
        state           NVARCHAR(20) NOT NULL,
        -- Opaque text; app-level <= 4,000 chars (the engine cap on non-MAX
        -- NVARCHAR is 4,000 — spec 002 CR-1 lesson; injection-shaped values
        -- are stored/compared as plain text, FR-012).
        suspend_data    NVARCHAR(MAX) NOT NULL,
        -- Raw 0-100; scaled = raw/100 is computed, not stored (FR-004).
        score_raw       INT NULL,
        -- Accumulated seconds.
        session_time    BIGINT NOT NULL,
        -- Plan decision (Principle X/IV): NULL = live attempt, set = final —
        -- the app-layer attempt-final discriminator (FR-005).
        finished_at     BIGINT NULL,
        created_at      BIGINT NOT NULL,
        updated_at      BIGINT NOT NULL
    );
    -- The US1.6 launch-race engine anchor (2601/2627 on both error legs):
    -- concurrent launches of the same (session, course) are arbitrated here;
    -- the losing inserter reads the winner's row (200 resume, never 409).
    CREATE UNIQUE INDEX uq_scorm_attempts_session_course
        ON dbo.scorm_attempts (session_id, course_id);
END
