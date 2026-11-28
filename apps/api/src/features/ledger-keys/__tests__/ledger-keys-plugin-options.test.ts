/**
 * The real api-key plugin, configured with the shipped options, must accept
 * every input the management routes accept: names up to 64 characters and
 * lifetimes from 1 to 365 days. It must also store a `start` that tells keys
 * apart (`drn_ro_` + 4 characters), not just the prefix.
 */
import { apiKey } from "@better-auth/api-key";
import { betterAuth } from "better-auth";
import { memoryAdapter } from "better-auth/adapters/memory";
import { describe, expect, it } from "vitest";
import {
	READ_ONLY_LEDGER_KEY_PREFIX,
	readOnlyLedgerApiKeyOptions,
	readOnlyLedgerKeyPermissions,
} from "../../auth/lib/read-only-ledger-key";

const DAY_SECONDS = 24 * 60 * 60;

async function buildAuthWithUser() {
	const memoryAuth = betterAuth({
		baseURL: "http://localhost:3000",
		secret: "test-only-secret-for-in-memory-auth-0123456789",
		database: memoryAdapter({
			user: [],
			session: [],
			account: [],
			verification: [],
			apikey: [],
		}),
		emailAndPassword: { enabled: true },
		plugins: [apiKey(readOnlyLedgerApiKeyOptions)],
	});
	const signUp = await memoryAuth.api.signUpEmail({
		body: {
			email: "owner@example.test",
			password: "correct-horse-battery-staple",
			name: "Owner",
		},
	});
	return { memoryAuth, userId: signUp.user.id };
}

function issueWith(
	memoryAuth: Awaited<ReturnType<typeof buildAuthWithUser>>["memoryAuth"],
	userId: string,
	name: string | undefined,
	expiresInDays: number,
) {
	return memoryAuth.api.createApiKey({
		body: {
			userId,
			...(name === undefined ? {} : { name }),
			prefix: READ_ONLY_LEDGER_KEY_PREFIX,
			permissions: readOnlyLedgerKeyPermissions(),
			metadata: { companyId: "cmp-a" },
			expiresIn: expiresInDays * DAY_SECONDS,
		},
	});
}

describe("api-key plugin options for ledger key management", () => {
	it("accepts a 64-character name and the 1 and 365 day bounds", async () => {
		const { memoryAuth, userId } = await buildAuthWithUser();
		for (const days of [1, 365]) {
			const created = await issueWith(memoryAuth, userId, "n".repeat(64), days);
			expect(created.name).toBe("n".repeat(64));
		}
	});

	it("refuses a nameless key and a name over 64 characters", async () => {
		const { memoryAuth, userId } = await buildAuthWithUser();
		await expect(issueWith(memoryAuth, userId, undefined, 1)).rejects.toThrow();
		await expect(
			issueWith(memoryAuth, userId, "n".repeat(65), 1),
		).rejects.toThrow();
	});

	it("stores the prefix plus 4 characters as start", async () => {
		const { memoryAuth, userId } = await buildAuthWithUser();
		const created = await issueWith(memoryAuth, userId, "laptop", 90);
		expect(created.start).toBe(created.key.slice(0, 11));
		expect(created.start?.startsWith(READ_ONLY_LEDGER_KEY_PREFIX)).toBe(true);
		expect(created.start).toHaveLength(READ_ONLY_LEDGER_KEY_PREFIX.length + 4);
	});
});
