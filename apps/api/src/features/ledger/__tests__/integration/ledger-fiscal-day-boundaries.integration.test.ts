/**
 * Fiscal-day boundaries of the trial balance and general ledger (T5e, S1/F1),
 * against a real Postgres. Run with RUN_DB_TESTS=1 and a disposable DATABASE_URL
 * (tables: users, companies, categories, transactions).
 *
 * Amounts are powers of two, so a credit total identifies exactly which rows
 * were included.
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

const ZONES = {
	lima: {
		timeZone: undefined,
		rows: [
			"2026-03-31T00:00:00.000Z", // 1.00  document dated 31 Mar
			"2026-04-01T04:59:59.999Z", // 2.00  last ms of 31 Mar in Lima
			"2026-04-01T05:00:00.000Z", // 4.00  00:00 of 1 Apr in Lima
			"2026-02-28T00:00:00.000Z", // 8.00  document dated 28 Feb
			"2028-02-29T00:00:00.000Z", // 16.00 document dated on a leap day
			"2028-03-01T04:59:59.999Z", // 32.00 last ms of 29 Feb 2028 in Lima
			"2028-03-01T05:00:00.000Z", // 64.00 00:00 of 1 Mar 2028 in Lima
		],
	},
	santiago: {
		timeZone: "America/Santiago",
		rows: [
			"2026-04-05T03:59:59.999Z", // 1.00  4 Apr 23:59 (25-hour day, DST ends)
			"2026-04-05T04:00:00.000Z", // 2.00  5 Apr 00:00
			"2026-09-06T03:59:59.999Z", // 4.00  5 Sep 23:59 (DST starts at midnight)
			"2026-09-06T04:00:00.000Z", // 8.00  6 Sep 01:00
		],
	},
	tokyo: {
		timeZone: "Asia/Tokyo",
		rows: [
			"2026-03-31T14:59:59.999Z", // 1.00  31 Mar 23:59 in Tokyo
			"2026-03-31T15:00:00.000Z", // 2.00  1 Apr 00:00 in Tokyo
		],
	},
	// A value the settings route accepts today but Postgres does not know.
	unknownZone: { timeZone: "Lima", rows: ["2026-04-01T04:30:00.000Z"] },
} as const;

describeDb("ledger fiscal-day boundaries (integration)", () => {
	const ownerId = randomUUID();
	const ids = {} as Record<keyof typeof ZONES, string>;

	beforeAll(async () => {
		await db.insert(users).values({
			id: ownerId,
			email: `t5e-bounds-${ownerId}@example.test`,
			password: "not-a-real-password",
			name: "T5e bounds",
		});
		let ruc = Math.floor(Math.random() * 900_000_000);
		for (const [name, zone] of Object.entries(ZONES)) {
			const companyId = randomUUID();
			const categoryId = randomUUID();
			ids[name as keyof typeof ZONES] = companyId;
			await db.insert(companies).values({
				id: companyId,
				ownerId,
				ruc: `20${String(ruc++).padStart(9, "0")}`,
				businessName: `T5e ${name}`,
				...(zone.timeZone ? { settingsTimezone: zone.timeZone } : {}),
			});
			await db.insert(categories).values({
				id: categoryId,
				companyId,
				name: "Ventas",
				type: "INCOME",
			});
			await db.insert(transactions).values(
				zone.rows.map((iso, index) => ({
					companyId,
					categoryId,
					type: "INCOME" as const,
					documentType: "FACTURA" as const,
					issueDate: new Date(iso),
					totalAmount: `${2 ** index}.00`,
					notes: iso,
				})),
			);
		}
		// Microsecond precision cannot be written through a JS Date.
		await db.execute(sql`
			INSERT INTO transactions (company_id, category_id, type, document_type, issue_date, total_amount, notes)
			SELECT ${ids.lima}::uuid, id, 'INCOME', 'FACTURA', '2026-04-01 04:59:59.999999', 128.00, 'last microsecond of 31 Mar'
			FROM categories WHERE company_id = ${ids.lima}::uuid
		`);
	}, 60_000);

	afterAll(async () => {
		const companyIds = Object.values(ids);
		await db
			.delete(transactions)
			.where(inArray(transactions.companyId, companyIds));
		await db
			.delete(categories)
			.where(inArray(categories.companyId, companyIds));
		await db.delete(companies).where(inArray(companies.id, companyIds));
		await db.delete(users).where(inArray(users.id, [ownerId]));
	});

	const credits = async (company: keyof typeof ZONES, asOf: string) =>
		(await getTrialBalance(ids[company], day(asOf))).credits;

	it("includes the last millisecond and microsecond of the day in Lima, not the next day", async () => {
		// 8 (28 Feb) + 1 (31 Mar) + 2 (23:59:59.999) + 128 (23:59:59.999999).
		expect(await credits("lima", "2026-03-31")).toBe("139.00");
		expect(await credits("lima", "2026-03-30")).toBe("8.00");
		expect(await credits("lima", "2026-04-01")).toBe("143.00");
	});

	it("handles a leap day", async () => {
		expect(await credits("lima", "2028-02-28")).toBe("143.00");
		expect(await credits("lima", "2028-02-29")).toBe("191.00");
		expect(parseLedgerDayBound("2026-02-29")).toBeNull();
		expect(parseLedgerDayBound("2100-02-29")).toBeNull();
	});

	it("follows daylight-saving changes of the company zone (America/Santiago)", async () => {
		expect(await credits("santiago", "2026-04-04")).toBe("1.00");
		expect(await credits("santiago", "2026-04-05")).toBe("3.00");
		expect(await credits("santiago", "2026-09-05")).toBe("7.00");
		expect(await credits("santiago", "2026-09-06")).toBe("15.00");
	});

	it("works for a zone ahead of UTC (Asia/Tokyo)", async () => {
		expect(await credits("tokyo", "2026-03-31")).toBe("1.00");
		expect(await credits("tokyo", "2026-04-01")).toBe("3.00");
	});

	// B2 — the API echoes asOfDate as UTC midnight; sending it back must mean the same day.
	it("reads a UTC-midnight date-time as that calendar day, like YYYY-MM-DD", async () => {
		expect(await credits("lima", "2026-03-31T00:00:00.000Z")).toBe("139.00");
	});

	// B2 — callers that build a Date from YYYY-MM-DD (agent tool get_ledger_entry).
	it("general ledger with new Date('YYYY-MM-DD') bounds covers exactly those days", async () => {
		const entries = await getGeneralLedger(
			ids.lima,
			new Date("2026-03-01"),
			new Date("2026-03-31"),
		);
		expect(entries.map((entry) => entry.glosa).sort()).toEqual([
			"2026-03-31T00:00:00.000Z",
			"2026-04-01T04:59:59.999Z",
			"last microsecond of 31 Mar",
		]);
	});

	// B1 — a time zone Postgres does not recognise must not turn the report into a 500.
	it("does not fail when the company time zone is not a known zone", async () => {
		await expect(
			getTrialBalance(ids.unknownZone, day("2026-03-31")),
		).resolves.toMatchObject({ credits: "1.00" });
	});
});
