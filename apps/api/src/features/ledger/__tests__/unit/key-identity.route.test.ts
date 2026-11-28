/**
 * `GET /api/ledger/key-identity` — which company a read-only ledger key
 * belongs to.
 *
 * Native clients call it before showing any ledger data and compare the RUC
 * with the workspace they are about to label. Only a key caller gets an
 * answer, always about the key's own company; sessions and header identity
 * get 403 `KEY_IDENTITY_REQUIRES_API_KEY`.
 */
import { Elysia } from "elysia";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("../../../auth/handlers/company-membership", async (importOriginal) => {
	const actual =
		await importOriginal<
			typeof import("../../../auth/handlers/company-membership")
		>();
	return { ...actual, listUserCompanyMemberships: vi.fn() };
});

vi.mock("../../application/queries/get-company-identity.query", () => ({
	getCompanyIdentity: vi.fn(),
}));

const { auth } = await import("../../../auth/auth.config");
const { listUserCompanyMemberships } = await import(
	"../../../auth/handlers/company-membership"
);
const { getCompanyIdentity } = await import(
	"../../application/queries/get-company-identity.query"
);
const { ledgerModule } = await import("../../routes");

const OWNER = "auth-user-owner";
const KEY = "drn_ro_testkeyAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA";
const PATH = "/api/ledger/key-identity";
const IDENTITY_A = {
	companyId: "cmp-a",
	ruc: "20123456789",
	businessName: "Empresa A S.A.C.",
};
const REQUIRES_KEY = {
	success: false,
	error: "This endpoint only answers read-only ledger key callers",
	code: "KEY_IDENTITY_REQUIRES_API_KEY",
};
const TENANT_MISMATCH = {
	success: false,
	error: "API key company does not match the requested company",
	code: "API_KEY_TENANT_MISMATCH",
};

type VerifyApiKey = typeof auth.api.verifyApiKey;

function validKey(companyId = "cmp-a") {
	return {
		valid: true,
		error: null,
		key: {
			id: "key-1",
			referenceId: OWNER,
			permissions: { ledger: ["read"] },
			metadata: { companyId },
		},
	};
}

function keyRequest(
	path: string = PATH,
	extraHeaders: Record<string, string> = {},
): Request {
	return new Request(`http://localhost${path}`, {
		headers: { authorization: `Bearer ${KEY}`, ...extraHeaders },
	});
}

describe("GET /api/ledger/key-identity", () => {
	const app = new Elysia().use(ledgerModule);
	const originalEnv = { ...process.env };
	let verifySpy: ReturnType<typeof vi.fn>;

	beforeEach(() => {
		vi.restoreAllMocks();
		vi.clearAllMocks();
		process.env = { ...originalEnv, NODE_ENV: "test" };
		delete process.env.SECURITY_ENFORCE_TEST_SESSION;
		vi.spyOn(auth.api, "getSession").mockResolvedValue(null as never);
		verifySpy = vi.fn().mockResolvedValue(validKey());
		vi.spyOn(auth.api, "verifyApiKey").mockImplementation(
			verifySpy as unknown as VerifyApiKey,
		);
		vi.mocked(listUserCompanyMemberships).mockResolvedValue([
			{
				companyId: "cmp-a",
				companyName: "A",
				ruc: "20123456789",
				countryCode: "PE",
				membershipRole: "OWNER",
				isDefault: true,
			},
		]);
		vi.mocked(getCompanyIdentity).mockResolvedValue(IDENTITY_A);
	});

	afterEach(() => {
		vi.restoreAllMocks();
		process.env = { ...originalEnv };
	});

	describe("key caller", () => {
		it.each(["test", "production"])(
			"answers the key company's id, RUC and business name only (%s)",
			async (env) => {
				process.env.NODE_ENV = env;
				const res = await app.handle(keyRequest());

				expect(res.status).toBe(200);
				expect(await res.json()).toEqual({ success: true, data: IDENTITY_A });
				expect(getCompanyIdentity).toHaveBeenCalledTimes(1);
				expect(getCompanyIdentity).toHaveBeenCalledWith("cmp-a");
			},
		);

		it("reads the company of the key, not a company named by the client", async () => {
			verifySpy.mockResolvedValue(validKey("cmp-b"));
			vi.mocked(listUserCompanyMemberships).mockResolvedValue([
				{
					companyId: "cmp-b",
					companyName: "B",
					ruc: "20999999999",
					countryCode: "PE",
					membershipRole: "OWNER",
					isDefault: true,
				},
			]);
			vi.mocked(getCompanyIdentity).mockResolvedValue({
				companyId: "cmp-b",
				ruc: "20999999999",
				businessName: "Empresa B",
			});

			const res = await app.handle(keyRequest());

			expect(res.status).toBe(200);
			expect(getCompanyIdentity).toHaveBeenCalledWith("cmp-b");
			expect(await res.json()).toEqual({
				success: true,
				data: {
					companyId: "cmp-b",
					ruc: "20999999999",
					businessName: "Empresa B",
				},
			});
		});

		it("returns no field other than companyId, ruc and businessName", async () => {
			vi.mocked(getCompanyIdentity).mockResolvedValue({
				...IDENTITY_A,
				ownerId: "legacy-owner",
				address: "Av. Siempre Viva 123",
				isActive: true,
			} as never);

			const res = await app.handle(keyRequest());

			expect(res.status).toBe(200);
			const body = (await res.json()) as { data: Record<string, unknown> };
			expect(Object.keys(body)).toEqual(["success", "data"]);
			expect(Object.keys(body.data).sort()).toEqual([
				"businessName",
				"companyId",
				"ruc",
			]);
			expect(body.data).toEqual(IDENTITY_A);
		});

		it("accepts ?companyId equal to the key company", async () => {
			const res = await app.handle(keyRequest(`${PATH}?companyId=cmp-a`));
			expect(res.status).toBe(200);
			expect(getCompanyIdentity).toHaveBeenCalledWith("cmp-a");
		});

		it.each([
			[`${PATH}?companyId=cmp-b`, {}],
			[`${PATH}?companyId=cmp-a&companyId=cmp-b`, {}],
			[PATH, { "x-company-id": "cmp-b" }],
			[PATH, { "x-active-company-id": "cmp-b" }],
		])(
			"refuses a foreign company (%s %j) with 403 API_KEY_TENANT_MISMATCH and reads nothing",
			async (path, headers) => {
				const res = await app.handle(keyRequest(path, headers));
				expect(res.status).toBe(403);
				expect(await res.json()).toEqual(TENANT_MISMATCH);
				expect(getCompanyIdentity).not.toHaveBeenCalled();
			},
		);

		it("answers 404 KEY_COMPANY_NOT_FOUND when the key's company row is missing", async () => {
			vi.mocked(getCompanyIdentity).mockResolvedValue(null);

			const res = await app.handle(keyRequest());

			expect(res.status).toBe(404);
			expect(await res.json()).toEqual({
				success: false,
				error: "The key's company was not found",
				code: "KEY_COMPANY_NOT_FOUND",
			});
			expect(getCompanyIdentity).toHaveBeenCalledWith("cmp-a");
		});

		it("answers 500 INTERNAL_ERROR without the underlying error message", async () => {
			vi.mocked(getCompanyIdentity).mockRejectedValue(
				new Error('relation "companies" does not exist'),
			);

			const res = await app.handle(keyRequest());

			expect(res.status).toBe(500);
			expect(await res.json()).toEqual({
				success: false,
				error: "Could not read the key's company",
				code: "INTERNAL_ERROR",
			});
		});

		it("refuses an invalid key with 401 API_KEY_INVALID and reads nothing", async () => {
			verifySpy.mockResolvedValue({
				valid: false,
				error: { message: "nope", code: "KEY_NOT_FOUND" },
				key: null,
			});

			const res = await app.handle(keyRequest());

			expect(res.status).toBe(401);
			expect(await res.json()).toMatchObject({ code: "API_KEY_INVALID" });
			expect(getCompanyIdentity).not.toHaveBeenCalled();
		});
	});

	describe("non-key callers", () => {
		const session = {
			session: { id: "sess-1" },
			user: { id: "auth-user-1", role: "admin", activeCompanyId: "cmp-a" },
		};

		it.each(["test", "production"])(
			"refuses a session caller with 403 KEY_IDENTITY_REQUIRES_API_KEY (%s)",
			async (env) => {
				process.env.NODE_ENV = env;
				vi.mocked(auth.api.getSession).mockResolvedValue(session as never);

				const res = await app.handle(
					new Request(`http://localhost${PATH}`, {
						headers: { cookie: "better-auth.session_token=s" },
					}),
				);

				expect(res.status).toBe(403);
				expect(await res.json()).toEqual(REQUIRES_KEY);
				expect(getCompanyIdentity).not.toHaveBeenCalled();
			},
		);

		it("refuses a session caller that also sends a key (the session wins)", async () => {
			vi.mocked(auth.api.getSession).mockResolvedValue(session as never);

			const res = await app.handle(
				keyRequest(PATH, { cookie: "better-auth.session_token=s" }),
			);

			expect(res.status).toBe(403);
			expect(await res.json()).toEqual(REQUIRES_KEY);
			expect(verifySpy).not.toHaveBeenCalled();
			expect(getCompanyIdentity).not.toHaveBeenCalled();
		});

		it("refuses a header-identity caller with 403 KEY_IDENTITY_REQUIRES_API_KEY", async () => {
			const res = await app.handle(
				new Request(`http://localhost${PATH}`, {
					headers: {
						"x-user-id": "user-1",
						"x-user-role": "owner",
						"x-company-id": "cmp-a",
					},
				}),
			);

			expect(res.status).toBe(403);
			expect(await res.json()).toEqual(REQUIRES_KEY);
			expect(getCompanyIdentity).not.toHaveBeenCalled();
		});

		it("answers 401 COMPANY_CONTEXT_REQUIRED without any caller, like the other ledger reads", async () => {
			const res = await app.handle(new Request(`http://localhost${PATH}`));

			expect(res.status).toBe(401);
			expect(await res.json()).toEqual({
				success: false,
				error: "Company context is required",
				code: "COMPANY_CONTEXT_REQUIRED",
			});
			expect(getCompanyIdentity).not.toHaveBeenCalled();
		});
	});
});
