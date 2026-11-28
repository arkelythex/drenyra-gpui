import type { LedgerDayInput } from "../../_internal/fiscal-day";
import type { GeneralLedgerEntry, TrialBalanceResult } from "../../queries";
import { getChartOfAccounts } from "../../queries/get-chart-of-accounts.query";
import { getGeneralLedger } from "../../queries/get-general-ledger.query";
import { getTrialBalance } from "../../queries/get-trial-balance.query";

/**
 * LedgerQueryService class.
 *
 * @example
 * ```ts
 * const value = new LedgerQueryService();
 * console.log(value);
 * ```
 */
export class LedgerQueryService {
	async getChartOfAccounts(companyId: string): Promise<
		Array<{
			code: string;
			name: string;
			type: "ASSET" | "LIABILITY" | "REVENUE" | "EXPENSE";
			activity: number;
			totalDebit: string;
			totalCredit: string;
			balance: string;
		}>
	> {
		return getChartOfAccounts(companyId);
	}

	async getGeneralLedger(
		companyId: string,
		startDate: LedgerDayInput,
		endDate: LedgerDayInput,
	): Promise<GeneralLedgerEntry[]> {
		return getGeneralLedger(companyId, startDate, endDate);
	}

	async getTrialBalance(
		companyId: string,
		asOfDate: LedgerDayInput,
	): Promise<TrialBalanceResult> {
		return getTrialBalance(companyId, asOfDate);
	}
}
