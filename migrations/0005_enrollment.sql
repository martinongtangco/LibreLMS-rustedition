-- 0005_enrollment — enrollments + completions (spec 004, Module Persistence Seam).
-- Idempotent whole-script batch (spec 001 CR-E: never split on ';').
-- No cross-module FKs (plan decision: an FK to dbo.courses would make this
-- slice's migration DDL-order-dependent on spec 003; integrity is the
-- CourseSource check at write time, organization_id is denormalized).
-- Timestamps: unix seconds (004's own convention — 001 uses DATETIME2; the
-- column type is a module choice, no DDL coupling).
IF OBJECT_ID(N'dbo.enrollments', N'U') IS NULL
BEGIN
    CREATE TABLE dbo.enrollments (
        id              NVARCHAR(40) NOT NULL PRIMARY KEY,
        learner_id      UNIQUEIDENTIFIER NOT NULL,
        course_id       NVARCHAR(40) NOT NULL,
        organization_id NVARCHAR(40) NOT NULL,
        status          NVARCHAR(20) NOT NULL,
        created_at      BIGINT     NOT NULL
    );
    -- The SC-003 race-safety anchor: the engine is the final arbiter under
    -- concurrency (plan PQ-1).
    CREATE UNIQUE INDEX uq_enrollments_learner_course
        ON dbo.enrollments (learner_id, course_id);
END
IF OBJECT_ID(N'dbo.completions', N'U') IS NULL
BEGIN
    CREATE TABLE dbo.completions (
        -- PK enforces "at most one completion per enrollment" in the engine
        -- (first-completion-wins, spec US3.2; no UPDATE path).
        enrollment_id NVARCHAR(40) NOT NULL PRIMARY KEY,
        completed_at  BIGINT       NOT NULL
    );
END
