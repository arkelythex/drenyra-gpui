/**
 * Read-only ledger keys through the real company scope guard and routes.
 *
 * Opted-in surface: `ledgerModule` GET routes and journal-entries GET `/`
 * and `/:id`. Every write, `POST /api/ledger/export/*`, and any route that
 * does not opt in must refuse a key with no data returned and no write.
 */
import { Elysia } from "elysia";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("../../auth/handlers/company-membership", async (importOriginal) => {
	const actual =
		await importOriginal<
			typeof import("../../auth/handlers/company-membership")
		>();
	return { ...actual, listUserCompanyMemberships: vi.fn() };
});

vi.mock("../../ledger/application/queries/get-chart-of-accounts.query", () => ({
	getChartOfAccounts: vi.fn(),
}));
vi.mock("../../ledger/application/queries/get-general-ledger.query", () => ({
	getGeneralLedger: vi.fn(),
}));
vi.mock("../../ledger/application/queries/get-trial-balance.query", () => ({
	getTrialBalance: vi.fn(),
}));
vi.mock(
	"../../ledger-mvp/application/queries/export-general-ledger-pdf.query",
	() => ({ exportGeneralLedgerPdf: vi.fn() }),
);
vi.mock(
	"../../ledger-mvp/application/queries/export-general-ledger-xlsx.query",
	() => ({ exportGeneralLedgerXlsx: vi.fn() }),
);

const ORGANIZATION_BY_COMPANY: Record<string, number> = {
	"cmp-a": 1,
	"cmp-b": 2,
};
const executeSpy = vi.fn();
const repo = { findById: vi.fn(), delete: vi.fn() };

vi.mock("../../journal-entries/application/_helpers", () => ({
	journalRepository: repo,
	periodRepository: {},
	accountService: {},
	resolveOrganizationId: async (companyId: string | undefined) => {
		if (!companyId) throw new Error("Contexto de empresa requerido");
		return ORGANIZATION_BY_COMPANY[companyId] ?? 99;
	},
	resolveTenantScope: async (companyId: string | undefined) => {
		if (!companyId) throw new Error("Contexto de empresa requerido");
		return {
			organizationId: String(ORGANIZATION_BY_COMPANY[companyId] ?? 99),
			companyId,
		};
	},
}));

vi.mock("@drenyra/application/use-cases/journal", () => {
	class UseCase {
		execute = executeSpy;
	}
	return {
		GetJournalEntriesUseCase: UseCase,
		CreateJournalEntryUseCase: UseCase,
		UpdateJournalEntryUseCase: UseCase,
		UpdateJournalEntryStatusUseCase: UseCase,
		DeleteJournalEntryUseCase: UseCase,
	};
});

const { auth } = await import("../../auth/auth.config");
const { listUserCompanyMemberships } = await import(
	"../../auth/handlers/company-membership"
);
const { getChartOfAccounts } = await import(
	"../../ledger/application/queries/get-chart-of-accounts.query"
);
const { getGeneralLedger } = await import(
	"../../ledger/application/queries/get-general-ledger.query"
);
const { getTrialBalance } = await import(
	"../../ledger/application/queries/get-trial-balance.query"
);
const { exportGeneralLedgerPdf } = await import(
	"../../ledger-mvp/application/queries/export-general-ledger-pdf.query"
);
const { exportGeneralLedgerXlsx } = await import(
	"../../ledger-mvp/application/queries/export-general-ledger-xlsx.query"
);
const { ledgerModule } = await import("../../ledger/routes");
const { journalEntryRoutes } = await import("../../journal-entries/api/routes");
const { ledgerExportRoutes } = await import("../../ledger-mvp/export.routes");

const OWNER = "auth-user-owner";
const KEY = "drn_ro_testkeyAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA";
const ENTRY_ID = "550e8400-e29b-41d4-a716-446655440000";

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
	path: string,
	method = "GET",
	extraHeaders: Record<string, string> = {},
	body?: unknown,
): Request {
	return new Request(`http://localhost${path}`, {
		method,
		headers: {
			authorization: `Bearer ${KEY}`,
			...(body ? { "content-type": "application/json" } : {}),
			...extraHeaders,
		},
		body: body ? JSON.stringify(body) : undefined,
	});
}

const LEDGER_GETS = [
	"/api/ledger/accounts",
	"/api/ledger/general?startDate=2026-03-01&endDate=2026-03-31",
	"/api/ledger/trial-balance?asOfDate=2026-03-31",
] as const;

const JOURNAL_WRITES: ReadonlyArray<[string, string, unknown]> = [
	[
		"POST",
		"/api/journal-entries",
		{
			date: "2026-03-01",
			gloss: "x",
			lines: [{ accountId: "a", debit: "1.00", credit: "0.00" }],
		},
	],
	["PATCH", `/api/journal-entries/${ENTRY_ID}`, { gloss: "x" }],
	["DELETE", `/api/journal-entries/${ENTRY_ID}`, undefined],
	["POST", `/api/journal-entries/${ENTRY_ID}/mayorizar`, undefined],
	["POST", `/api/journal-entries/${ENTRY_ID}/declarar`, undefined],
	["POST", `/api/journal-entries/${ENTRY_ID}/approve`, undefined],
	["POST", `/api/journal-entries/${ENTRY_ID}/reject`, undefined],
];

describe("read-only ledger key routes", () => {
	const app = new Elysia()
		.use(ledgerModule)
		.use(journalEntryRoutes)
		.use(ledgerExportRoutes);
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
		vi.mocked(getChartOfAccounts).mockResolvedValue([]);
		vi.mocked(getGeneralLedger).mockResolvedValue([] as never);
		vi.mocked(getTrialBalance).mockResolvedValue({} as never);
		executeSpy.mockResolvedValue([]);
		repo.findById.mockResolvedValue({ toJSON: () => ({ id: ENTRY_ID }) });
	});

	afterEach(() => {
		vi.restoreAllMocks();
		process.env = { ...originalEnv };
	});

	describe("opted-in surface", () => {
		it("ledgerModule only exposes GET routes", () => {
			expect(
				ledgerModule.routes.map((route) => `${route.method} ${route.path}`),
			).toEqual([
				"GET /api/ledger/accounts",
				"GET /api/ledger/general",
				"GET /api/ledger/trial-balance",
			]);
		});

		it("journal-entries exposes exactly GET / and GET /:id as reads", () => {
			expect(
				journalEntryRoutes.routes
					.filter((route) => route.method === "GET")
					.map((route) => route.path),
			).toEqual(["/api/journal-entries/", "/api/journal-entries/:id"]);
		});
	});

	describe("happy path", () => {
		it("serves every ledger GET for the key company only", async () => {
			for (const path of LEDGER_GETS) {
				const res = await app.handle(keyRequest(path));
				expect(res.status, path).toBe(200);
			}
			expect(getChartOfAccounts).toHaveBeenCalledWith("cmp-a");
			expect(vi.mocked(getGeneralLedger).mock.calls[0]?.[0]).toBe("cmp-a");
			expect(vi.mocked(getTrialBalance).mock.calls[0]?.[0]).toBe("cmp-a");
		});

		it("lists journal entries for the key company's organization", async () => {
			const res = await app.handle(keyRequest("/api/journal-entries"));
			expect(res.status).toBe(200);
			expect(executeSpy).toHaveBeenCalledTimes(1);
			expect(executeSpy.mock.calls[0]?.[0]).toMatchObject({
				organizationId: 1,
			});
		});

		it("reads one journal entry inside the key company's scope", async () => {
			const res = await app.handle(
				keyRequest(`/api/journal-entries/${ENTRY_ID}`),
			);
			expect(res.status).toBe(200);
			expect(repo.findById).toHaveBeenCalledWith(
				{ organizationId: "1", companyId: "cmp-a" },
				ENTRY_ID,
			);
		});
	});

	describe("tenant isolation", () => {
		it("a key for company A never reads company B, even with x-company-id: B", async () => {
			const requests = [
				...LEDGER_GETS.map((path) =>
					keyRequest(path, "GET", { "x-company-id": "cmp-b" }),
				),
				keyRequest("/api/journal-entries", "GET", { "x-company-id": "cmp-b" }),
				keyRequest(`/api/journal-entries/${ENTRY_ID}`, "GET", {
					"x-company-id": "cmp-b",
				}),
			];

			for (const request of requests) {
				const res = await app.handle(request);
				expect(res.status, request.url).toBe(403);
				expect(await res.json()).toMatchObject({
					success: false,
					code: "API_KEY_TENANT_MISMATCH",
				});
			}
			expect(getChartOfAccounts).not.toHaveBeenCalled();
			expect(getGeneralLedger).not.toHaveBeenCalled();
			expect(getTrialBalance).not.toHaveBeenCalled();
			expect(executeSpy).not.toHaveBeenCalled();
			expect(repo.findById).not.toHaveBeenCalled();
		});

		it.each([
			["test", "cmp-a", "cmp-b"],
			["test", "cmp-b", "cmp-a"],
			["test", "cmp-b", "cmp-c"],
			["production", "cmp-a", "cmp-b"],
			["production", "cmp-b", "cmp-a"],
		])(
			"refuses conflicting x-company-id / x-active-company-id with a key (%s, %s vs %s) with 403 API_KEY_TENANT_MISMATCH",
			async (env, companyHeader, activeCompanyHeader) => {
				process.env.NODE_ENV = env;
				for (const path of [
					...LEDGER_GETS,
					"/api/journal-entries",
					`/api/journal-entries/${ENTRY_ID}`,
				]) {
					const res = await app.handle(
						keyRequest(path, "GET", {
							"x-company-id": companyHeader,
							"x-active-company-id": activeCompanyHeader,
						}),
					);
					expect(res.status, path).toBe(403);
					expect(await res.json()).toEqual({
						success: false,
						error: "API key company does not match the requested company",
						code: "API_KEY_TENANT_MISMATCH",
					});
				}
				expect(getChartOfAccounts).not.toHaveBeenCalled();
				expect(getGeneralLedger).not.toHaveBeenCalled();
				expect(getTrialBalance).not.toHaveBeenCalled();
				expect(executeSpy).not.toHaveBeenCalled();
				expect(repo.findById).not.toHaveBeenCalled();
			},
		);

		it("keeps matching x-company-id and x-active-company-id working with a key", async () => {
			const res = await app.handle(
				keyRequest("/api/ledger/accounts", "GET", {
					"x-company-id": "cmp-a",
					"x-active-company-id": "cmp-a",
				}),
			);
			expect(res.status).toBe(200);
			expect(getChartOfAccounts).toHaveBeenCalledWith("cmp-a");
		});

		it("ignores ?companyId for another company and keeps the key company", async () => {
			const res = await app.handle(
				keyRequest("/api/ledger/accounts?companyId=cmp-b"),
			);
			expect(res.status).toBe(200);
			expect(getChartOfAccounts).toHaveBeenCalledWith("cmp-a");
			expect(getChartOfAccounts).not.toHaveBeenCalledWith("cmp-b");
		});
	});

	describe("refusals", () => {
		it.each(JOURNAL_WRITES)(
			"%s %s refuses a key with 403 and writes nothing",
			async (method, path, body) => {
				const res = await app.handle(keyRequest(path, method, {}, body));
				expect(res.status).toBe(403);
				expect(await res.json()).toMatchObject({
					success: false,
					code: "API_KEY_READ_ONLY",
				});
				expect(executeSpy).not.toHaveBeenCalled();
				expect(repo.findById).not.toHaveBeenCalled();
				expect(repo.delete).not.toHaveBeenCalled();
				expect(verifySpy).not.toHaveBeenCalled();
			},
		);

		it.each(["/api/ledger/export/pdf", "/api/ledger/export/xlsx"])(
			"POST %s ignores the key and returns no data",
			async (path) => {
				const res = await app.handle(
					keyRequest(path, "POST", {}, { startDate: "2026-03-01" }),
				);
				expect(res.status).toBe(401);
				expect(exportGeneralLedgerPdf).not.toHaveBeenCalled();
				expect(exportGeneralLedgerXlsx).not.toHaveBeenCalled();
				expect(verifySpy).not.toHaveBeenCalled();
			},
		);

		it("refuses an invalid key on every opted-in GET with 401 and no data", async () => {
			verifySpy.mockResolvedValue({
				valid: false,
				error: { message: "nope", code: "KEY_NOT_FOUND" },
				key: null,
			});
			for (const path of [
				...LEDGER_GETS,
				"/api/journal-entries",
				`/api/journal-entries/${ENTRY_ID}`,
			]) {
				const res = await app.handle(keyRequest(path));
				expect(res.status, path).toBe(401);
				expect(await res.json()).toMatchObject({ code: "API_KEY_INVALID" });
			}
			expect(getChartOfAccounts).not.toHaveBeenCalled();
			expect(executeSpy).not.toHaveBeenCalled();
			expect(repo.findById).not.toHaveBeenCalled();
		});

		it("answers a rate-limited key with 429, Retry-After and no data", async () => {
			verifySpy.mockResolvedValue({
				valid: false,
				error: {
					message: "Rate limit exceeded.",
					code: "RATE_LIMITED",
					details: { tryAgainIn: 12_345 },
				},
				key: null,
			});
			const res = await app.handle(keyRequest("/api/ledger/accounts"));
			expect(res.status).toBe(429);
			expect(res.headers.get("retry-after")).toBe("13");
			expect(await res.json()).toEqual({
				success: false,
				error: "API key rate limit exceeded; retry later",
				code: "API_KEY_RATE_LIMITED",
			});
			expect(getChartOfAccounts).not.toHaveBeenCalled();
		});

		it("falls back to the key's rate-limit window for Retry-After", async () => {
			for (const error of [
				{ message: "Rate limit exceeded.", code: "RATE_LIMITED" },
				{ message: "Usage exceeded.", code: "USAGE_EXCEEDED" },
			]) {
				verifySpy.mockResolvedValueOnce({ valid: false, error, key: null });
				const res = await app.handle(keyRequest("/api/ledger/accounts"));
				expect(res.status, error.code).toBe(429);
				expect(res.headers.get("retry-after"), error.code).toBe("60");
			}
		});

		it("sends no Retry-After on other key refusals", async () => {
			verifySpy.mockResolvedValue({
				valid: false,
				error: { message: "nope", code: "KEY_NOT_FOUND" },
				key: null,
			});
			const res = await app.handle(keyRequest("/api/ledger/accounts"));
			expect(res.status).toBe(401);
			expect(res.headers.get("retry-after")).toBeNull();
		});

		it("refuses a key whose owner lost membership with 403 and no data", async () => {
			vi.mocked(listUserCompanyMemberships).mockResolvedValue([]);
			const res = await app.handle(keyRequest("/api/ledger/accounts"));
			expect(res.status).toBe(403);
			expect(await res.json()).toMatchObject({
				code: "API_KEY_MEMBERSHIP_INACTIVE",
			});
			expect(getChartOfAccounts).not.toHaveBeenCalled();
		});

		it("does not fall back to header identity when a key is refused", async () => {
			verifySpy.mockResolvedValue({
				valid: false,
				error: { message: "nope", code: "KEY_EXPIRED" },
				key: null,
			});
			const res = await app.handle(
				keyRequest("/api/ledger/accounts", "GET", {
					"x-user-id": "user-1",
					"x-user-role": "owner",
					"x-company-id": "cmp-a",
				}),
			);
			expect(res.status).toBe(401);
			expect(getChartOfAccounts).not.toHaveBeenCalled();
		});
	});

	describe("production NODE_ENV", () => {
		beforeEach(() => {
			process.env.NODE_ENV = "production";
		});

		it("serves an opted-in GET for the key company", async () => {
			const res = await app.handle(keyRequest("/api/ledger/accounts"));
			expect(res.status).toBe(200);
			expect(getChartOfAccounts).toHaveBeenCalledWith("cmp-a");
		});

		it("refuses writes, foreign companies and export the same way", async () => {
			const write = await app.handle(
				keyRequest(`/api/journal-entries/${ENTRY_ID}`, "DELETE"),
			);
			const foreign = await app.handle(
				keyRequest("/api/ledger/accounts", "GET", { "x-company-id": "cmp-b" }),
			);
			const exported = await app.handle(
				keyRequest("/api/ledger/export/pdf", "POST", {}, {}),
			);

			expect(write.status).toBe(403);
			expect(foreign.status).toBe(403);
			expect(exported.status).toBe(401);
			expect(executeSpy).not.toHaveBeenCalled();
			expect(getChartOfAccounts).not.toHaveBeenCalled();
			expect(exportGeneralLedgerPdf).not.toHaveBeenCalled();
		});
	});
});
