import { describe, expect, it } from "vitest";
import {
	ledgerDayToInstant,
	parseLedgerDayBound,
	toLedgerDayBound,
} from "../../_internal/fiscal-day";

describe("parseLedgerDayBound", () => {
	it("reads YYYY-MM-DD as a whole calendar day", () => {
		expect(parseLedgerDayBound("2026-03-31")).toEqual({
			kind: "calendar-day",
			day: "2026-03-31",
		});
		expect(parseLedgerDayBound("2024-02-29")).toEqual({
			kind: "calendar-day",
			day: "2024-02-29",
		});
	});

	it("rejects calendar days that do not exist", () => {
		expect(parseLedgerDayBound("2026-02-30")).toBeNull();
		expect(parseLedgerDayBound("2026-02-29")).toBeNull();
		expect(parseLedgerDayBound("2026-13-01")).toBeNull();
		expect(parseLedgerDayBound("2026-00-10")).toBeNull();
	});

	it("reads an ISO date-time as an instant", () => {
		expect(parseLedgerDayBound("2026-04-01T04:59:59.999Z")).toEqual({
			kind: "instant",
			at: new Date("2026-04-01T04:59:59.999Z"),
		});
	});

	it("reads a date-time at exactly 00:00:00.000 UTC as that calendar day (echoed asOfDate)", () => {
		expect(parseLedgerDayBound("2026-03-31T00:00:00.000Z")).toEqual({
			kind: "calendar-day",
			day: "2026-03-31",
		});
		expect(parseLedgerDayBound("2026-03-31T00:00:00Z")).toEqual({
			kind: "calendar-day",
			day: "2026-03-31",
		});
	});

	it("keeps any other date-time as an instant", () => {
		expect(parseLedgerDayBound("2026-03-31T00:00:00.001Z")).toEqual({
			kind: "instant",
			at: new Date("2026-03-31T00:00:00.001Z"),
		});
		// Lima midnight is 05:00 UTC: an instant, not a UTC-midnight label.
		expect(parseLedgerDayBound("2026-03-31T00:00:00-05:00")).toEqual({
			kind: "instant",
			at: new Date("2026-03-31T05:00:00.000Z"),
		});
	});

	it("rejects values that are not dates", () => {
		expect(parseLedgerDayBound("invalid")).toBeNull();
		expect(parseLedgerDayBound("not-a-date")).toBeNull();
	});
});

describe("ledgerDayToInstant", () => {
	it("echoes a calendar day as its UTC midnight (response contract)", () => {
		expect(
			ledgerDayToInstant({
				kind: "calendar-day",
				day: "2026-03-31",
			}).toISOString(),
		).toBe("2026-03-31T00:00:00.000Z");
	});

	it("reads a Date at exactly 00:00:00.000 UTC as that calendar day (new Date('YYYY-MM-DD'))", () => {
		expect(toLedgerDayBound(new Date("2026-03-31"))).toEqual({
			kind: "calendar-day",
			day: "2026-03-31",
		});
		expect(
			toLedgerDayBound({
				kind: "instant",
				at: new Date("2026-03-31T00:00:00.000Z"),
			}),
		).toEqual({ kind: "calendar-day", day: "2026-03-31" });
		expect(ledgerDayToInstant(new Date("2026-03-31")).toISOString()).toBe(
			"2026-03-31T00:00:00.000Z",
		);
	});

	it("returns an instant unchanged and treats a bare Date as an instant", () => {
		const at = new Date("2026-04-01T04:30:00.000Z");
		expect(ledgerDayToInstant({ kind: "instant", at })).toBe(at);
		expect(ledgerDayToInstant(at)).toBe(at);
		expect(toLedgerDayBound(at)).toEqual({ kind: "instant", at });
	});
});
