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
 * a label of that UTC date (19:00 of the previous day in Lima).
 */
import { type SQL, sql } from "@drenyra/persistence/query";
import { companies, transactions } from "@drenyra/persistence/schema";
import {
	DEFAULT_FISCAL_TIME_ZONE,
	type LedgerDayInput,
	toLedgerDayBound,
} from "./fiscal-day";

/**
 * IANA fiscal time zone of the company (`companies.settings_timezone`), or
 * `America/Lima` when it is not set. Scoped to the requested company only.
 */
export function companyFiscalTimeZone(companyId: string): SQL {
	return sql`COALESCE(NULLIF((SELECT ${companies.settingsTimezone} FROM ${companies} WHERE ${companies.id} = ${companyId}), ''), ${DEFAULT_FISCAL_TIME_ZONE})`;
}

/** Fiscal calendar day (SQL `date`) of `transactions.issue_date`. */
export function transactionFiscalDay(timeZone: SQL): SQL {
	const issueDate = transactions.issueDate;
	return sql`(CASE WHEN ${issueDate} = date_trunc('day', ${issueDate}) THEN ${issueDate}::date ELSE ((${issueDate} AT TIME ZONE 'UTC') AT TIME ZONE ${timeZone})::date END)`;
}

/** Calendar day requested by the caller, as a SQL `date` in the fiscal zone. */
export function requestedFiscalDay(input: LedgerDayInput, timeZone: SQL): SQL {
	const bound = toLedgerDayBound(input);
	if (bound.kind === "calendar-day") {
		return sql`CAST(${bound.day} AS date)`;
	}
	return sql`(CAST(${bound.at.toISOString()} AS timestamptz) AT TIME ZONE ${timeZone})::date`;
}
