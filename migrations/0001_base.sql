-- 0001_base — base schema (data-model.md). Idempotent by design (research R1).
-- Executed by the startup migration runner inside a transaction where supported.

IF OBJECT_ID(N'dbo.schema_migrations', N'U') IS NULL
CREATE TABLE dbo.schema_migrations (
    name NVARCHAR(200) PRIMARY KEY,
    checksum NVARCHAR(64) NOT NULL,
    applied_at DATETIME2 NOT NULL
);

IF OBJECT_ID(N'dbo.users', N'U') IS NULL
CREATE TABLE dbo.users (
    id UNIQUEIDENTIFIER PRIMARY KEY,
    email NVARCHAR(320) NOT NULL UNIQUE,
    name NVARCHAR(200) NOT NULL,
    password_hash NVARCHAR(500) NOT NULL,
    theme NVARCHAR(20) NOT NULL DEFAULT N'default',
    created_at DATETIME2 NOT NULL
);

IF OBJECT_ID(N'dbo.organizations', N'U') IS NULL
CREATE TABLE dbo.organizations (
    id UNIQUEIDENTIFIER PRIMARY KEY,
    name NVARCHAR(200) NOT NULL
);

IF OBJECT_ID(N'dbo.user_organizations', N'U') IS NULL
CREATE TABLE dbo.user_organizations (
    user_id UNIQUEIDENTIFIER NOT NULL REFERENCES dbo.users(id),
    organization_id UNIQUEIDENTIFIER NOT NULL REFERENCES dbo.organizations(id),
    role NVARCHAR(40) NOT NULL CHECK (role IN (N'admin', N'member')),
    PRIMARY KEY (user_id, organization_id)
);

IF OBJECT_ID(N'dbo.sessions', N'U') IS NULL
CREATE TABLE dbo.sessions (
    token_hash NVARCHAR(64) PRIMARY KEY,
    user_id UNIQUEIDENTIFIER NOT NULL REFERENCES dbo.users(id),
    organization_id UNIQUEIDENTIFIER NOT NULL REFERENCES dbo.organizations(id),
    created_at DATETIME2 NOT NULL,
    last_activity DATETIME2 NOT NULL,
    expires_at DATETIME2 NOT NULL
);
