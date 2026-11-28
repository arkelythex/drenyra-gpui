import type { ApiKeyConfigurationOptions } from "@better-auth/api-key";

/**
 * Read-only ledger keys — Better Auth api-key plugin configuration.
 *
 * A read-only ledger key is a personal token for the native app, the CLI and
 * agents. It carries `ledger:read`, is bound to exactly one company through
 * its metadata, and is accepted only on the allowlisted ledger GET routes for
 * that company (see `features/security/read-only-ledger-key.ts`).
 *
 * Security choices pinned here (and in `read-only-ledger-key-config.test.ts`):
 * - Keys are stored hashed (plugin default; `disableKeyHashing: false`).
 * - Keys never become sessions: `enableSessionForAPIKeys: false`, so
 *   `auth.api.getSession` ignores them.
 * - The plugin's own HTTP endpoints are disabled
 *   (`READ_ONLY_LEDGER_KEY_DISABLED_HTTP_PATHS`): over HTTP the plugin lets a
 *   client set `metadata` and `prefix`, so issuance stays server-side, where
 *   permissions and the company binding are decided.
 * - Rate limiting is on for every key.
 * - Keys expire after 90 days by default and at most after 365 days.
 * - Every key has a name of at most 64 characters (the management routes
 *   validate it first; the plugin enforces the same bounds).
 * - The stored `start` is the prefix plus 4 random characters, enough to
 *   tell keys apart in a list without revealing a usable part of the key.
 *
 * @module auth/lib/read-only-ledger-key
 */

/** Prefix of every read-only ledger key (`drn_ro_…`). */
export const READ_ONLY_LEDGER_KEY_PREFIX = "drn_ro_";

/** Resource and action a key needs for the read-only ledger routes. */
export const READ_ONLY_LEDGER_KEY_RESOURCE = "ledger";
export const READ_ONLY_LEDGER_KEY_ACTION = "read";

const SECONDS_PER_DAY = 24 * 60 * 60;

/** Default key lifetime: 90 days, in seconds (the plugin reads seconds). */
export const READ_ONLY_LEDGER_KEY_DEFAULT_EXPIRES_IN_SECONDS =
	90 * SECONDS_PER_DAY;

/** Longest lifetime a caller may request, in days. */
export const READ_ONLY_LEDGER_KEY_MAX_EXPIRES_IN_DAYS = 365;

/** Longest key name the plugin accepts (it measures `name.length`). */
export const READ_ONLY_LEDGER_KEY_MAX_NAME_LENGTH = 64;

/** Characters of the key kept in clear as `start`: `drn_ro_` + 4. */
export const READ_ONLY_LEDGER_KEY_START_LENGTH =
	READ_ONLY_LEDGER_KEY_PREFIX.length + 4;

/** Rate limit per key: 120 verifications per minute. */
export const READ_ONLY_LEDGER_KEY_RATE_LIMIT = {
	timeWindowMs: 60 * 1000,
	maxRequests: 120,
} as const;

/**
 * Better Auth paths of the api-key plugin that must not be reachable over
 * HTTP. Server code still calls them through `auth.api.*`.
 */
export const READ_ONLY_LEDGER_KEY_DISABLED_HTTP_PATHS = [
	"/api-key/create",
	"/api-key/get",
	"/api-key/update",
	"/api-key/delete",
	"/api-key/list",
] as const;

/**
 * Permissions a key must carry, as the plugin's `verifyApiKey` expects them.
 * Returns a fresh object so callers cannot mutate a shared value.
 */
export function readOnlyLedgerKeyPermissions(): Record<string, string[]> {
	return { [READ_ONLY_LEDGER_KEY_RESOURCE]: [READ_ONLY_LEDGER_KEY_ACTION] };
}

/** Options passed to `apiKey(...)` in `auth.config.ts`. */
export const readOnlyLedgerApiKeyOptions = {
	defaultPrefix: READ_ONLY_LEDGER_KEY_PREFIX,
	apiKeyHeaders: "x-api-key",
	references: "user",
	disableKeyHashing: false,
	enableSessionForAPIKeys: false,
	enableMetadata: true,
	requireName: true,
	minimumNameLength: 1,
	maximumNameLength: READ_ONLY_LEDGER_KEY_MAX_NAME_LENGTH,
	startingCharactersConfig: {
		shouldStore: true,
		charactersLength: READ_ONLY_LEDGER_KEY_START_LENGTH,
	},
	keyExpiration: {
		defaultExpiresIn: READ_ONLY_LEDGER_KEY_DEFAULT_EXPIRES_IN_SECONDS,
		minExpiresIn: 1,
		maxExpiresIn: READ_ONLY_LEDGER_KEY_MAX_EXPIRES_IN_DAYS,
	},
	rateLimit: {
		enabled: true,
		timeWindow: READ_ONLY_LEDGER_KEY_RATE_LIMIT.timeWindowMs,
		maxRequests: READ_ONLY_LEDGER_KEY_RATE_LIMIT.maxRequests,
	},
} satisfies ApiKeyConfigurationOptions;
