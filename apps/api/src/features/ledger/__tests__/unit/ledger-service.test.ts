/**
 * LedgerService Unit Tests
 *
 * @module ledger/__tests__/unit/ledger-service.test
 */

import { db } from "@drenyra/persistence/client";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { LedgerService } from "../../index";

vi.mock("@drenyra/persistence/client", () => ({
	db: {
		select: vi.fn().mockReturnThis(),
		from: vi.fn().mockReturnThis(),
		leftJoin: vi.fn().mockReturnThis(),
		where: vi.fn().mockReturnThis(),
		groupBy: vi.fn().mockReturnThis(),
		orderBy: vi.fn().mockResolvedValue([]),
		execute: vi.fn().mockResolvedValue([{ zone: "America/Lima" }]),
	},
}));

vi.mock("@drenyra/persistence/query", () => ({
	and: vi.fn(),
	eq: vi.fn(),
	asc: vi.fn(),
	desc: vi.fn(),
	gte: vi.fn(),
	lte: vi.fn(),
	sql: vi.fn((strings: TemplateStringsArray) => strings.join("")),
}));

describe("LedgerService", () => {
	beforeEach(() => {
		vi.clearAllMocks();
	});

	describe("getChartOfAccounts", () => {
		it("should return default chart when no categories exist", async () => {
			(db.orderBy as ReturnType<typeof vi.fn>).mockResolvedValue([]);

			const result = await LedgerService.getChartOfAccounts("cmp-1");

			expect(result).toHaveLength(6);
			expect(result[0]).toMatchObject({
				code: "1040",
				name: "Caja y bancos",
				type: "ASSET",
				totalDebit: "0.00",
				totalCredit: "0.00",
				balance: "0.00",
			});
		});

		it("should map categories with transactions to chart accounts", async () => {
			(db.orderBy as ReturnType<typeof vi.fn>).mockResolvedValue([
				{
					categoryId: "cat-1",
					name: "Ventas locales",
					direction: "INCOME",
					activity: 5,
					totalDebit: 0,
					totalCredit: 5000,
				},
			]);

			const result = await LedgerService.getChartOfAccounts("cmp-1");

			expect(result).toHaveLength(1);
			expect(result[0]).toMatchObject({
				name: "Ventas locales",
				type: "REVENUE",
				activity: 5,
			});
		});

		it("should resolve account type based on direction", async () => {
			(db.orderBy as ReturnType<typeof vi.fn>).mockResolvedValue([
				{
					categoryId: "cat-1",
					name: "Compras",
					direction: "EXPENSE",
					activity: 3,
					totalDebit: 2000,
					totalCredit: 0,
				},
				{
					categoryId: "cat-2",
					name: "Cuenta bancaria",
					direction: null,
					activity: 1,
					totalDebit: 3000,
					totalCredit: 1000,
				},
			]);

			const result = await LedgerService.getChartOfAccounts("cmp-1");

			expect(result[0].type).toBe("EXPENSE");
			expect(result[1].type).toBe("ASSET");
		});

		it("should resolve account type as LIABILITY when credits exceed debits", async () => {
			(db.orderBy as ReturnType<typeof vi.fn>).mockResolvedValue([
				{
					categoryId: "cat-1",
					name: "Cuenta por pagar",
					direction: null,
					activity: 2,
					totalDebit: 500,
					totalCredit: 2000,
				},
			]);

			const result = await LedgerService.getChartOfAccounts("cmp-1");

			expect(result[0].type).toBe("LIABILITY");
		});

		it("should filter out rows with null name", async () => {
			(db.orderBy as ReturnType<typeof vi.fn>).mockResolvedValue([
				{
					categoryId: "cat-1",
					name: null,
					direction: "INCOME",
					activity: 0,
					totalDebit: 0,
					totalCredit: 0,
				},
				{
					categoryId: "cat-2",
					name: "Valid Category",
					direction: "EXPENSE",
					activity: 1,
					totalDebit: 100,
					totalCredit: 0,
				},
			]);

			const result = await LedgerService.getChartOfAccounts("cmp-1");

			expect(result).toHaveLength(1);
			expect(result[0].name).toBe("Valid Category");
		});
	});

	describe("getGeneralLedger", () => {
		it("should return formatted general ledger entries", async () => {
			(db.orderBy as ReturnType<typeof vi.fn>).mockResolvedValue([
				{
					id: "tx-1",
					issueDate: new Date("2026-03-01"),
					series: "F001",
					number: "000001",
					notes: "Venta de productos",
					documentType: "FACTURA",
					totalAmount: "1180.00",
					direction: "INCOME",
					categoryName: "Ventas",
				},
			]);

			const result = await LedgerService.getGeneralLedger(
				"cmp-1",
				new Date("2026-03-01"),
				new Date("2026-03-31"),
			);

			expect(result).toHaveLength(1);
			expect(result[0]).toMatchObject({
				id: "tx-1",
				voucher: "F001-000001",
				glosa: "Venta de productos",
				cuenta: "Ventas",
				debe: 0,
				haber: 1180,
				doc: "FACTURA F001 000001",
				bancarizado: false,
			});
		});

		it("should format expense entries with debe instead of haber", async () => {
			(db.orderBy as ReturnType<typeof vi.fn>).mockResolvedValue([
				{
					id: "tx-2",
					issueDate: new Date("2026-03-05"),
					series: "B001",
					number: "000001",
					notes: null,
					documentType: "BOLETA",
					totalAmount: "500.00",
					direction: "EXPENSE",
					categoryName: "Compras",
				},
			]);

			const result = await LedgerService.getGeneralLedger(
				"cmp-1",
				new Date("2026-03-01"),
				new Date("2026-03-31"),
			);

			expect(result[0].debe).toBe(500);
			expect(result[0].haber).toBe(0);
		});

		it("should generate voucher from transaction ID when series/number missing", async () => {
			(db.orderBy as ReturnType<typeof vi.fn>).mockResolvedValue([
				{
					id: "tx-no-voucher",
					issueDate: new Date("2026-03-01"),
					series: null,
					number: null,
					notes: null,
					documentType: "MOVIMIENTO_BANCARIO",
					totalAmount: "100.00",
					direction: "EXPENSE",
					categoryName: null,
				},
			]);

			const result = await LedgerService.getGeneralLedger(
				"cmp-1",
				new Date("2026-03-01"),
				new Date("2026-03-31"),
			);

			expect(result[0].voucher).toMatch(/^TX-/);
			expect(result[0].bancarizado).toBe(true);
			expect(result[0].cuenta).toBe("SIN CATEGORIA");
		});

		it("should return empty array when no transactions exist", async () => {
			(db.orderBy as ReturnType<typeof vi.fn>).mockResolvedValue([]);

			const result = await LedgerService.getGeneralLedger(
				"cmp-1",
				new Date("2026-03-01"),
				new Date("2026-03-31"),
			);

			expect(result).toEqual([]);
		});

		it("should handle invalid amount strings gracefully", async () => {
			(db.orderBy as ReturnType<typeof vi.fn>).mockResolvedValue([
				{
					id: "tx-bad-amount",
					issueDate: new Date("2026-03-01"),
					series: "F001",
					number: "000001",
					notes: null,
					documentType: "FACTURA",
					totalAmount: "not-a-number",
					direction: "INCOME",
					categoryName: "Ventas",
				},
			]);

			const result = await LedgerService.getGeneralLedger(
				"cmp-1",
				new Date("2026-03-01"),
				new Date("2026-03-31"),
			);

			expect(result[0].haber).toBe(0);
		});
	});

	describe("getTrialBalance", () => {
		/** Row shape produced by the trial balance SQL (all money as decimal text). */
		function trialRow(overrides: Record<string, unknown>) {
			return {
				categoryId: "cat",
				name: "Cuenta",
				direction: null,
				activity: 0,
				debits: "0.00",
				credits: "0.00",
				balance: "0.00",
				totalDebits: "0.00",
				totalCredits: "0.00",
				totalBalance: "0.00",
				...overrides,
			};
		}

		it("should return trial balance with accounts and totals", async () => {
			const totals = {
				totalDebits: "5000.00",
				totalCredits: "5000.00",
				totalBalance: "0.00",
			};
			(db.orderBy as ReturnType<typeof vi.fn>).mockResolvedValue([
				trialRow({
					categoryId: "cat-1",
					name: "Caja",
					activity: 10,
					debits: "5000.00",
					credits: "2000.00",
					balance: "3000.00",
					...totals,
				}),
				trialRow({
					categoryId: "cat-2",
					name: "Ventas",
					direction: "INCOME",
					activity: 8,
					debits: "0.00",
					credits: "3000.00",
					balance: "-3000.00",
					...totals,
				}),
			]);

			const result = await LedgerService.getTrialBalance("cmp-1", {
				kind: "calendar-day",
				day: "2026-03-31",
			});

			expect(result.asOfDate).toBe("2026-03-31T00:00:00.000Z");
			expect(result.accounts).toHaveLength(2);
			expect(result.accounts[0]).toMatchObject({
				name: "Caja",
				debits: "5000.00",
				credits: "2000.00",
				balance: "3000.00",
				activity: 10,
			});
			expect(result.debits).toBe("5000.00");
			expect(result.credits).toBe("5000.00");
			expect(result.balance).toBe("0.00");
		});

		it("should keep 0.10 + 0.20 exactly 0.30 (SQL decimal text, no JS floats)", async () => {
			const totals = {
				totalDebits: "0.30",
				totalCredits: "0.00",
				totalBalance: "0.30",
			};
			(db.orderBy as ReturnType<typeof vi.fn>).mockResolvedValue([
				trialRow({ name: "A", debits: "0.10", balance: "0.10", ...totals }),
				trialRow({ name: "B", debits: "0.20", balance: "0.20", ...totals }),
			]);

			const result = await LedgerService.getTrialBalance("cmp-1", {
				kind: "calendar-day",
				day: "2026-03-31",
			});

			expect(result.debits).toBe("0.30");
			expect(result.balance).toBe("0.30");
		});

		it("should return large amounts exactly and without exponent notation", async () => {
			(db.orderBy as ReturnType<typeof vi.fn>).mockResolvedValue([
				trialRow({
					name: "Masivo",
					credits: "99999999999999999999999.99",
					balance: "-99999999999999999999999.99",
					totalDebits: "0.00",
					totalCredits: "99999999999999999999999.99",
					totalBalance: "-99999999999999999999999.99",
				}),
			]);

			const result = await LedgerService.getTrialBalance("cmp-1", {
				kind: "calendar-day",
				day: "2026-03-31",
			});

			expect(result.accounts[0]?.credits).toBe("99999999999999999999999.99");
			expect(result.credits).toBe("99999999999999999999999.99");
			expect(result.balance).toBe("-99999999999999999999999.99");
			expect(JSON.stringify(result)).not.toMatch(/e\+/i);
		});

		it("should return negative balances as signed decimal text", async () => {
			(db.orderBy as ReturnType<typeof vi.fn>).mockResolvedValue([
				trialRow({
					name: "Ventas",
					direction: "INCOME",
					credits: "150.25",
					balance: "-150.25",
					totalCredits: "150.25",
					totalBalance: "-150.25",
				}),
			]);

			const result = await LedgerService.getTrialBalance("cmp-1", {
				kind: "calendar-day",
				day: "2026-03-31",
			});

			expect(result.accounts[0]?.balance).toBe("-150.25");
			expect(result.balance).toBe("-150.25");
		});

		it("should refuse amounts that are not exact 2-decimal text", async () => {
			(db.orderBy as ReturnType<typeof vi.fn>).mockResolvedValue([
				trialRow({ name: "Caja", debits: 5000, totalDebits: "5000.00" }),
			]);

			await expect(
				LedgerService.getTrialBalance("cmp-1", {
					kind: "calendar-day",
					day: "2026-03-31",
				}),
			).rejects.toThrow(/exact 2-decimal/);

			(db.orderBy as ReturnType<typeof vi.fn>).mockResolvedValue([
				trialRow({ name: "Caja", totalCredits: "1e+21" }),
			]);

			await expect(
				LedgerService.getTrialBalance("cmp-1", {
					kind: "calendar-day",
					day: "2026-03-31",
				}),
			).rejects.toThrow(/exact 2-decimal/);
		});

		it("should return empty trial balance when no categories exist", async () => {
			(db.orderBy as ReturnType<typeof vi.fn>).mockResolvedValue([]);

			const result = await LedgerService.getTrialBalance(
				"cmp-1",
				new Date("2026-03-31"),
			);

			expect(result.accounts).toHaveLength(0);
			expect(result.debits).toBe("0.00");
			expect(result.credits).toBe("0.00");
			expect(result.balance).toBe("0.00");
		});

		it("should filter out accounts with null names", async () => {
			const totals = {
				totalDebits: "1000.00",
				totalCredits: "1000.00",
				totalBalance: "0.00",
			};
			(db.orderBy as ReturnType<typeof vi.fn>).mockResolvedValue([
				trialRow({ categoryId: "cat-null", name: null, ...totals }),
				trialRow({
					categoryId: "cat-valid",
					name: "IGV",
					activity: 5,
					debits: "1000.00",
					credits: "1000.00",
					...totals,
				}),
			]);

			const result = await LedgerService.getTrialBalance(
				"cmp-1",
				new Date("2026-03-31"),
			);

			expect(result.accounts).toHaveLength(1);
			expect(result.accounts[0]?.name).toBe("IGV");
		});
	});

	describe("fiscal zone lookup", () => {
		// pg_timezone_names is a full function scan (~13-50 ms); it must run
		// once per report, not once per use of the zone in the SQL.
		it("resolves the company zone with a single query per trial balance", async () => {
			(db.orderBy as ReturnType<typeof vi.fn>).mockResolvedValue([]);

			await LedgerService.getTrialBalance("cmp-1", {
				kind: "instant",
				at: new Date("2026-04-01T04:59:59.999Z"),
			});

			expect(db.execute).toHaveBeenCalledOnce();
		});

		it("resolves the company zone with a single query per general ledger", async () => {
			(db.orderBy as ReturnType<typeof vi.fn>).mockResolvedValue([]);

			await LedgerService.getGeneralLedger(
				"cmp-1",
				{ kind: "instant", at: new Date("2026-03-01T10:00:00.000Z") },
				{ kind: "instant", at: new Date("2026-04-01T04:59:59.999Z") },
			);

			expect(db.execute).toHaveBeenCalledOnce();
		});
	});
});
