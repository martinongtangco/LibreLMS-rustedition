-- 0002_seed — development identity (spec Assumptions). Idempotent: only inserts
-- when the dev user is absent. Real user provisioning is a later slice.

IF NOT EXISTS (SELECT 1 FROM dbo.users WHERE email = N'dev@example.com')
BEGIN
    DECLARE @user UNIQUEIDENTIFIER = NEWID();
    DECLARE @org UNIQUEIDENTIFIER = NEWID();
    INSERT INTO dbo.organizations (id, name) VALUES (@org, N'Dev Org');
    -- argon2id hash of "dev-password" (dev only; never a production credential)
    INSERT INTO dbo.users (id, email, name, password_hash, theme, created_at)
    VALUES (@user, N'dev@example.com', N'Dev', N'dev-password-hash-placeholder', N'default', SYSDATETIME());
    INSERT INTO dbo.user_organizations (user_id, organization_id, role)
    VALUES (@user, @org, N'admin');
END
