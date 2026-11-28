import { z } from "zod";
import {
	READ_ONLY_LEDGER_KEY_MAX_EXPIRES_IN_DAYS,
	READ_ONLY_LEDGER_KEY_MAX_NAME_LENGTH,
} from "../auth/lib/read-only-ledger-key";

/**
 * Input schemas for the ledger key management routes.
 *
 * The issue body is strict: a client may choose only the company, a name and
 * a lifetime. Owner, prefix, permissions, metadata and limits are decided by
 * the server, so any other property is refused (400) instead of ignored.
 *
 * @module ledger-keys/schemas
 */

/** Longest key name, in UTF-16 units (the plugin measures `name.length`). */
export const LEDGER_KEY_NAME_MAX_LENGTH = READ_ONLY_LEDGER_KEY_MAX_NAME_LENGTH;

/** Lifetime used when the client does not ask for one, in days. */
export const LEDGER_KEY_DEFAULT_EXPIRES_IN_DAYS = 90;

/** Control, format and line/paragraph separator characters are not visible. */
const VISIBLE_TEXT = /^[^\p{Cc}\p{Cf}\p{Zl}\p{Zp}]+$/u;

/** Company ids are Postgres UUIDs; `guid` accepts every UUID shape. */
const companyIdSchema = z.guid({ message: "companyId must be a UUID" });

export const issueLedgerKeyBodySchema = z.strictObject({
	companyId: companyIdSchema,
	name: z
		.string()
		.trim()
		.min(1, { message: "name is required" })
		.max(LEDGER_KEY_NAME_MAX_LENGTH, {
			message: `name must be at most ${LEDGER_KEY_NAME_MAX_LENGTH} characters`,
		})
		.regex(VISIBLE_TEXT, {
			message: "name must contain visible characters only",
		}),
	expiresInDays: z
		.number()
		.int({ message: "expiresInDays must be a whole number of days" })
		.min(1, { message: "expiresInDays must be at least 1" })
		.max(READ_ONLY_LEDGER_KEY_MAX_EXPIRES_IN_DAYS, {
			message: `expiresInDays must be at most ${READ_ONLY_LEDGER_KEY_MAX_EXPIRES_IN_DAYS}`,
		})
		.optional(),
});

export type IssueLedgerKeyBody = z.infer<typeof issueLedgerKeyBodySchema>;

export const listLedgerKeysQuerySchema = z.object({
	companyId: companyIdSchema,
});

/** Key ids are opaque plugin ids; anything else is simply not found. */
export const ledgerKeyIdSchema = z.string().min(1).max(128);

/** First validation message, for the `fail()` envelope. */
export function firstIssueMessage(error: z.ZodError): string {
	return error.issues[0]?.message ?? "Invalid request";
}
