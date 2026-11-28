import { db as globalDb } from "@drenyra/persistence/client";
import { eq } from "@drenyra/persistence/query";
import { companies } from "@drenyra/persistence/schema";

/**
 * Who a company is, as a native client needs it to confirm that a read-only
 * ledger key belongs to the workspace it is about to label.
 *
 * @example
 * ```ts
 * const value: CompanyIdentity = {
 *   companyId: "0b9f…",
 *   ruc: "20123456789",
 *   businessName: "Empresa S.A.C.",
 * };
 * ```
 */
export interface CompanyIdentity {
	companyId: string;
	ruc: string;
	businessName: string;
}

/**
 * Reads the identity (id, RUC, business name) of exactly one company.
 *
 * Tenant-scoped: the only filter is the caller's own `companyId` (from its
 * company context), and only these three columns are selected.
 *
 * @param companyId - The caller's company (company context), never a value
 *   taken from the request.
 * @returns The company identity, or `null` when no row has that id.
 * @example
 * ```ts
 * const identity = await getCompanyIdentity(companyContext.companyId);
 * ```
 */
export async function getCompanyIdentity(
	companyId: string,
): Promise<CompanyIdentity | null> {
	const rows = await globalDb
		.select({
			companyId: companies.id,
			ruc: companies.ruc,
			businessName: companies.businessName,
		})
		.from(companies)
		.where(eq(companies.id, companyId))
		.limit(1);
	return rows[0] ?? null;
}
