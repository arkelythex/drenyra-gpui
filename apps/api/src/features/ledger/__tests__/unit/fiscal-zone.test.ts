/**
 * Company fiscal zone resolution and fiscal-day SQL filters:
 * - R3-002/R4-003: a stored zone that falls back to America/Lima is logged.
 * - R4-004: known zone names are cached per process with a TTL.
 * - R4-001/R3-004: date filters carry a sargable `issue_date` prefilter.
 */
import type { SQL } from "drizzle-orm";
import { PgDialect } from "drizzle-orm/pg-core";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const mocks = vi.hoisted(() => ({
	execute: vi.fn(),
	warn: vi.fn(),
}));

vi.mock("@drenyra/persistence/client", () => ({
	db: { execute: mocks.execute },
}));

vi.mock("../../../../lib/logger", () => ({
	createLogger: () => ({ warn: mocks.warn, error: vi.fn(), info: vi.fn() }),
}));

const {
	KNOWN_TIME_ZONES_TTL_MS,
	knownTimeZoneNames,
	resetKnownTimeZoneNamesCache,
} = await import("../../_internal/known-time-zones");
const { companyFiscalTimeZone, fiscalDayRangeFilter, fiscalDayUpToFilter } =
	await import("../../_internal/fiscal-day-sql");

const dialect = new PgDialect();
const render = (query: SQL) => dialect.sqlToQuery(query);

const ZONE_NAMES = [
	{ name: "America/Lima" },
	{ name: "Asia/Tokyo" },
	{ name: "UTC" },
];

/** Answers the zone-name scan and the company row by the SQL they send. */
function answerQueries(storedZone: string | null) {
	mocks.execute.mockImplementation(async (query: SQL) =>
		render(query).sql.includes("pg_timezone_names")
			? ZONE_NAMES
			: [{ zone: storedZone }],
	);
}

const zoneScans = () =>
	mocks.execute.mock.calls.filter(([query]) =>
		render(query as SQL).sql.includes("pg_timezone_names"),
	).length;

beforeEach(() => {
	resetKnownTimeZoneNamesCache();
	mocks.execute.mockReset();
	mocks.warn.mockReset();
});

afterEach(() => {
	vi.useRealTimers();
});

describe("knownTimeZoneNames — per-process cache", () => {
	it("scans pg_timezone_names once for two calls", async () => {
		answerQueries(null);

		const first = await knownTimeZoneNames();
		const second = await knownTimeZoneNames();

		expect(mocks.execute).toHaveBeenCalledOnce();
		expect(first.has("America/Lima")).toBe(true);
		expect(second).toBe(first);
	});

	it("shares one scan between concurrent calls", async () => {
		answerQueries(null);

		await Promise.all([knownTimeZoneNames(), knownTimeZoneNames()]);

		expect(mocks.execute).toHaveBeenCalledOnce();
	});

	it("scans again once the TTL (1 h) has passed", async () => {
		vi.useFakeTimers({ toFake: ["Date"] });
		vi.setSystemTime(new Date("2026-11-27T10:00:00.000Z"));
		answerQueries(null);

		await knownTimeZoneNames();
		vi.setSystemTime(Date.now() + KNOWN_TIME_ZONES_TTL_MS - 1);
		await knownTimeZoneNames();
		expect(mocks.execute).toHaveBeenCalledOnce();

		vi.setSystemTime(Date.now() + 2);
		await knownTimeZoneNames();
		expect(mocks.execute).toHaveBeenCalledTimes(2);
		expect(KNOWN_TIME_ZONES_TTL_MS).toBe(3_600_000);
	});

	it("does not cache an empty answer", async () => {
		mocks.execute.mockResolvedValueOnce([]).mockResolvedValue(ZONE_NAMES);

		expect((await knownTimeZoneNames()).size).toBe(0);
		expect((await knownTimeZoneNames()).has("UTC")).toBe(true);
		expect(mocks.execute).toHaveBeenCalledTimes(2);
	});
});

describe("companyFiscalTimeZone", () => {
	it("returns a known stored zone (trimmed) without warning", async () => {
		answerQueries("  Asia/Tokyo ");

		await expect(companyFiscalTimeZone("cmp-1")).resolves.toBe("Asia/Tokyo");
		expect(mocks.warn).not.toHaveBeenCalled();
	});

	it("reads the zone of the requested company only", async () => {
		answerQueries("UTC");

		await companyFiscalTimeZone("cmp-42");

		const companyQuery = mocks.execute.mock.calls
			.map(([query]) => render(query as SQL))
			.find((query) => query.sql.includes("settings_timezone"));
		expect(companyQuery?.sql).toContain('"companies"."id" = $1');
		expect(companyQuery?.params).toEqual(["cmp-42"]);
	});

	it("falls back to America/Lima for an unknown stored zone and logs one warning", async () => {
		answerQueries(" Lima ");

		await expect(companyFiscalTimeZone("cmp-1")).resolves.toBe("America/Lima");
		expect(mocks.warn).toHaveBeenCalledOnce();
		expect(mocks.warn).toHaveBeenCalledWith(
			{
				companyId: "cmp-1",
				storedTimeZone: "Lima",
				fallbackTimeZone: "America/Lima",
			},
			"Company fiscal time zone is not a known zone; falling back to America/Lima",
		);
	});

	it("logs at most 64 characters of the stored value", async () => {
		answerQueries(`Zone/${"x".repeat(100)}`);

		await companyFiscalTimeZone("cmp-1");

		const [fields] = mocks.warn.mock.calls[0] ?? [];
		expect(fields.storedTimeZone).toBe(`Zone/${"x".repeat(59)}`);
		expect(fields.storedTimeZone).toHaveLength(64);
	});

	it.each([[null], [""], ["   "]])(
		"uses America/Lima without warning when the stored zone is %j",
		async (stored) => {
			answerQueries(stored);

			await expect(companyFiscalTimeZone("cmp-1")).resolves.toBe(
				"America/Lima",
			);
			expect(mocks.warn).not.toHaveBeenCalled();
		},
	);

	it("reuses the cached zone names across reports", async () => {
		answerQueries("Asia/Tokyo");

		await companyFiscalTimeZone("cmp-1");
		await companyFiscalTimeZone("cmp-2");

		expect(zoneScans()).toBe(1);
	});
});

describe("fiscal-day filters — sargable issue_date prefilter", () => {
	it("general ledger: issue_date >= start - 1 day AND issue_date < end + 2 days, ANDed with the fiscal-day range", () => {
		const { sql, params } = render(
			fiscalDayRangeFilter("America/Lima", "2026-03-01", "2026-03-31"),
		);

		expect(sql).toMatch(
			/"transactions"\."issue_date" >= \(CAST\(\$(\d+) AS date\) - 1\)::timestamp AND "transactions"\."issue_date" < \(CAST\(\$(\d+) AS date\) \+ 2\)::timestamp AND /,
		);
		const [, startParam, endParam] =
			/>= \(CAST\(\$(\d+) AS date\) - 1\).*< \(CAST\(\$(\d+) AS date\) \+ 2\)/.exec(
				sql,
			) ?? [];
		expect(params[Number(startParam) - 1]).toBe("2026-03-01");
		expect(params[Number(endParam) - 1]).toBe("2026-03-31");
		expect(sql).toContain("AT TIME ZONE");
		expect(params).toContain("America/Lima");
	});

	it("trial balance: only issue_date < as_of + 2 days, ANDed with the fiscal-day bound", () => {
		const { sql, params } = render(
			fiscalDayUpToFilter("America/Lima", "2026-03-31"),
		);

		expect(sql).toMatch(
			/^\("transactions"\."issue_date" < \(CAST\(\$1 AS date\) \+ 2\)::timestamp AND /,
		);
		expect(sql).not.toMatch(/"issue_date" >=/);
		expect(params[0]).toBe("2026-03-31");
	});
});
