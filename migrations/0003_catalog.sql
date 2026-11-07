-- 0003_catalog — courses table (spec 003, Module Persistence Seam).
-- Idempotent whole-script batch (spec 001 CR-E: never split on ';').
IF OBJECT_ID(N'dbo.courses', N'U') IS NULL
BEGIN
    CREATE TABLE dbo.courses (
        id              UNIQUEIDENTIFIER NOT NULL PRIMARY KEY,
        organization_id UNIQUEIDENTIFIER NOT NULL,
        title           NVARCHAR(200)  NOT NULL,
        description     NVARCHAR(MAX)  NULL,  -- CR-1: 5000-char cap is app-level (FR-007); non-MAX NVARCHAR maxes at 4000 (error 2717)
        created_at      DATETIME2      NOT NULL,
        updated_at      DATETIME2      NOT NULL
    );
    CREATE INDEX idx_courses_org ON dbo.courses (organization_id);
END
