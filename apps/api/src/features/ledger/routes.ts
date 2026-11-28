import { Elysia } from "elysia";
import { z } from "zod";
import { createLogger } from "../../lib/logger";
import { companyScopeGuard } from "../../shared/plugins";
import { fail, getErrorMessage, ok } from "../shared/api-response";
import { getChartOfAccounts } from "./application/queries/get-chart-of-accounts.query";
import { getCompanyIdentity } from "./application/queries/get-company-identity.query";
import { getGeneralLedger } from "./application/queries/get-general-ledger.query";
import { getTrialBalance } from "./application/queries/get-trial-balance.query";

const logger = createLogger({ feature: "ledger", handler: "routes" });

const parseIsoDateOrNull = (value: string): Date | null => {
	const parsed = new Date(value);
	return Number.isNaN(parsed.getTime()) ? null : parsed;
};

/**
 * ledgerModule const.
 *
 * Read-only ledger routes. Besides sessions, they accept a read-only ledger
 * key (`drn_ro_…`) scoped to its own company (`allowReadOnlyLedgerKey`).
 * Keep this module GET-only: a write route added here would be refused for
 * keys, but belongs in a module that does not opt in.
 *
 * `GET /key-identity` answers only key callers: it names the key's company
 * (`companyId`, `ruc`, `businessName`) so a native client can check that the
 * key belongs to the workspace (RUC) it is about to label before it shows any
 * ledger data. Sessions and header identity get 403
 * `KEY_IDENTITY_REQUIRES_API_KEY`.
 *
 * @example
 * ```ts
 * console.log(ledgerModule);
 * ```
 */
export const ledgerModule = new Elysia({ prefix: "/api/ledger" })
	.use(
		companyScopeGuard({
			allowHeaderFallback: true,
			allowReadOnlyLedgerKey: true,
		}),
	)
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
				set.status = 500;
				return fail(getErrorMessage(error), "INTERNAL_ERROR");
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
				const startDate = parseIsoDateOrNull(query.startDate);
				const endDate = parseIsoDateOrNull(query.endDate);
				if (!startDate || !endDate) {
					set.status = 400;
					return fail(
						"startDate/endDate invalid format. Use ISO date.",
						"INVALID_DATE",
					);
				}
				if (startDate.getTime() > endDate.getTime()) {
					set.status = 400;
					return fail(
						"startDate must be less than or equal to endDate.",
						"INVALID_DATE_RANGE",
					);
				}

				const result = await getGeneralLedger(
					companyContext.companyId,
					startDate,
					endDate,
				);
				return ok(result);
			} catch (error: unknown) {
				set.status = 500;
				return fail(getErrorMessage(error), "INTERNAL_ERROR");
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
				const asOfDate = parseIsoDateOrNull(query.asOfDate);
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
				set.status = 500;
				return fail(getErrorMessage(error), "INTERNAL_ERROR");
			}
		},
		{
			query: z.object({
				asOfDate: z.string().min(1),
			}),
			detail: { tags: ["Ledger"], summary: "Get trial balance" },
		},
	)
	.get(
		"/key-identity",
		async ({ companyContext, set }) => {
			if (!companyContext) {
				set.status = 401;
				return fail("Company context is required", "COMPANY_CONTEXT_REQUIRED");
			}
			if (companyContext.readOnly !== true) {
				set.status = 403;
				return fail(
					"This endpoint only answers read-only ledger key callers",
					"KEY_IDENTITY_REQUIRES_API_KEY",
				);
			}
			try {
				const identity = await getCompanyIdentity(companyContext.companyId);
				if (!identity) {
					set.status = 404;
					return fail(
						"The key's company was not found",
						"KEY_COMPANY_NOT_FOUND",
					);
				}
				// Built field by field: the answer never carries more than these.
				return ok({
					companyId: identity.companyId,
					ruc: identity.ruc,
					businessName: identity.businessName,
				});
			} catch (error: unknown) {
				// Logged by name only; the cause is not echoed to a key holder.
				logger.error(
					{
						event: "ledger_key_identity_failed",
						companyId: companyContext.companyId,
						errorName: error instanceof Error ? error.name : typeof error,
					},
					"Ledger key identity lookup failed",
				);
				set.status = 500;
				return fail("Could not read the key's company", "INTERNAL_ERROR");
			}
		},
		{
			detail: {
				tags: ["Ledger"],
				summary: "Identify the company of a read-only ledger key",
			},
		},
	);
