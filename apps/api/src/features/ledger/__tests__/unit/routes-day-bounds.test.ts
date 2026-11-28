/**
 * Ledger routes pass date query values to the queries as fiscal-day bounds:
 * `YYYY-MM-DD` is a whole calendar day, an ISO date-time is an instant.
 */
import { Elysia } from "elysia";
import { beforeEach, describe, expect, it, vi } from "vitest";

const mocks = vi.hoisted(() => ({
	getTrialBalance: vi.fn(),
	getGeneralLedger: vi.fn(),
	getChartOfAccounts: vi.fn(),
	logError: vi.fn(),
}));

vi.mock("../../../../lib/logger", () => ({
	createLogger: () => ({ error: mocks.logError }),
}));

vi.mock("../../../../shared/plugins", () => ({
	companyScopeGuard: () => (app: Elysia) =>
		app.derive(() => ({
			companyContext: {
				userId: "user-1",
				authUserId: "user-1",
				legacyUserId: "user-1",
				role: "OWNER",
				companyId: "cmp-1",
			},
		})),
}));

vi.mock("../../application/queries/get-trial-balance.query", () => ({
	getTrialBalance: mocks.getTrialBalance,
}));

vi.mock("../../application/queries/get-general-ledger.query", () => ({
	getGeneralLedger: mocks.getGeneralLedger,
}));

vi.mock("../../application/queries/get-chart-of-accounts.query", () => ({
	getChartOfAccounts: mocks.getChartOfAccounts,
}));

const { ledgerModule } = await import("../../routes");
const { LedgerDateRangeError } = await import("../../_internal/fiscal-day");

describe("ledger routes — fiscal-day bounds", () => {
	const app = new Elysia().use(ledgerModule);

	beforeEach(() => {
		mocks.getTrialBalance.mockReset().mockResolvedValue({
			asOfDate: "2026-03-31T00:00:00.000Z",
			debits: "0.00",
			credits: "0.00",
			balance: "0.00",
			accounts: [],
		});
		mocks.getGeneralLedger.mockReset().mockResolvedValue([]);
		mocks.getChartOfAccounts.mockReset().mockResolvedValue([]);
		mocks.logError.mockReset();
	});

	it("passes asOfDate=YYYY-MM-DD as a whole calendar day", async () => {
		const response = await app.handle(
			new Request(
				"http://localhost/api/ledger/trial-balance?asOfDate=2026-03-31",
			),
		);

		expect(response.status).toBe(200);
		expect(mocks.getTrialBalance).toHaveBeenCalledExactlyOnceWith("cmp-1", {
			kind: "calendar-day",
			day: "2026-03-31",
		});
	});

	it("passes an ISO date-time asOfDate as an instant", async () => {
		const response = await app.handle(
			new Request(
				"http://localhost/api/ledger/trial-balance?asOfDate=2026-04-01T04:59:59.999Z",
			),
		);

		expect(response.status).toBe(200);
		expect(mocks.getTrialBalance).toHaveBeenCalledExactlyOnceWith("cmp-1", {
			kind: "instant",
			at: new Date("2026-04-01T04:59:59.999Z"),
		});
	});

	it("rejects a calendar day that does not exist with 400 INVALID_DATE", async () => {
		const response = await app.handle(
			new Request(
				"http://localhost/api/ledger/trial-balance?asOfDate=2026-02-30",
			),
		);

		expect(response.status).toBe(400);
		expect(await response.json()).toMatchObject({
			success: false,
			code: "INVALID_DATE",
		});
		expect(mocks.getTrialBalance).not.toHaveBeenCalled();
	});

	it("passes general ledger startDate/endDate as calendar days", async () => {
		const response = await app.handle(
			new Request(
				"http://localhost/api/ledger/general?startDate=2026-03-01&endDate=2026-03-31",
			),
		);

		expect(response.status).toBe(200);
		expect(mocks.getGeneralLedger).toHaveBeenCalledExactlyOnceWith(
			"cmp-1",
			{ kind: "calendar-day", day: "2026-03-01" },
			{ kind: "calendar-day", day: "2026-03-31" },
		);
	});

	it("keeps rejecting startDate after endDate with 400 INVALID_DATE_RANGE", async () => {
		const response = await app.handle(
			new Request(
				"http://localhost/api/ledger/general?startDate=2026-03-31&endDate=2026-03-01",
			),
		);

		expect(response.status).toBe(400);
		expect(await response.json()).toMatchObject({
			success: false,
			code: "INVALID_DATE_RANGE",
		});
		expect(mocks.getGeneralLedger).not.toHaveBeenCalled();
	});

	it("accepts a one-day range (startDate equal to endDate)", async () => {
		const response = await app.handle(
			new Request(
				"http://localhost/api/ledger/general?startDate=2026-03-31&endDate=2026-03-31",
			),
		);

		expect(response.status).toBe(200);
		expect(mocks.getGeneralLedger).toHaveBeenCalledOnce();
	});

	// R3-001 — the order of mixed bounds is decided on fiscal days in the
	// company zone (by the query), not on representative instants.
	it("accepts an instant late on 31 Mar in Lima (after 00:00Z) up to day 31 Mar", async () => {
		const response = await app.handle(
			new Request(
				"http://localhost/api/ledger/general?startDate=2026-04-01T03:00:00.000Z&endDate=2026-03-31",
			),
		);

		expect(response.status).toBe(200);
		expect(mocks.getGeneralLedger).toHaveBeenCalledExactlyOnceWith(
			"cmp-1",
			{ kind: "instant", at: new Date("2026-04-01T03:00:00.000Z") },
			{ kind: "calendar-day", day: "2026-03-31" },
		);
	});

	it("answers 400 INVALID_DATE_RANGE when the fiscal range is inverted (day 31 Mar to 03:00Z 31 Mar)", async () => {
		mocks.getGeneralLedger.mockRejectedValue(
			new LedgerDateRangeError("2026-03-31", "2026-03-30"),
		);

		const response = await app.handle(
			new Request(
				"http://localhost/api/ledger/general?startDate=2026-03-31&endDate=2026-03-31T03:00:00.000Z",
			),
		);

		expect(response.status).toBe(400);
		expect(await response.json()).toEqual({
			success: false,
			error: "startDate must be less than or equal to endDate.",
			code: "INVALID_DATE_RANGE",
		});
		expect(mocks.getGeneralLedger).toHaveBeenCalledExactlyOnceWith(
			"cmp-1",
			{ kind: "calendar-day", day: "2026-03-31" },
			{ kind: "instant", at: new Date("2026-03-31T03:00:00.000Z") },
		);
		expect(mocks.logError).not.toHaveBeenCalled();
	});

	// R4-002 — the log carries the driver code/severity and a sanitized message.
	it("logs the driver error code, severity, name and a message without SQL or parameters", async () => {
		const driverError = Object.assign(
			new Error('invalid input syntax for type uuid: "secret-param"'),
			{
				name: "PostgresError",
				code: "22P02",
				severity: "ERROR",
				query: 'SELECT "id" FROM "transactions" WHERE "company_id" = $1',
				parameters: ["secret-param"],
			},
		);
		const wrapped = Object.assign(
			new Error(
				'Failed query: SELECT "id" FROM "transactions" WHERE "company_id" = $1\nparams: secret-param',
			),
			{ cause: driverError },
		);
		mocks.getTrialBalance.mockRejectedValue(wrapped);

		const response = await app.handle(
			new Request(
				"http://localhost/api/ledger/trial-balance?asOfDate=2026-03-31",
			),
		);

		expect(response.status).toBe(500);
		expect(mocks.logError).toHaveBeenCalledOnce();
		const [fields, message] = mocks.logError.mock.calls[0] ?? [];
		expect(message).toBe("Ledger query failed");
		expect(fields).toEqual({
			route: "/api/ledger/trial-balance",
			errorName: "PostgresError",
			errorCode: "22P02",
			errorSeverity: "ERROR",
			errorMessage: 'invalid input syntax for type uuid: "?"',
		});
		const logged = JSON.stringify(mocks.logError.mock.calls);
		expect(logged).not.toContain("SELECT");
		expect(logged).not.toContain("secret-param");
		expect(logged).not.toContain("transactions");
	});

	it("logs only the error name and a redacted message for a wrapped query error without a driver cause", async () => {
		mocks.getTrialBalance.mockRejectedValue(
			new Error('Failed query: SELECT 1 FROM "companies"\nparams: 20123456789'),
		);

		await app.handle(
			new Request(
				"http://localhost/api/ledger/trial-balance?asOfDate=2026-03-31",
			),
		);

		const [fields] = mocks.logError.mock.calls[0] ?? [];
		expect(fields).toEqual({
			route: "/api/ledger/trial-balance",
			errorName: "Error",
			errorMessage: "Failed query (SQL text redacted)",
		});
		const logged = JSON.stringify(mocks.logError.mock.calls);
		expect(logged).not.toContain("SELECT");
		expect(logged).not.toContain("20123456789");
	});

	it("reads asOfDate at exactly 00:00:00.000Z (the echoed value) as that calendar day", async () => {
		const response = await app.handle(
			new Request(
				"http://localhost/api/ledger/trial-balance?asOfDate=2026-03-31T00:00:00.000Z",
			),
		);

		expect(response.status).toBe(200);
		expect(mocks.getTrialBalance).toHaveBeenCalledExactlyOnceWith("cmp-1", {
			kind: "calendar-day",
			day: "2026-03-31",
		});
	});

	it.each([
		["/api/ledger/trial-balance?asOfDate=2026-03-31", "getTrialBalance"],
		[
			"/api/ledger/general?startDate=2026-03-01&endDate=2026-03-31",
			"getGeneralLedger",
		],
		["/api/ledger/accounts", "getChartOfAccounts"],
	] as const)(
		"%s answers 500 with a fixed message, never the error text",
		async (path, query) => {
			const leak =
				'invalid input syntax: SELECT "settings_timezone" FROM "companies"';
			const failure = new Error(leak);
			failure.name = "PostgresError";
			mocks[query].mockRejectedValue(failure);

			const response = await app.handle(new Request(`http://localhost${path}`));
			const body = await response.text();

			expect(response.status).toBe(500);
			expect(JSON.parse(body)).toEqual({
				success: false,
				error: "Ledger query failed",
				code: "INTERNAL_ERROR",
			});
			expect(body).not.toContain("SELECT");
			expect(body).not.toContain("settings_timezone");
			expect(mocks.logError).toHaveBeenCalledOnce();
			const [fields] = mocks.logError.mock.calls[0] ?? [];
			expect(fields).toMatchObject({ errorName: "PostgresError" });
			expect(JSON.stringify(mocks.logError.mock.calls)).not.toContain("SELECT");
		},
	);
});
