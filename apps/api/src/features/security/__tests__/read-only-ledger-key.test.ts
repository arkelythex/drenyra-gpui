/**
 * Read-only ledger keys in the caller resolver.
 *
 * A key (`drn_ro_…`, issued by the Better Auth api-key plugin) is accepted
 * only on routes that opt in with `allowReadOnlyLedgerKey`, only for GET or
 * HEAD, only when no session resolved, only with `ledger:read`, only for the
 * one company bound in its metadata, and only while its owner is still an
 * active member of that company. Everything else is refused or ignored.
 */
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("../../auth/handlers/company-membership", async (importOriginal) => {
	const actual =
		await importOriginal<
			typeof import("../../auth/handlers/company-membership")
		>();
	return { ...actual, listUserCompanyMemberships: vi.fn() };
});

const { auth } = await import("../../auth/auth.config");
const { listUserCompanyMemberships } = await import(
	"../../auth/handlers/company-membership"
);
const { AUTHENTICATED_CALLER_KIND, resolveAuthenticatedCaller } = await import(
	"../authenticated-caller"
);
const { resolveSessionContext } = await import("../session-context");

const OWNER = "auth-user-owner";
const COMPANY_A = "cmp-a";
const COMPANY_B = "cmp-b";
const KEY = "drn_ro_testkeyAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA";

type VerifyApiKey = typeof auth.api.verifyApiKey;

function validKey(overrides: Record<string, unknown> = {}) {
	return {
		valid: true,
		error: null,
		key: {
			id: "key-1",
			referenceId: OWNER,
			permissions: { ledger: ["read"] },
			metadata: { companyId: COMPANY_A },
			...overrides,
		},
	};
}

function membership(companyId: string) {
	return {
		companyId,
		companyName: `Company ${companyId}`,
		ruc: "20123456789",
		countryCode: "PE",
		membershipRole: "OWNER",
		isDefault: true,
	};
}

function bearer(extra: Record<string, string> = {}): Record<string, string> {
	return { authorization: `Bearer ${KEY}`, ...extra };
}

describe("read-only ledger key caller", () => {
	const originalEnv = { ...process.env };
	let verifySpy: ReturnType<typeof vi.fn>;

	beforeEach(() => {
		vi.restoreAllMocks();
		process.env = {
			...originalEnv,
			NODE_ENV: "test",
			SECURITY_ENFORCE_TEST_SESSION: "true",
		};
		vi.spyOn(auth.api, "getSession").mockResolvedValue(null as never);
		verifySpy = vi.fn().mockResolvedValue(validKey());
		vi.spyOn(auth.api, "verifyApiKey").mockImplementation(
			verifySpy as unknown as VerifyApiKey,
		);
		vi.mocked(listUserCompanyMemberships).mockReset();
		vi.mocked(listUserCompanyMemberships).mockResolvedValue([
			membership(COMPANY_A),
		]);
	});

	afterEach(() => {
		vi.restoreAllMocks();
		process.env = { ...originalEnv };
	});

	it("accepts a valid key on an opted-in GET route as a read-only caller bound to the key company", async () => {
		const result = await resolveAuthenticatedCaller({
			headers: bearer(),
			allowReadOnlyLedgerKey: true,
			method: "GET",
		});

		expect(result).toEqual({
			ok: true,
			caller: {
				kind: AUTHENTICATED_CALLER_KIND.API_KEY,
				userId: OWNER,
				authUserId: OWNER,
				legacyUserId: null,
				role: "viewer",
				companyId: COMPANY_A,
				sessionId: null,
				serviceId: null,
				readOnly: true,
				apiKeyId: "key-1",
			},
		});
		expect(verifySpy).toHaveBeenCalledWith({
			body: { key: KEY, permissions: { ledger: ["read"] } },
		});
		expect(listUserCompanyMemberships).toHaveBeenCalledWith(OWNER);
	});

	it("accepts the key from the x-api-key header", async () => {
		const result = await resolveAuthenticatedCaller({
			headers: { "x-api-key": KEY },
			allowReadOnlyLedgerKey: true,
			method: "GET",
		});

		expect(result).toMatchObject({
			ok: true,
			caller: { kind: "api-key", companyId: COMPANY_A, readOnly: true },
		});
	});

	it("accepts HEAD like GET", async () => {
		const result = await resolveAuthenticatedCaller({
			headers: bearer(),
			allowReadOnlyLedgerKey: true,
			method: "HEAD",
		});

		expect(result).toMatchObject({ ok: true, caller: { kind: "api-key" } });
	});

	it.each(["POST", "PUT", "PATCH", "DELETE"])(
		"refuses %s with a key before verifying it",
		async (method) => {
			const result = await resolveAuthenticatedCaller({
				headers: bearer(),
				allowReadOnlyLedgerKey: true,
				method,
			});

			expect(result).toEqual({
				ok: false,
				status: 403,
				code: "API_KEY_READ_ONLY",
				error: "Read-only API keys cannot perform write operations",
			});
			expect(verifySpy).not.toHaveBeenCalled();
		},
	);

	it("refuses a key when the method is unknown", async () => {
		const result = await resolveAuthenticatedCaller({
			headers: bearer(),
			allowReadOnlyLedgerKey: true,
		});

		expect(result).toMatchObject({
			ok: false,
			status: 403,
			code: "API_KEY_READ_ONLY",
		});
		expect(verifySpy).not.toHaveBeenCalled();
	});

	it("ignores the key on routes that do not opt in", async () => {
		const result = await resolveAuthenticatedCaller({
			headers: bearer(),
			method: "GET",
		});

		expect(result).toMatchObject({
			ok: false,
			status: 401,
			code: "SESSION_REQUIRED",
		});
		expect(verifySpy).not.toHaveBeenCalled();
	});

	it("refuses an invalid, expired or revoked key with 401", async () => {
		for (const code of ["KEY_NOT_FOUND", "KEY_EXPIRED", "KEY_DISABLED"]) {
			verifySpy.mockResolvedValueOnce({
				valid: false,
				error: { message: "nope", code },
				key: null,
			});
			const result = await resolveAuthenticatedCaller({
				headers: bearer(),
				allowReadOnlyLedgerKey: true,
				method: "GET",
			});

			expect(result).toEqual({
				ok: false,
				status: 401,
				code: "API_KEY_INVALID",
				error: "API key is invalid, expired or revoked",
			});
		}
		expect(listUserCompanyMemberships).not.toHaveBeenCalled();
	});

	it("refuses a rate-limited key with 429 API_KEY_RATE_LIMITED at the source", async () => {
		for (const code of ["RATE_LIMITED", "USAGE_EXCEEDED"]) {
			verifySpy.mockResolvedValueOnce({
				valid: false,
				error: { message: "slow down", code },
				key: null,
			});
			const result = await resolveAuthenticatedCaller({
				headers: bearer(),
				allowReadOnlyLedgerKey: true,
				method: "GET",
			});

			expect(result).toEqual({
				ok: false,
				status: 429,
				code: "API_KEY_RATE_LIMITED",
				error: "API key rate limit exceeded; retry later",
				retryAfterSeconds: 60,
			});
		}
	});

	it("carries the plugin's tryAgainIn as whole seconds (at least 1)", async () => {
		for (const [tryAgainIn, expected] of [
			[12_345, 13],
			[1, 1],
			[0, 1],
		] as const) {
			verifySpy.mockResolvedValueOnce({
				valid: false,
				error: {
					message: "slow down",
					code: "RATE_LIMITED",
					details: { tryAgainIn },
				},
				key: null,
			});
			const result = await resolveAuthenticatedCaller({
				headers: bearer(),
				allowReadOnlyLedgerKey: true,
				method: "GET",
			});

			expect(result).toMatchObject({
				ok: false,
				code: "API_KEY_RATE_LIMITED",
				retryAfterSeconds: expected,
			});
		}
	});

	it("ignores an x-api-key without the drn_ro_ prefix, as if no key were sent", async () => {
		const result = await resolveAuthenticatedCaller({
			headers: { "x-api-key": "sk_live_other" },
			allowReadOnlyLedgerKey: true,
			method: "GET",
		});
		const withoutKey = await resolveAuthenticatedCaller({
			headers: {},
			allowReadOnlyLedgerKey: true,
			method: "GET",
		});

		expect(result).toEqual(withoutKey);
		expect(result).toMatchObject({
			ok: false,
			status: 401,
			code: "SESSION_REQUIRED",
		});
		expect(verifySpy).not.toHaveBeenCalled();
	});

	it("keeps the header-fallback caller when an unrelated x-api-key is sent without a session", async () => {
		process.env.SECURITY_ENFORCE_TEST_SESSION = "false";
		const identity = {
			"x-user-id": "user-1",
			"x-user-role": "owner",
			"x-company-id": COMPANY_A,
		};
		const input = {
			requireSession: false,
			allowHeaderFallback: true,
			allowReadOnlyLedgerKey: true,
			method: "GET",
		};

		const result = await resolveAuthenticatedCaller({
			...input,
			headers: { ...identity, "x-api-key": "sk_live_other" },
		});
		const baseline = await resolveAuthenticatedCaller({
			...input,
			headers: identity,
		});

		expect(result).toEqual(baseline);
		expect(result).toMatchObject({
			ok: true,
			caller: { kind: "header-fallback", companyId: COMPANY_A },
		});
		expect(verifySpy).not.toHaveBeenCalled();
	});

	it("uses a drn_ro_ bearer key even when an unrelated x-api-key is also sent", async () => {
		const result = await resolveAuthenticatedCaller({
			headers: bearer({ "x-api-key": "sk_live_other" }),
			allowReadOnlyLedgerKey: true,
			method: "GET",
		});

		expect(result).toMatchObject({ ok: true, caller: { kind: "api-key" } });
		expect(verifySpy).toHaveBeenCalledWith({
			body: { key: KEY, permissions: { ledger: ["read"] } },
		});
	});

	it("refuses a key without ledger:read even if the plugin reports it valid", async () => {
		verifySpy.mockResolvedValueOnce(
			validKey({ permissions: { ledger: ["write"] } }),
		);

		const result = await resolveAuthenticatedCaller({
			headers: bearer(),
			allowReadOnlyLedgerKey: true,
			method: "GET",
		});

		expect(result).toEqual({
			ok: false,
			status: 403,
			code: "API_KEY_FORBIDDEN",
			error: "API key does not grant ledger:read",
		});
	});

	it("refuses a key that is not bound to a company", async () => {
		verifySpy.mockResolvedValueOnce(validKey({ metadata: null }));

		const result = await resolveAuthenticatedCaller({
			headers: bearer(),
			allowReadOnlyLedgerKey: true,
			method: "GET",
		});

		expect(result).toEqual({
			ok: false,
			status: 403,
			code: "API_KEY_NOT_COMPANY_BOUND",
			error: "API key is not bound to a company",
		});
	});

	it("refuses a key whose owner is no longer an active member of the key company", async () => {
		vi.mocked(listUserCompanyMemberships).mockResolvedValueOnce([
			membership(COMPANY_B),
		]);

		const result = await resolveAuthenticatedCaller({
			headers: bearer(),
			allowReadOnlyLedgerKey: true,
			method: "GET",
		});

		expect(result).toEqual({
			ok: false,
			status: 403,
			code: "API_KEY_MEMBERSHIP_INACTIVE",
			error: "API key owner is not an active member of the key company",
		});
	});

	const UNAVAILABLE = {
		ok: false,
		status: 503,
		code: "API_KEY_VERIFICATION_UNAVAILABLE",
		error: "API key verification is temporarily unavailable; retry later",
		retryAfterSeconds: 5,
	} as const;

	it("fails closed with 503 API_KEY_VERIFICATION_UNAVAILABLE when verification throws", async () => {
		verifySpy.mockRejectedValueOnce(new Error("db down"));

		const result = await resolveAuthenticatedCaller({
			headers: bearer(),
			allowReadOnlyLedgerKey: true,
			method: "GET",
		});

		expect(result).toEqual(UNAVAILABLE);
		expect(listUserCompanyMemberships).not.toHaveBeenCalled();
	});

	it("fails closed with 503 API_KEY_VERIFICATION_UNAVAILABLE when the membership lookup throws", async () => {
		vi.mocked(listUserCompanyMemberships).mockRejectedValueOnce(
			new Error("db down"),
		);

		const result = await resolveAuthenticatedCaller({
			headers: bearer(),
			allowReadOnlyLedgerKey: true,
			method: "GET",
		});

		expect(result).toEqual(UNAVAILABLE);
	});

	it("never falls back to header identity when verification is unavailable", async () => {
		process.env.SECURITY_ENFORCE_TEST_SESSION = "false";
		verifySpy.mockRejectedValueOnce(new Error("db down"));

		const result = await resolveAuthenticatedCaller({
			headers: bearer({
				"x-user-id": "user-1",
				"x-user-role": "owner",
				"x-company-id": COMPANY_A,
			}),
			requireSession: false,
			allowReadOnlyLedgerKey: true,
			allowHeaderFallback: true,
			method: "GET",
		});

		expect(result).toEqual(UNAVAILABLE);
	});

	describe("tenant isolation", () => {
		it("refuses x-company-id for another company", async () => {
			const result = await resolveAuthenticatedCaller({
				headers: bearer({ "x-company-id": COMPANY_B }),
				allowReadOnlyLedgerKey: true,
				method: "GET",
			});

			expect(result).toEqual({
				ok: false,
				status: 403,
				code: "API_KEY_TENANT_MISMATCH",
				error: "API key company does not match the requested company",
			});
		});

		it("refuses x-active-company-id for another company", async () => {
			const result = await resolveAuthenticatedCaller({
				headers: bearer({ "x-active-company-id": COMPANY_B }),
				allowReadOnlyLedgerKey: true,
				method: "GET",
			});

			expect(result).toMatchObject({
				ok: false,
				status: 403,
				code: "API_KEY_TENANT_MISMATCH",
			});
		});

		it("refuses a requested companyId for another company", async () => {
			const result = await resolveAuthenticatedCaller({
				headers: bearer(),
				requestedCompanyId: COMPANY_B,
				allowReadOnlyLedgerKey: true,
				method: "GET",
			});

			expect(result).toMatchObject({
				ok: false,
				status: 403,
				code: "API_KEY_TENANT_MISMATCH",
			});
		});

		it("refuses a key-only requested companyId (query) for another company", async () => {
			for (const requested of [[COMPANY_B], [COMPANY_A, COMPANY_B], [""]]) {
				const result = await resolveAuthenticatedCaller({
					headers: bearer(),
					readOnlyLedgerKeyRequestedCompanyIds: requested,
					allowReadOnlyLedgerKey: true,
					method: "GET",
				});

				expect(result, JSON.stringify(requested)).toEqual({
					ok: false,
					status: 403,
					code: "API_KEY_TENANT_MISMATCH",
					error: "API key company does not match the requested company",
				});
			}
		});

		it("accepts a key-only requested companyId equal to the key company", async () => {
			const result = await resolveAuthenticatedCaller({
				headers: bearer(),
				readOnlyLedgerKeyRequestedCompanyIds: [COMPANY_A],
				allowReadOnlyLedgerKey: true,
				method: "GET",
			});

			expect(result).toMatchObject({
				ok: true,
				caller: { kind: "api-key", companyId: COMPANY_A },
			});
		});

		it("does not apply the key-only requested companyId to a session caller", async () => {
			vi.mocked(auth.api.getSession).mockResolvedValue({
				session: { id: "sess-1" },
				user: { id: "auth-user-1", role: "admin", activeCompanyId: COMPANY_A },
			} as never);

			const result = await resolveAuthenticatedCaller({
				headers: { cookie: "better-auth.session_token=s" },
				readOnlyLedgerKeyRequestedCompanyIds: [COMPANY_B],
				allowReadOnlyLedgerKey: true,
				method: "GET",
			});

			expect(result).toMatchObject({
				ok: true,
				caller: { kind: "session", companyId: COMPANY_A },
			});
		});

		it("accepts x-company-id equal to the key company", async () => {
			const result = await resolveAuthenticatedCaller({
				headers: bearer({ "x-company-id": COMPANY_A }),
				allowReadOnlyLedgerKey: true,
				method: "GET",
			});

			expect(result).toMatchObject({
				ok: true,
				caller: { companyId: COMPANY_A },
			});
		});

		it("takes identity only from the key, never from identity headers", async () => {
			process.env.SECURITY_ENFORCE_TEST_SESSION = "false";
			const result = await resolveAuthenticatedCaller({
				headers: bearer({
					"x-user-id": "attacker",
					"x-auth-user-id": "attacker",
					"x-user-role": "owner",
				}),
				allowReadOnlyLedgerKey: true,
				allowHeaderFallback: true,
				method: "GET",
			});

			expect(result).toMatchObject({
				ok: true,
				caller: { userId: OWNER, authUserId: OWNER, role: "viewer" },
			});
		});

		it("never falls back to header identity when an opted-in key is refused", async () => {
			process.env.SECURITY_ENFORCE_TEST_SESSION = "false";
			verifySpy.mockResolvedValueOnce({
				valid: false,
				error: { message: "nope", code: "KEY_NOT_FOUND" },
				key: null,
			});

			const result = await resolveAuthenticatedCaller({
				headers: bearer({
					"x-user-id": "user-1",
					"x-user-role": "owner",
					"x-company-id": COMPANY_A,
				}),
				requireSession: false,
				allowReadOnlyLedgerKey: true,
				allowHeaderFallback: true,
				method: "GET",
			});

			expect(result).toMatchObject({
				ok: false,
				status: 401,
				code: "API_KEY_INVALID",
			});
		});
	});

	it("lets a resolved session win over a key without verifying the key", async () => {
		vi.mocked(auth.api.getSession).mockResolvedValue({
			session: { id: "sess-1" },
			user: { id: "auth-user-1", role: "admin", activeCompanyId: COMPANY_B },
		} as never);

		const result = await resolveAuthenticatedCaller({
			headers: {
				cookie: "better-auth.session_token=s",
				authorization: `Bearer ${KEY}`,
			},
			allowReadOnlyLedgerKey: true,
			method: "POST",
		});

		expect(result).toMatchObject({
			ok: true,
			caller: { kind: "session", companyId: COMPANY_B },
		});
		expect(result.ok && result.caller.readOnly).toBeFalsy();
		expect(verifySpy).not.toHaveBeenCalled();
	});

	describe("production NODE_ENV", () => {
		beforeEach(() => {
			process.env.NODE_ENV = "production";
			delete process.env.SECURITY_ENFORCE_TEST_SESSION;
		});

		it("accepts a valid key the same way", async () => {
			const result = await resolveAuthenticatedCaller({
				headers: bearer(),
				allowReadOnlyLedgerKey: true,
				allowHeaderFallback: true,
				requireSession: false,
				method: "GET",
			});

			expect(result).toMatchObject({
				ok: true,
				caller: { kind: "api-key", companyId: COMPANY_A, readOnly: true },
			});
		});

		it("refuses writes, foreign companies and invalid keys the same way", async () => {
			const write = await resolveAuthenticatedCaller({
				headers: bearer(),
				allowReadOnlyLedgerKey: true,
				requireSession: false,
				method: "DELETE",
			});
			const foreign = await resolveAuthenticatedCaller({
				headers: bearer({ "x-company-id": COMPANY_B }),
				allowReadOnlyLedgerKey: true,
				requireSession: false,
				method: "GET",
			});
			verifySpy.mockResolvedValueOnce({
				valid: false,
				error: { message: "nope", code: "KEY_EXPIRED" },
				key: null,
			});
			const invalid = await resolveAuthenticatedCaller({
				headers: bearer(),
				allowReadOnlyLedgerKey: true,
				requireSession: false,
				method: "GET",
			});

			expect(write).toMatchObject({ status: 403, code: "API_KEY_READ_ONLY" });
			expect(foreign).toMatchObject({
				status: 403,
				code: "API_KEY_TENANT_MISMATCH",
			});
			expect(invalid).toMatchObject({ status: 401, code: "API_KEY_INVALID" });
		});

		it("ignores the key on routes that do not opt in", async () => {
			const result = await resolveAuthenticatedCaller({
				headers: bearer(),
				method: "GET",
			});

			expect(result).toMatchObject({ ok: false, code: "SESSION_REQUIRED" });
			expect(verifySpy).not.toHaveBeenCalled();
		});
	});

	describe("resolveSessionContext", () => {
		it("carries readOnly for a key caller", async () => {
			const result = await resolveSessionContext({
				headers: bearer(),
				requireSession: false,
				allowHeaderFallback: true,
				allowReadOnlyLedgerKey: true,
				method: "GET",
			});

			expect(result).toEqual({
				ok: true,
				context: {
					userId: OWNER,
					authUserId: OWNER,
					legacyUserId: null,
					role: "viewer",
					companyId: COMPANY_A,
					readOnly: true,
				},
			});
		});

		it("does not mark session callers read-only", async () => {
			vi.mocked(auth.api.getSession).mockResolvedValue({
				session: { id: "sess-1" },
				user: { id: "auth-user-1", role: "admin", activeCompanyId: COMPANY_A },
			} as never);

			const result = await resolveSessionContext({
				headers: { cookie: "better-auth.session_token=s" },
				allowReadOnlyLedgerKey: true,
				method: "GET",
			});

			expect(result).toMatchObject({ ok: true });
			expect(result.ok && result.context.readOnly).toBeFalsy();
		});
	});
});
