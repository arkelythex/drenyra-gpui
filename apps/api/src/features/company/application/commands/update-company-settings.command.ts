import { companies } from "@drenyra/infrastructure";
import { db } from "@drenyra/persistence/client";
import { eq, sql } from "drizzle-orm";
import type { CompanySettingsResult } from "../queries/get-company-settings.query";

export interface UpdateCompanySettingsInput {
	companyId: string;
	language?: string;
	timezone?: string;
	currency?: string;
	companyName?: string;
	companyRuc?: string;
	autoClosePeriod?: boolean;
	showAmountsInWords?: boolean;
}

function serializeCompanySettings(
	company: typeof companies.$inferSelect,
): CompanySettingsResult {
	return {
		language: company.settingsLanguage,
		timezone: company.settingsTimezone,
		currency: company.settingsCurrency,
		companyName: company.businessName,
		companyRuc: company.ruc,
		autoClosePeriod: company.settingsAutoClosePeriod,
		showAmountsInWords: company.settingsShowAmountsInWords,
	};
}

/** Outcome of a settings update. */
export type UpdateCompanySettingsResult =
	| { readonly status: "updated"; readonly settings: CompanySettingsResult }
	| { readonly status: "not_found" }
	| { readonly status: "invalid_timezone" };

/**
 * Whether `name` is exactly a time zone name Postgres knows.
 *
 * The fiscal zone is consumed by Postgres (`AT TIME ZONE` in the ledger
 * reports), so Postgres is the authority: `Intl.supportedValuesOf("timeZone")`
 * differs from the server's tzdata in both directions (e.g. it lists aliases
 * the server may not ship and omits `Etc/UTC`). Validating against
 * `pg_timezone_names` keeps "accepted here" equal to "honoured by the ledger".
 */
async function isKnownTimeZone(name: string): Promise<boolean> {
	const rows = await db.execute<{ known: boolean }>(
		sql`SELECT EXISTS (SELECT 1 FROM pg_catalog.pg_timezone_names WHERE name = ${name}) AS known`,
	);
	return rows[0]?.known === true;
}

/**
 * Update a company's settings.
 *
 * @param input - Settings to change; `timezone` is trimmed and must be an IANA
 *   name known to Postgres.
 * @returns `invalid_timezone` without writing when the zone is unknown,
 *   `not_found` when the company does not exist, else the stored settings.
 * @example
 * ```ts
 * const result = await updateCompanySettings({ companyId, timezone: "America/Lima" });
 * ```
 */
export async function updateCompanySettings(
	input: UpdateCompanySettingsInput,
): Promise<UpdateCompanySettingsResult> {
	const timezone = input.timezone?.trim();
	if (timezone !== undefined && !(await isKnownTimeZone(timezone))) {
		return { status: "invalid_timezone" };
	}

	const [updated] = await db
		.update(companies)
		.set({
			settingsLanguage: input.language,
			settingsTimezone: timezone,
			settingsCurrency: input.currency,
			businessName: input.companyName,
			ruc: input.companyRuc,
			settingsAutoClosePeriod: input.autoClosePeriod,
			settingsShowAmountsInWords: input.showAmountsInWords,
			updatedAt: new Date(),
		})
		.where(eq(companies.id, input.companyId))
		.returning();

	if (!updated) return { status: "not_found" };
	return { status: "updated", settings: serializeCompanySettings(updated) };
}
