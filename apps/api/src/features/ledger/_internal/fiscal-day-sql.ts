/**
 * SQL for fiscal-day filters on `transactions.issue_date`.
 *
 * `issue_date` is `timestamp without time zone`. Drizzle writes a JS `Date` as
 * its UTC wall clock, and writers store two kinds of values:
 * - calendar-day labels: `new Date("YYYY-MM-DD")` → exactly 00:00:00 (UTC
 *   midnight of the document date), e.g. UBL `cbc:IssueDate` in the inbox;
 * - instants: `new Date()` → the UTC wall clock of the moment, e.g. manual
 *   transactions and inter-company postings.
 *
 * The fiscal day of a row is therefore: the stored date for an exact-midnight
 * label, otherwise the calendar day of the UTC instant in the company's fiscal
 * zone. "Day ≤ D" is the exclusive upper bound "before the start of D + 1".
 * Known ambiguity: an instant stored at exactly 00:00:00.000000 UTC is read as
 * a label of that UTC date (19:00 of the previous day in Lima). Requested
 * bounds follow the same rule (see `toLedgerDayBound`).
 */
import { db as globalDb } from "@drenyra/persistence/client";
import { type SQL, sql } from "@drenyra/persistence/query";
import { companies, transactions } from "@drenyra/persistence/schema";
import { createLogger } from "../../../lib/logger";
import {
	DEFAULT_FISCAL_TIME_ZONE,
	isValidCalendarDay,
	type LedgerDayBound,
	type LedgerDayInput,
	toLedgerDayBound,
} from "./fiscal-day";
import { knownTimeZoneNames } from "./known-time-zones";

const logger = createLogger({ feature: "ledger", component: "fiscal-day" });

/** Longest stored zone value written to the fallback warning. */
const LOGGED_ZONE_MAX_LENGTH = 64;

/**
 * IANA fiscal time zone of the company (`companies.settings_timezone`), or
 * `America/Lima` when it is not set or is not exactly (after trimming) a zone
 * name Postgres knows (`pg_timezone_names`, cached per process, see
 * `knownTimeZoneNames`). An unknown value therefore never reaches
 * `AT TIME ZONE`, where it would fail the whole report; a non-blank value that
 * falls back is logged once per call as a warning. Scoped to the requested
 * company only. Resolved once per report.
 *
 * @returns The zone name, to pass to the fiscal-day SQL helpers.
 */
export async function companyFiscalTimeZone(
	companyId: string,
): Promise<string> {
	const rows = await globalDb.execute<{ zone: unknown }>(
		sql`SELECT ${companies.settingsTimezone} AS zone FROM ${companies} WHERE ${companies.id} = ${companyId}`,
	);
	const stored = rows[0]?.zone;
	const configured = typeof stored === "string" ? stored.trim() : "";
	if (configured.length === 0) return DEFAULT_FISCAL_TIME_ZONE;

	const known = await knownTimeZoneNames();
	if (known.has(configured)) return configured;

	logger.warn(
		{
			companyId,
			storedTimeZone: configured.slice(0, LOGGED_ZONE_MAX_LENGTH),
			fallbackTimeZone: DEFAULT_FISCAL_TIME_ZONE,
		},
		`Company fiscal time zone is not a known zone; falling back to ${DEFAULT_FISCAL_TIME_ZONE}`,
	);
	return DEFAULT_FISCAL_TIME_ZONE;
}

/**
 * Fiscal days (`YYYY-MM-DD`) of the bounds in `timeZone`. Calendar-day bounds
 * need no query; instants are converted by Postgres in one query, with the
 * same `AT TIME ZONE` rules the row filter uses.
 */
async function resolveFiscalDays(
	timeZone: string,
	bounds: readonly LedgerDayBound[],
): Promise<string[]> {
	const instants = bounds.filter(
		(bound): bound is Extract<LedgerDayBound, { kind: "instant" }> =>
			bound.kind === "instant",
	);
	let resolved: string[] = [];
	if (instants.length > 0) {
		const days = instants
			.map(
				(bound) =>
					sql`to_char(CAST(${bound.at.toISOString()} AS timestamptz) AT TIME ZONE ${timeZone}, 'YYYY-MM-DD')`,
			)
			.reduce((list, day) => sql`${list}, ${day}`);
		const rows = await globalDb.execute<{ days: unknown }>(
			sql`SELECT concat_ws(',', ${days}) AS days`,
		);
		const text = rows[0]?.days;
		resolved = typeof text === "string" ? text.split(",") : [];
		if (
			resolved.length !== instants.length ||
			!resolved.every(isValidCalendarDay)
		) {
			throw new Error("Could not resolve the fiscal day of a ledger date");
		}
	}
	let next = 0;
	return bounds.map((bound) => {
		if (bound.kind === "calendar-day") return bound.day;
		const day = resolved[next++];
		if (day === undefined) {
			throw new Error("Could not resolve the fiscal day of a ledger date");
		}
		return day;
	});
}

/** Fiscal day (`YYYY-MM-DD`) of one requested bound in `timeZone`. */
export async function resolveFiscalDay(
	timeZone: string,
	input: LedgerDayInput,
): Promise<string> {
	const [day] = await resolveFiscalDays(timeZone, [toLedgerDayBound(input)]);
	if (day === undefined) {
		throw new Error("Could not resolve the fiscal day of a ledger date");
	}
	return day;
}

/** Fiscal days (`YYYY-MM-DD`) of a requested range in `timeZone`. */
export async function resolveFiscalDayRange(
	timeZone: string,
	start: LedgerDayInput,
	end: LedgerDayInput,
): Promise<{ startDay: string; endDay: string }> {
	const [startDay, endDay] = await resolveFiscalDays(timeZone, [
		toLedgerDayBound(start),
		toLedgerDayBound(end),
	]);
	if (startDay === undefined || endDay === undefined) {
		throw new Error("Could not resolve the fiscal day of a ledger date");
	}
	return { startDay, endDay };
}

/** Fiscal calendar day (SQL `date`) of `transactions.issue_date`. */
export function transactionFiscalDay(timeZone: string): SQL {
	const issueDate = transactions.issueDate;
	return sql`(CASE WHEN ${issueDate} = date_trunc('day', ${issueDate}) THEN ${issueDate}::date ELSE ((${issueDate} AT TIME ZONE 'UTC') AT TIME ZONE ${timeZone})::date END)`;
}

/*
 * Sargable prefilter. A row whose fiscal day is D has `issue_date` (UTC wall
 * clock) in [D - 1 day, D + 2 days): a label is exactly D 00:00, and an
 * instant of local day D is less than 24 h away from D in UTC for any zone
 * offset (Postgres zones stay within -12 h .. +14 h). The prefilter compares
 * the bare column with constants, so an index on `issue_date` can serve it; the
 * fiscal-day predicate stays the exact filter.
 */

/** Rows whose fiscal day is in [startDay, endDay] (both `YYYY-MM-DD`). */
export function fiscalDayRangeFilter(
	timeZone: string,
	startDay: string,
	endDay: string,
): SQL {
	const issueDate = transactions.issueDate;
	const fiscalDay = transactionFiscalDay(timeZone);
	return sql`(${issueDate} >= (CAST(${startDay} AS date) - 1)::timestamp AND ${issueDate} < (CAST(${endDay} AS date) + 2)::timestamp AND ${fiscalDay} >= CAST(${startDay} AS date) AND ${fiscalDay} <= CAST(${endDay} AS date))`;
}

/** Rows whose fiscal day is on or before asOfDay (`YYYY-MM-DD`). */
export function fiscalDayUpToFilter(timeZone: string, asOfDay: string): SQL {
	const issueDate = transactions.issueDate;
	return sql`(${issueDate} < (CAST(${asOfDay} AS date) + 2)::timestamp AND ${transactionFiscalDay(timeZone)} <= CAST(${asOfDay} AS date))`;
}
