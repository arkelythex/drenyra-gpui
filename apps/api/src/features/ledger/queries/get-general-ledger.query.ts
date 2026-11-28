import { db as globalDb } from "@drenyra/persistence/client";
import { and, desc, eq, sql } from "@drenyra/persistence/query";
import { categories, transactions } from "@drenyra/persistence/schema";
import type { LedgerDayInput } from "../_internal/fiscal-day";
import {
	companyFiscalTimeZone,
	requestedFiscalDay,
	transactionFiscalDay,
} from "../_internal/fiscal-day-sql";
import { formatVoucher, toNumber } from "../_internal/ledger-utils";

/**
 * GeneralLedgerEntry interface.
 *
 * @example
 * ```ts
 * const value: GeneralLedgerEntry = {} as GeneralLedgerEntry;
 * console.log(value);
 * ```
 */
export interface GeneralLedgerEntry {
	id: string;
	date: string;
	voucher: string;
	glosa: string;
	cuenta: string;
	debe: number;
	haber: number;
	doc: string;
	bancarizado: boolean;
}

/**
 * General ledger entries between two fiscal days (both inclusive, whole days
 * in the company's fiscal zone).
 */
export async function getGeneralLedger(
	companyId: string,
	start: LedgerDayInput,
	end: LedgerDayInput,
): Promise<GeneralLedgerEntry[]> {
	const timeZone = await companyFiscalTimeZone(companyId);
	const fiscalDay = transactionFiscalDay(timeZone);
	const rows = await globalDb
		.select({
			id: transactions.id,
			issueDate: transactions.issueDate,
			series: transactions.series,
			number: transactions.number,
			notes: transactions.notes,
			documentType: transactions.documentType,
			totalAmount: transactions.totalAmount,
			direction: transactions.type,
			categoryName: categories.name,
		})
		.from(transactions)
		.leftJoin(categories, eq(categories.id, transactions.categoryId))
		.where(
			and(
				eq(transactions.companyId, companyId),
				sql`${fiscalDay} >= ${requestedFiscalDay(start, timeZone)}`,
				sql`${fiscalDay} <= ${requestedFiscalDay(end, timeZone)}`,
			),
		)
		.orderBy(desc(transactions.issueDate), desc(transactions.createdAt));

	return rows.map((row) => {
		const amount = toNumber(row.totalAmount);
		const isIncome = row.direction === "INCOME";
		const voucher = formatVoucher(row.series, row.number, row.id);
		const docParts = [row.documentType, row.series, row.number].filter(
			(value): value is string =>
				typeof value === "string" && value.trim().length > 0,
		);

		return {
			id: row.id,
			date: row.issueDate.toISOString(),
			voucher,
			glosa: row.notes?.trim().length
				? row.notes
				: `${row.documentType} ${voucher}`,
			cuenta: row.categoryName ?? "SIN CATEGORIA",
			debe: isIncome ? 0 : amount,
			haber: isIncome ? amount : 0,
			doc: docParts.join(" "),
			bancarizado: row.documentType === "MOVIMIENTO_BANCARIO",
		};
	});
}
