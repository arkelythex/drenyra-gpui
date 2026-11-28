-- TEST-ONLY. Never apply to a shared, staging or production database.
--
-- Prepares a DISPOSABLE database for
-- apps/api/src/features/ledger-keys/__tests__/integration/ledger-keys.integration.test.ts
--
-- Why: the repo's journaled migration chain does not apply on a clean database
-- today (see odd/tasks/gpui-ledger-readonly-keys.md, L4), so the test database
-- is built from 0000_canonical_baseline.sql + 0035_auth_api_keys.sql. Those two
-- files lack auth columns that the current drizzle schema
-- (packages/persistence/src/schema/auth.schema.ts) selects, and no migration in
-- the repo adds them (pre-existing schema drift, recorded, not fixed here):
--   auth_users: MFA columns; auth_sessions: mfa_verified;
--   auth_user_companies: membership_status.
-- Better Auth's drizzle adapter and the membership query read them, so the
-- integration test needs them. Every statement is additive and idempotent.

ALTER TABLE auth_users ADD COLUMN IF NOT EXISTS totp_secret text;
ALTER TABLE auth_users ADD COLUMN IF NOT EXISTS totp_enabled boolean NOT NULL DEFAULT false;
ALTER TABLE auth_users ADD COLUMN IF NOT EXISTS totp_verified_at timestamp;
ALTER TABLE auth_users ADD COLUMN IF NOT EXISTS recovery_codes jsonb;
ALTER TABLE auth_users ADD COLUMN IF NOT EXISTS mfa_failure_count integer NOT NULL DEFAULT 0;
ALTER TABLE auth_users ADD COLUMN IF NOT EXISTS mfa_last_failure_at timestamp;
ALTER TABLE auth_users ADD COLUMN IF NOT EXISTS mfa_method text DEFAULT '';

ALTER TABLE auth_sessions ADD COLUMN IF NOT EXISTS mfa_verified boolean NOT NULL DEFAULT false;

ALTER TABLE auth_user_companies ADD COLUMN IF NOT EXISTS membership_status varchar(20) NOT NULL DEFAULT 'active';
