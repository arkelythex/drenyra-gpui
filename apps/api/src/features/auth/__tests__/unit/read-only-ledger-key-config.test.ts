/**
 * Read-only ledger key configuration of the Better Auth api-key plugin.
 *
 * The production `auth` instance must register the plugin with the shipped
 * options. A second, in-memory instance built from the same options runs the
 * real plugin to prove: keys carry the `drn_ro_` prefix, expire after 90 days,
 * are rate limited, never yield a session through `getSession`, verify with
 * `ledger:read` only, stop verifying once revoked, and cannot be issued over
 * HTTP.
 */
import { apiKey } from "@better-auth/api-key";
import { betterAuth } from "better-auth";
import { memoryAdapter } from "better-auth/adapters/memory";
import { describe, expect, it } from "vitest";
import { auth } from "../../auth.config";
import {
	READ_ONLY_LEDGER_KEY_DEFAULT_EXPIRES_IN_SECONDS,
	READ_ONLY_LEDGER_KEY_DISABLED_HTTP_PATHS,
	READ_ONLY_LEDGER_KEY_PREFIX,
	readOnlyLedgerApiKeyOptions,
	readOnlyLedgerKeyPermissions,
} from "../../lib/read-only-ledger-key";

const BASE_URL = "http://localhost:3000";

function buildInMemoryAuth() {
	return betterAuth({
		baseURL: BASE_URL,
		secret: "test-only-secret-for-in-memory-auth-0123456789",
		database: memoryAdapter({
			user: [],
			session: [],
			account: [],
			verification: [],
			apikey: [],
		}),
		emailAndPassword: { enabled: true },
		disabledPaths: [...READ_ONLY_LEDGER_KEY_DISABLED_HTTP_PATHS],
		plugins: [apiKey(readOnlyLedgerApiKeyOptions)],
	});
}

async function createUserWithKey() {
	const memoryAuth = buildInMemoryAuth();
	const signUp = await memoryAuth.api.signUpEmail({
		body: {
			email: "owner@example.test",
			password: "correct-horse-battery-staple",
			name: "Owner",
		},
	});
	const userId = signUp.user.id;
	const created = await memoryAuth.api.createApiKey({
		body: {
			userId,
			name: "gpui",
			permissions: readOnlyLedgerKeyPermissions(),
			metadata: { companyId: "cmp-a" },
		},
	});
	return { memoryAuth, userId, created };
}

describe("read-only ledger key configuration", () => {
	it("pins the plugin options", () => {
		expect(readOnlyLedgerApiKeyOptions).toMatchObject({
			defaultPrefix: "drn_ro_",
			apiKeyHeaders: "x-api-key",
			references: "user",
			disableKeyHashing: false,
			enableSessionForAPIKeys: false,
			enableMetadata: true,
			keyExpiration: {
				defaultExpiresIn: 90 * 24 * 60 * 60,
				minExpiresIn: 1,
				maxExpiresIn: 365,
			},
			rateLimit: { enabled: true, timeWindow: 60_000, maxRequests: 120 },
		});
		expect(readOnlyLedgerKeyPermissions()).toEqual({ ledger: ["read"] });
	});

	it("registers the api-key plugin and disables its HTTP paths on the production auth instance", () => {
		const plugins = auth.options.plugins ?? [];
		expect(plugins.map((plugin) => plugin.id)).toContain("api-key");
		expect(auth.options.disabledPaths).toEqual(
			expect.arrayContaining([...READ_ONLY_LEDGER_KEY_DISABLED_HTTP_PATHS]),
		);
		expect(typeof auth.api.verifyApiKey).toBe("function");
		expect(typeof auth.api.createApiKey).toBe("function");
	});

	describe("real plugin, in memory", () => {
		it("issues hashed, prefixed, rate-limited keys that expire after 90 days", async () => {
			const before = Date.now();
			const { created, userId } = await createUserWithKey();

			expect(created.key.startsWith(READ_ONLY_LEDGER_KEY_PREFIX)).toBe(true);
			expect(created.referenceId).toBe(userId);
			expect(created.rateLimitEnabled).toBe(true);
			expect(created.rateLimitMax).toBe(120);
			expect(created.rateLimitTimeWindow).toBe(60_000);
			const expiresAt = new Date(created.expiresAt ?? 0).getTime();
			const expected =
				before + READ_ONLY_LEDGER_KEY_DEFAULT_EXPIRES_IN_SECONDS * 1000;
			expect(Math.abs(expiresAt - expected)).toBeLessThan(60_000);
		});

		it("never turns a key into a session", async () => {
			const { memoryAuth, created } = await createUserWithKey();

			const viaApiKeyHeader = await memoryAuth.api.getSession({
				headers: new Headers({ "x-api-key": created.key }),
			});
			const viaBearer = await memoryAuth.api.getSession({
				headers: new Headers({ authorization: `Bearer ${created.key}` }),
			});

			expect(viaApiKeyHeader).toBeNull();
			expect(viaBearer).toBeNull();
		});

		it("verifies with ledger:read and returns the owner, company metadata and permissions", async () => {
			const { memoryAuth, created, userId } = await createUserWithKey();

			const result = await memoryAuth.api.verifyApiKey({
				body: { key: created.key, permissions: readOnlyLedgerKeyPermissions() },
			});

			expect(result.valid).toBe(true);
			expect(result.key?.referenceId).toBe(userId);
			expect(result.key?.metadata).toEqual({ companyId: "cmp-a" });
			expect(result.key?.permissions).toEqual({ ledger: ["read"] });
		});

		it("refuses a permission the key does not carry", async () => {
			const { memoryAuth, created } = await createUserWithKey();

			const result = await memoryAuth.api.verifyApiKey({
				body: { key: created.key, permissions: { ledger: ["write"] } },
			});

			expect(result.valid).toBe(false);
			expect(result.key).toBeNull();
		});

		it("stops verifying a revoked key", async () => {
			const { memoryAuth, created, userId } = await createUserWithKey();

			await memoryAuth.api.updateApiKey({
				body: { keyId: created.id, userId, enabled: false },
			});
			const result = await memoryAuth.api.verifyApiKey({
				body: { key: created.key, permissions: readOnlyLedgerKeyPermissions() },
			});

			expect(result.valid).toBe(false);
			expect(result.error?.code).toBe("KEY_DISABLED");
		});

		it("cannot issue keys over HTTP", async () => {
			const memoryAuth = buildInMemoryAuth();

			const response = await memoryAuth.handler(
				new Request(`${BASE_URL}/api/auth/api-key/create`, {
					method: "POST",
					headers: { "content-type": "application/json" },
					body: JSON.stringify({ name: "x", metadata: { companyId: "cmp-b" } }),
				}),
			);

			expect(response.status).toBe(404);
		});
	});
});
