/**
 * Fiscal-day semantics for ledger date filters.
 *
 * Ledger reports work on whole calendar days in the company's fiscal time
 * zone (`companies.settings_timezone`, default `America/Lima`). A caller asks
 * for a day either as a calendar day (`YYYY-MM-DD`, the preferred form) or as
 * an instant (an ISO date-time), which is resolved to its calendar day in the
 * company's zone.
 */

/** A ledger day as requested by the caller. */
export type LedgerDayBound =
	| { readonly kind: "calendar-day"; readonly day: string }
	| { readonly kind: "instant"; readonly at: Date };

/** Accepted by the ledger queries; a bare `Date` is treated as an instant. */
export type LedgerDayInput = LedgerDayBound | Date;

/** Fiscal time zone used when the company has none configured. */
export const DEFAULT_FISCAL_TIME_ZONE = "America/Lima";

const CALENDAR_DAY_PATTERN = /^(\d{4})-(\d{2})-(\d{2})$/;

function isValidCalendarDay(value: string): boolean {
	const match = CALENDAR_DAY_PATTERN.exec(value);
	if (!match) return false;
	const year = Number(match[1]);
	const month = Number(match[2]);
	const day = Number(match[3]);
	const probe = new Date(Date.UTC(year, month - 1, day));
	return (
		probe.getUTCFullYear() === year &&
		probe.getUTCMonth() === month - 1 &&
		probe.getUTCDate() === day
	);
}

/**
 * Parse a ledger date query value.
 *
 * @param value - `YYYY-MM-DD` (calendar day) or an ISO date-time (instant).
 * @returns The bound, or `null` when the value is not a valid date.
 * @example
 * ```ts
 * parseLedgerDayBound("2026-03-31"); // { kind: "calendar-day", day: "2026-03-31" }
 * ```
 */
export function parseLedgerDayBound(value: string): LedgerDayBound | null {
	if (CALENDAR_DAY_PATTERN.test(value)) {
		return isValidCalendarDay(value)
			? { kind: "calendar-day", day: value }
			: null;
	}
	const at = new Date(value);
	return Number.isNaN(at.getTime()) ? null : { kind: "instant", at };
}

/** Normalise a query input to a bound (`Date` → instant). */
export function toLedgerDayBound(input: LedgerDayInput): LedgerDayBound {
	return input instanceof Date ? { kind: "instant", at: input } : input;
}

/**
 * Representative instant of a bound, used to order two bounds and to echo the
 * requested date in responses (`YYYY-MM-DD` → `YYYY-MM-DDT00:00:00.000Z`).
 */
export function ledgerDayToInstant(input: LedgerDayInput): Date {
	const bound = toLedgerDayBound(input);
	return bound.kind === "instant"
		? bound.at
		: new Date(`${bound.day}T00:00:00.000Z`);
}
