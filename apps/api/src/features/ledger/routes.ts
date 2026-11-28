import { Elysia } from "elysia";
import { z } from "zod";
import { createLogger } from "../../lib/logger";
import { companyScopeGuard } from "../../shared/plugins";
import { fail, ok } from "../shared/api-response";
import {
	isInvertedCalendarDayRange,
	LedgerDateRangeError,
	parseLedgerDayBound,
} from "./_internal/fiscal-day";
import { describeQueryError } from "./_internal/query-error-log";
import { getChartOfAccounts } from "./application/queries/get-chart-of-accounts.query";
import { getGeneralLedger } from "./application/queries/get-general-ledger.query";
import { getTrialBalance } from "./application/queries/get-trial-balance.query";

const logger = createLogger({ feature: "ledger", handler: "routes" });

/**
 * 500 answer for a failed ledger query. The body is a fixed message: driver
 * errors carry SQL text and parameters, which must not reach the client. The
 * log keeps the error name, the driver code/severity (e.g. Postgres SQLSTATE)
 * and a sanitized message, never SQL text or parameters.
 */
function ledgerQueryFailed(
	route: string,
	error: unknown,
	set: { status?: number | string },
) {
	logger.error({ route, ...describeQueryError(error) }, "Ledger query failed");
	set.status = 500;
	return fail("Ledger query failed", "INTERNAL_ERROR");
}

/** 400 answer for a start day after the end day. */
function invalidDateRange(set: { status?: number | string }) {
	set.status = 400;
	return fail(
		"startDate must be less than or equal to endDate.",
		"INVALID_DATE_RANGE",
	);
}

/**
 * ledgerModule const.
 *
 * @example
 * ```ts
 * console.log(ledgerModule);
 * ```
 */
export const ledgerModule = new Elysia({ prefix: "/api/ledger" })
	.use(companyScopeGuard({ allowHeaderFallback: true }))
	.get(
		"/accounts",
		async ({ companyContext, set }) => {
			if (!companyContext) {
				set.status = 401;
				return fail("Company context is required", "COMPANY_CONTEXT_REQUIRED");
			}
			try {
				const result = await getChartOfAccounts(companyContext.companyId);
				return ok(result);
			} catch (error: unknown) {
				return ledgerQueryFailed("/api/ledger/accounts", error, set);
			}
		},
		{
			detail: { tags: ["Ledger"], summary: "Get chart of accounts" },
		},
	)
	.get(
		"/general",
		async ({ query, companyContext, set }) => {
			if (!companyContext) {
				set.status = 401;
				return fail("Company context is required", "COMPANY_CONTEXT_REQUIRED");
			}
			try {
				const startDate = parseLedgerDayBound(query.startDate);
				const endDate = parseLedgerDayBound(query.endDate);
				if (!startDate || !endDate) {
					set.status = 400;
					return fail(
						"startDate/endDate invalid format. Use ISO date.",
						"INVALID_DATE",
					);
				}
				// Two calendar days are ordered here; a range with an instant is
				// ordered by the query on fiscal days in the company zone.
				if (isInvertedCalendarDayRange(startDate, endDate)) {
					return invalidDateRange(set);
				}

				const result = await getGeneralLedger(
					companyContext.companyId,
					startDate,
					endDate,
				);
				return ok(result);
			} catch (error: unknown) {
				if (error instanceof LedgerDateRangeError) {
					return invalidDateRange(set);
				}
				return ledgerQueryFailed("/api/ledger/general", error, set);
			}
		},
		{
			query: z.object({
				startDate: z.string().min(1),
				endDate: z.string().min(1),
			}),
			detail: { tags: ["Ledger"], summary: "Get general ledger" },
		},
	)
	.get(
		"/trial-balance",
		async ({ query, companyContext, set }) => {
			if (!companyContext) {
				set.status = 401;
				return fail("Company context is required", "COMPANY_CONTEXT_REQUIRED");
			}
			try {
				const asOfDate = parseLedgerDayBound(query.asOfDate);
				if (!asOfDate) {
					set.status = 400;
					return fail("asOfDate invalid format. Use ISO date.", "INVALID_DATE");
				}

				const result = await getTrialBalance(
					companyContext.companyId,
					asOfDate,
				);
				return ok(result);
			} catch (error: unknown) {
				return ledgerQueryFailed("/api/ledger/trial-balance", error, set);
			}
		},
		{
			query: z.object({
				asOfDate: z.string().min(1),
			}),
			detail: { tags: ["Ledger"], summary: "Get trial balance" },
		},
	);
