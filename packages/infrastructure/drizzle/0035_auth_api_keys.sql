-- Read-only ledger keys — storage for the Better Auth api-key plugin.
--
-- Creates "auth_api_keys" (Better Auth model `apikey`, mapped in
-- apps/api/src/features/auth/auth.config.ts) with the columns that
-- @better-auth/api-key@1.6.23 requires. "key" holds only the SHA-256 hash of
-- the key; "permissions" and "metadata" are JSON strings written by the
-- plugin ("metadata"."companyId" binds a key to one company).
--
-- Additive only: no existing table, column or row changes. Keys are deleted
-- with their owner (ON DELETE CASCADE on auth_users).
--
-- Column definitions are drizzle-kit output for `authApiKeys`
-- (packages/persistence/src/schema/auth.schema.ts). Written idempotent like
-- 0019–0034 because `drizzle-kit generate` cannot diff this tree
-- non-interactively (the 0018 snapshot predates hand-written migrations).
--
-- Idempotent: safe to apply more than once.

CREATE TABLE IF NOT EXISTS "auth_api_keys" (
	"id" text PRIMARY KEY NOT NULL,
	"config_id" text DEFAULT 'default' NOT NULL,
	"name" text,
	"start" text,
	"reference_id" text NOT NULL,
	"prefix" text,
	"key" text NOT NULL,
	"refill_interval" bigint,
	"refill_amount" integer,
	"last_refill_at" timestamp,
	"enabled" boolean DEFAULT true,
	"rate_limit_enabled" boolean DEFAULT true,
	"rate_limit_time_window" bigint,
	"rate_limit_max" integer,
	"request_count" integer DEFAULT 0,
	"remaining" integer,
	"last_request" timestamp,
	"expires_at" timestamp,
	"created_at" timestamp NOT NULL,
	"updated_at" timestamp NOT NULL,
	"permissions" text,
	"metadata" text
);
--> statement-breakpoint

DO $$
BEGIN
  IF NOT EXISTS (
    SELECT 1 FROM pg_constraint
    WHERE conname = 'auth_api_keys_reference_id_auth_users_id_fk'
  ) THEN
    ALTER TABLE "auth_api_keys"
      ADD CONSTRAINT "auth_api_keys_reference_id_auth_users_id_fk"
      FOREIGN KEY ("reference_id") REFERENCES "public"."auth_users"("id")
      ON DELETE cascade ON UPDATE no action;
  END IF;
END
$$;
--> statement-breakpoint

CREATE INDEX IF NOT EXISTS "auth_api_keys_config_id_idx" ON "auth_api_keys" USING btree ("config_id");
--> statement-breakpoint
CREATE INDEX IF NOT EXISTS "auth_api_keys_reference_id_idx" ON "auth_api_keys" USING btree ("reference_id");
--> statement-breakpoint
CREATE INDEX IF NOT EXISTS "auth_api_keys_key_idx" ON "auth_api_keys" USING btree ("key");
