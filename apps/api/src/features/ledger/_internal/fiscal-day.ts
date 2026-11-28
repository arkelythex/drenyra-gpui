/**
 * Fiscal-day semantics for ledger date filters.
 *
 * Ledger reports work on whole calendar days in the company's fiscal time
 * zone (`companies.settings_timezone`, default `America/Lima`). A caller asks
 * for a day either as a calendar day (`YYYY-MM-DD`, the preferred form) or as
 * an instant (an ISO date-time), which is resolved to its calendar day in the
 * company's zone.
 *
 * Label rule (same as for stored rows): a value at exactly 00:00:00.000 UTC is
 * a calendar-day label of that UTC date, not an instant. The API echoes a day
 * as `YYYY-MM-DDT00:00:00.000Z` and `new Date("YYYY-MM-DD")` builds that same
 * value, so sending either back means that calendar day.
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

const DAY_MS = 86_400_000;

/** Whether `value` is a `YYYY-MM-DD` day that exists in the calendar. */
export function isValidCalendarDay(value: string): boolean {
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
	return Number.isNaN(at.getTime()) ? null : instantBound(at);
}

/**
 * Bound for an instant, applying the label rule: exactly 00:00:00.000 UTC is
 * the calendar day of that UTC date; any other instant stays an instant.
 */
function instantBound(at: Date): LedgerDayBound {
	if (at.getTime() % DAY_MS === 0) {
		const day = at.toISOString().slice(0, 10);
		if (isValidCalendarDay(day)) return { kind: "calendar-day", day };
	}
	return { kind: "instant", at };
}

/**
 * Normalise a query input to a bound. A `Date` or an instant at exactly
 * 00:00:00.000 UTC becomes that calendar day; any other one stays an instant.
 */
export function toLedgerDayBound(input: LedgerDayInput): LedgerDayBound {
	if (input instanceof Date) return instantBound(input);
	return input.kind === "instant" ? instantBound(input.at) : input;
}

/**
 * Representative instant of a bound, used to echo the requested date in
 * responses and report headers (`YYYY-MM-DD` → `YYYY-MM-DDT00:00:00.000Z`).
 * Never use it to order two bounds: an instant's day depends on the company
 * zone (see `isInvertedCalendarDayRange` and `LedgerDateRangeError`).
 */
export function ledgerDayToInstant(input: LedgerDayInput): Date {
	const bound = toLedgerDayBound(input);
	return bound.kind === "instant"
		? bound.at
		: new Date(`${bound.day}T00:00:00.000Z`);
}

/**
 * Whether both bounds are calendar days and the start day is after the end
 * day. Decidable without the company zone, so callers can refuse it before any
 * query. A range with an instant is ordered by the query, on fiscal days in the
 * company zone, which throws `LedgerDateRangeError` when it is inverted.
 */
export function isInvertedCalendarDayRange(
	start: LedgerDayInput,
	end: LedgerDayInput,
): boolean {
	const first = toLedgerDayBound(start);
	const last = toLedgerDayBound(end);
	return (
		first.kind === "calendar-day" &&
		last.kind === "calendar-day" &&
		first.day > last.day
	);
}

/**
 * The requested start day is after the end day once both bounds are resolved
 * to fiscal days in the company zone. Routes answer it with
 * `400 INVALID_DATE_RANGE` instead of an empty ledger.
 */
export class LedgerDateRangeError extends Error {
	readonly code = "INVALID_DATE_RANGE";

	constructor(
		readonly startDay: string,
		readonly endDay: string,
	) {
		super(
			`Ledger start day ${startDay} is after end day ${endDay} in the company fiscal zone`,
		);
		this.name = "LedgerDateRangeError";
	}
}
