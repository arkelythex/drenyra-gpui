import { db as globalDb } from "@drenyra/persistence/client";
import { and, asc, eq, sql } from "@drenyra/persistence/query";
import { categories, transactions } from "@drenyra/persistence/schema";
import {
	type LedgerDayInput,
	ledgerDayToInstant,
} from "../_internal/fiscal-day";
import {
	companyFiscalTimeZone,
	fiscalDayUpToFilter,
	resolveFiscalDay,
} from "../_internal/fiscal-day-sql";
import {
	resolveAccountCode,
	type TransactionDirection,
} from "../_internal/ledger-utils";

/** Row of the trial balance SQL; money columns are decimal text from Postgres. */
interface TrialBalanceRow {
	categoryId: string | null;
	name: string | null;
	direction: TransactionDirection | null;
	activity: number;
	debits: unknown;
	credits: unknown;
	balance: unknown;
	totalDebits: unknown;
	totalCredits: unknown;
	totalBalance: unknown;
}

const EXACT_AMOUNT = /^-?\d+\.\d{2}$/;

/** Money must arrive as exact 2-decimal text; anything else is a defect. */
function exactAmount(value: unknown, field: string): string {
	if (typeof value === "string" && EXACT_AMOUNT.test(value)) return value;
	throw new Error(`Trial balance ${field} is not an exact 2-decimal amount`);
}

/**
 * TrialBalanceAccount interface.
 *
 * @example
 * ```ts
 * const value: TrialBalanceAccount = {} as TrialBalanceAccount;
 * console.log(value);
 * ```
 */
export interface TrialBalanceAccount {
	code: string;
	name: string;
	debits: string;
	credits: string;
	balance: string;
	activity: number;
}

/**
 * TrialBalanceResult interface.
 *
 * @example
 * ```ts
 * const value: TrialBalanceResult = {} as TrialBalanceResult;
 * console.log(value);
 * ```
 */
export interface TrialBalanceResult {
	asOfDate: string;
	debits: string;
	credits: string;
	balance: string;
	accounts: TrialBalanceAccount[];
}

/**
 * Trial balance of a company as of a fiscal day (inclusive, whole day in the
 * company's fiscal zone). Sums and balances are computed in SQL over the
 * `numeric` amounts and returned as exact 2-decimal text.
 */
export async function getTrialBalance(
	companyId: string,
	asOf: LedgerDayInput,
): Promise<TrialBalanceResult> {
	const timeZone = await companyFiscalTimeZone(companyId);
	const asOfDay = await resolveFiscalDay(timeZone, asOf);
	const debitSum = sql`COALESCE(SUM(CASE WHEN ${transactions.type} = 'EXPENSE' THEN ${transactions.totalAmount} END), 0)`;
	const creditSum = sql`COALESCE(SUM(CASE WHEN ${transactions.type} = 'INCOME' THEN ${transactions.totalAmount} END), 0)`;

	const rows: TrialBalanceRow[] = await globalDb
		.select({
			categoryId: categories.id,
			name: categories.name,
			direction: categories.type,
			activity: sql<number>`COUNT(${transactions.id})::int`,
			debits: sql<string>`ROUND(${debitSum}, 2)::text`,
			credits: sql<string>`ROUND(${creditSum}, 2)::text`,
			balance: sql<string>`ROUND(${debitSum} - ${creditSum}, 2)::text`,
			totalDebits: sql<string>`ROUND(SUM(${debitSum}) OVER (), 2)::text`,
			totalCredits: sql<string>`ROUND(SUM(${creditSum}) OVER (), 2)::text`,
			totalBalance: sql<string>`ROUND(SUM(${debitSum} - ${creditSum}) OVER (), 2)::text`,
		})
		.from(categories)
		.leftJoin(
			transactions,
			and(
				eq(transactions.companyId, companyId),
				eq(transactions.categoryId, categories.id),
				fiscalDayUpToFilter(timeZone, asOfDay),
			),
		)
		// Unnamed categories are not listed, so they must not reach the totals.
		.where(
			and(eq(categories.companyId, companyId), sql`${categories.name} <> ''`),
		)
		.groupBy(categories.id, categories.name, categories.type)
		.orderBy(asc(categories.name));

	const accounts = rows
		.map((row, index) => mapTrialBalanceAccount(row, index))
		.filter((row): row is TrialBalanceAccount => row !== null);

	const first = rows[0];

	return {
		asOfDate: ledgerDayToInstant(asOf).toISOString(),
		debits: first ? exactAmount(first.totalDebits, "total debits") : "0.00",
		credits: first ? exactAmount(first.totalCredits, "total credits") : "0.00",
		balance: first ? exactAmount(first.totalBalance, "total balance") : "0.00",
		accounts,
	};
}

function mapTrialBalanceAccount(
	row: TrialBalanceRow,
	index: number,
): TrialBalanceAccount | null {
	if (!row.name) return null;

	return {
		code: resolveAccountCode(row.direction, index),
		name: row.name,
		debits: exactAmount(row.debits, "debits"),
		credits: exactAmount(row.credits, "credits"),
		balance: exactAmount(row.balance, "balance"),
		activity: Number(row.activity ?? 0),
	};
}
