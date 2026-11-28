/**
 * Trial balance and general ledger against a real Postgres:
 * - F1: a day is the whole calendar day in the company's fiscal time zone.
 * - F2: sums and balances are exact decimal text (no JS floats).
 *
 * Run with RUN_DB_TESTS=1 and DATABASE_URL pointing at a disposable database
 * that has the `transactions`, `categories`, `companies` and `users` tables.
 *
 * `transactions.issue_date` is `timestamp without time zone`; Drizzle writes a
 * JS Date as its UTC wall clock. Writers store either a calendar-day label
 * (`new Date("YYYY-MM-DD")` → UTC midnight, e.g. UBL `cbc:IssueDate` in the
 * inbox) or an instant (`new Date()`). Both are inserted here through Drizzle.
 *
 * @module ledger/__tests__/integration/ledger-fiscal-day.integration.test
 */

import { randomUUID } from "node:crypto";
import { db } from "@drenyra/persistence/client";
import { inArray, sql } from "@drenyra/persistence/query";
import {
	categories,
	companies,
	transactions,
	users,
} from "@drenyra/persistence/schema";
import { afterAll, beforeAll, describe, expect, it } from "vitest";
import { parseLedgerDayBound } from "../../_internal/fiscal-day";
import { getGeneralLedger, getTrialBalance } from "../../queries";

const describeDb = process.env.RUN_DB_TESTS === "1" ? describe : describe.skip;

function day(value: string) {
	const bound = parseLedgerDayBound(value);
	if (!bound) throw new Error(`invalid test day ${value}`);
	return bound;
}

describeDb("ledger fiscal day and exact money (integration)", () => {
	const ownerId = randomUUID();
	const limaCompanyId = randomUUID();
	const utcCompanyId = randomUUID();
	const moneyCompanyId = randomUUID();
	const otherCompanyId = randomUUID();
	const companyIds = [
		limaCompanyId,
		utcCompanyId,
		moneyCompanyId,
		otherCompanyId,
	];

	const limaSalesId = randomUUID();
	const utcSalesId = randomUUID();
	const centsId = randomUUID();
	const purchasesId = randomUUID();
	const massiveId = randomUUID();

	const tx = {
		lastDayLate: randomUUID(),
		nextDayMidnight: randomUUID(),
		nextDayLabel: randomUUID(),
		lastDayLabel: randomUUID(),
		firstDayMidnight: randomUUID(),
		firstDayLabel: randomUUID(),
		beforeFirstDay: randomUUID(),
	};

	function income(
		id: string,
		companyId: string,
		categoryId: string,
		issueDate: Date,
		totalAmount: string,
	) {
		return {
			id,
			companyId,
			categoryId,
			type: "INCOME" as const,
			documentType: "FACTURA" as const,
			issueDate,
			totalAmount,
		};
	}

	beforeAll(async () => {
		await db.insert(users).values({
			id: ownerId,
			email: `t5e-${ownerId}@example.test`,
			password: "not-a-real-password",
			name: "T5e owner",
		});
		const ruc = (n: number) => `20${String(n).padStart(9, "0")}`;
		const seed = Math.floor(Math.random() * 1_000_000_000);
		await db.insert(companies).values([
			// No settingsTimezone: the column default (America/Lima) applies.
			{ id: limaCompanyId, ownerId, ruc: ruc(seed), businessName: "Lima SAC" },
			{
				id: utcCompanyId,
				ownerId,
				ruc: ruc(seed + 1),
				businessName: "UTC SAC",
				settingsTimezone: "UTC",
			},
			{
				id: moneyCompanyId,
				ownerId,
				ruc: ruc(seed + 2),
				businessName: "Money SAC",
			},
			{
				id: otherCompanyId,
				ownerId,
				ruc: ruc(seed + 3),
				businessName: "Other SAC",
			},
		]);
		await db.insert(categories).values([
			{
				id: limaSalesId,
				companyId: limaCompanyId,
				name: "Ventas",
				type: "INCOME",
			},
			{
				id: utcSalesId,
				companyId: utcCompanyId,
				name: "Ventas",
				type: "INCOME",
			},
			{
				id: centsId,
				companyId: moneyCompanyId,
				name: "Centavos",
				type: "EXPENSE",
			},
			{
				id: purchasesId,
				companyId: moneyCompanyId,
				name: "Compras",
				type: "EXPENSE",
			},
			{
				id: massiveId,
				companyId: moneyCompanyId,
				name: "Masivo",
				type: "INCOME",
			},
		]);

		await db.insert(transactions).values([
			// 23:30 Lima on 2026-03-31 (instant).
			income(
				tx.lastDayLate,
				limaCompanyId,
				limaSalesId,
				new Date("2026-04-01T04:30:00.000Z"),
				"100.00",
			),
			// 00:00 Lima on 2026-04-01 (instant).
			income(
				tx.nextDayMidnight,
				limaCompanyId,
				limaSalesId,
				new Date("2026-04-01T05:00:00.000Z"),
				"7.00",
			),
			// Document dated 2026-04-01 (calendar-day label).
			income(
				tx.nextDayLabel,
				limaCompanyId,
				limaSalesId,
				new Date("2026-04-01"),
				"50.00",
			),
			// Document dated 2026-03-31 (calendar-day label).
			income(
				tx.lastDayLabel,
				limaCompanyId,
				limaSalesId,
				new Date("2026-03-31"),
				"20.00",
			),
			// 00:00 Lima on 2026-03-01 (instant).
			income(
				tx.firstDayMidnight,
				limaCompanyId,
				limaSalesId,
				new Date("2026-03-01T05:00:00.000Z"),
				"3.00",
			),
			// Document dated 2026-03-01 (calendar-day label).
			income(
				tx.firstDayLabel,
				limaCompanyId,
				limaSalesId,
				new Date("2026-03-01"),
				"0.50",
			),
			// 23:59:59 Lima on 2026-02-28 (instant).
			income(
				tx.beforeFirstDay,
				limaCompanyId,
				limaSalesId,
				new Date("2026-03-01T04:59:59.000Z"),
				"0.25",
			),

			// Same instant and label for a company whose fiscal zone is UTC.
			income(
				randomUUID(),
				utcCompanyId,
				utcSalesId,
				new Date("2026-04-01T04:30:00.000Z"),
				"100.00",
			),
			income(
				randomUUID(),
				utcCompanyId,
				utcSalesId,
				new Date("2026-03-31"),
				"20.00",
			),

			// Exact money: 0.10 + 0.20 and 3 × the largest storable amount.
			{
				...income(
					randomUUID(),
					moneyCompanyId,
					centsId,
					new Date("2026-03-15"),
					"0.10",
				),
				type: "EXPENSE" as const,
			},
			{
				...income(
					randomUUID(),
					moneyCompanyId,
					centsId,
					new Date("2026-03-15"),
					"0.20",
				),
				type: "EXPENSE" as const,
			},
			{
				...income(
					randomUUID(),
					moneyCompanyId,
					purchasesId,
					new Date("2026-03-15"),
					"9999999999.99",
				),
				type: "EXPENSE" as const,
			},
			{
				...income(
					randomUUID(),
					moneyCompanyId,
					purchasesId,
					new Date("2026-03-15"),
					"9999999999.99",
				),
				type: "EXPENSE" as const,
			},
			{
				...income(
					randomUUID(),
					moneyCompanyId,
					purchasesId,
					new Date("2026-03-15"),
					"9999999999.99",
				),
				type: "EXPENSE" as const,
			},
			income(
				randomUUID(),
				moneyCompanyId,
				massiveId,
				new Date("2026-03-15"),
				"0.01",
			),
			// Another tenant's row pointing at this company's category must not count.
			income(
				randomUUID(),
				otherCompanyId,
				massiveId,
				new Date("2026-03-15"),
				"1.00",
			),
		]);

		// 10 000 × 9 999 999 999.99 (numeric(12,2) maximum) exceeds 2^53 cents:
		// a JS float sum cannot hold it exactly. Bulk insert in SQL (test fixture).
		await db.execute(sql`
			INSERT INTO transactions (company_id, category_id, type, document_type, issue_date, total_amount)
			SELECT ${moneyCompanyId}::uuid, ${massiveId}::uuid, 'INCOME', 'FACTURA', '2026-03-15 00:00:00', 9999999999.99
			FROM generate_series(1, 10000)
		`);
	}, 60_000);

	afterAll(async () => {
		await db
			.delete(transactions)
			.where(inArray(transactions.companyId, companyIds));
		await db
			.delete(categories)
			.where(inArray(categories.companyId, companyIds));
		await db.delete(companies).where(inArray(companies.id, companyIds));
		await db.delete(users).where(inArray(users.id, [ownerId]));
	});

	describe("F1 — whole calendar day in the company's fiscal zone", () => {
		it("trial balance as of YYYY-MM-DD includes 23:30 Lima of that day and excludes the next day", async () => {
			const result = await getTrialBalance(limaCompanyId, day("2026-03-31"));

			// 100.00 (23:30 Lima) + 20.00 (dated 31) + 3.00 + 0.50 + 0.25.
			// Excluded: 7.00 (00:00 Lima on 1 Apr) and 50.00 (dated 1 Apr).
			expect(result.accounts).toEqual([
				{
					code: expect.any(String),
					name: "Ventas",
					debits: "0.00",
					credits: "123.75",
					balance: "-123.75",
					activity: 5,
				},
			]);
			expect(result.credits).toBe("123.75");
			expect(result.debits).toBe("0.00");
			expect(result.balance).toBe("-123.75");
			expect(result.asOfDate).toBe("2026-03-31T00:00:00.000Z");
		});

		it("trial balance as of the previous day excludes the whole last day", async () => {
			const result = await getTrialBalance(limaCompanyId, day("2026-03-30"));

			expect(result.credits).toBe("3.75");
			expect(result.accounts[0]?.activity).toBe(3);
		});

		it("an instant asOfDate resolves to its calendar day in the company's zone", async () => {
			// What a browser in Lima sends for "end of 31 March".
			const result = await getTrialBalance(
				limaCompanyId,
				day("2026-04-01T04:59:59.999Z"),
			);

			expect(result.credits).toBe("123.75");
		});

		it("uses the company's configured fiscal zone (UTC company)", async () => {
			const result = await getTrialBalance(utcCompanyId, day("2026-03-31"));

			// 04:30 UTC on 1 April belongs to 1 April in UTC.
			expect(result.credits).toBe("20.00");
		});

		it("general ledger includes the first day from 00:00 and the whole last day", async () => {
			const entries = await getGeneralLedger(
				limaCompanyId,
				day("2026-03-01"),
				day("2026-03-31"),
			);

			expect(entries.map((entry) => entry.id).sort()).toEqual(
				[
					tx.lastDayLate,
					tx.lastDayLabel,
					tx.firstDayMidnight,
					tx.firstDayLabel,
				].sort(),
			);
		});

		it("general ledger for a single day returns only that day", async () => {
			const entries = await getGeneralLedger(
				limaCompanyId,
				day("2026-04-01"),
				day("2026-04-01"),
			);

			expect(entries.map((entry) => entry.id).sort()).toEqual(
				[tx.nextDayMidnight, tx.nextDayLabel].sort(),
			);
		});
	});

	describe("F2 — exact sums and balances", () => {
		it("sums in SQL and returns exact 2-decimal text", async () => {
			const result = await getTrialBalance(moneyCompanyId, day("2026-03-31"));
			const byName = new Map(result.accounts.map((a) => [a.name, a]));

			expect(byName.get("Centavos")).toMatchObject({
				debits: "0.30",
				credits: "0.00",
				balance: "0.30",
				activity: 2,
			});
			expect(byName.get("Compras")).toMatchObject({
				debits: "29999999999.97",
				credits: "0.00",
				balance: "29999999999.97",
				activity: 3,
			});
			// The other tenant's 1.00 is not counted.
			expect(byName.get("Masivo")).toMatchObject({
				debits: "0.00",
				credits: "99999999999900.01",
				balance: "-99999999999900.01",
				activity: 10_001,
			});

			expect(result.debits).toBe("30000000000.27");
			expect(result.credits).toBe("99999999999900.01");
			expect(result.balance).toBe("-99969999999899.74");
			expect(JSON.stringify(result)).not.toMatch(/e[+-]/i);
		});
	});
});
