import { Elysia } from "elysia";
import { createLogger } from "../../lib/logger";
import {
	buildForwardedAuthHeaders,
	type HeaderContainer,
	resolveSessionIdentityFromHeaders,
} from "../auth/handlers/session-identity";
import { readPresentedLedgerKey } from "../security/read-only-ledger-key";
import { fail, ok } from "../shared/api-response";
import {
	firstIssueMessage,
	issueLedgerKeyBodySchema,
	LEDGER_KEY_DEFAULT_EXPIRES_IN_DAYS,
	ledgerKeyIdSchema,
	listLedgerKeysQuerySchema,
} from "./ledger-keys.schemas";
import {
	issueLedgerKey,
	type LedgerKeyFailure,
	listLedgerKeys,
	revokeLedgerKey,
} from "./ledger-keys.service";

/**
 * Ledger key management routes (`/api/ledger-keys`).
 *
 * - `POST /` issues a read-only ledger key for one company (returned once).
 * - `GET /?companyId=` lists the caller's own keys for that company.
 * - `DELETE /:id` revokes one of the caller's own keys.
 *
 * Session only: a presented API key (`x-api-key` or
 * `Authorization: Bearer drn_ro_…`) is refused before anything else, and
 * header identity (`x-user-id`, …) is never accepted, in any environment.
 * This module does not use `companyScopeGuard`: the company comes from the
 * request and is checked against the session user's active memberships.
 *
 * @module ledger-keys/routes
 */

const logger = createLogger({ feature: "ledger-keys", handler: "routes" });

type SessionCaller =
	| { ok: true; userId: string; sessionHeaders: Headers }
	| { ok: false; status: 401 | 403; code: string; error: string };

async function resolveSessionCaller(
	headers: HeaderContainer,
): Promise<SessionCaller> {
	if (readPresentedLedgerKey(headers) !== null) {
		return {
			ok: false,
			status: 403,
			code: "API_KEY_NOT_ALLOWED",
			error: "API keys cannot manage ledger keys; sign in with a session",
		};
	}

	const identity = await resolveSessionIdentityFromHeaders(headers);
	if (!identity.authUserId || !identity.sessionId) {
		return {
			ok: false,
			status: 401,
			code: "SESSION_REQUIRED",
			error: "A signed-in session is required to manage ledger keys",
		};
	}

	return {
		ok: true,
		userId: identity.authUserId,
		sessionHeaders: new Headers(buildForwardedAuthHeaders(headers)),
	};
}

function unexpectedError(error: unknown, operation: string) {
	// Log the failure kind only: never request bodies, keys or hashes.
	logger.error(
		{
			operation,
			errorName: error instanceof Error ? error.name : typeof error,
		},
		"Ledger key operation failed",
	);
	return fail(
		"Could not complete the ledger key operation",
		"LEDGER_KEY_ERROR",
	);
}

function refusal(
	set: { status?: number | string },
	failure: LedgerKeyFailure | Extract<SessionCaller, { ok: false }>,
) {
	set.status = failure.status;
	return fail(failure.error, failure.code);
}

export const ledgerKeysRoutes = new Elysia({ prefix: "/api/ledger-keys" })
	.post(
		"/",
		async ({ headers, body, set }) => {
			const caller = await resolveSessionCaller(headers);
			if (!caller.ok) return refusal(set, caller);

			const parsed = issueLedgerKeyBodySchema.safeParse(body);
			if (!parsed.success) {
				set.status = 400;
				return fail(firstIssueMessage(parsed.error), "VALIDATION_ERROR");
			}

			try {
				const result = await issueLedgerKey({
					userId: caller.userId,
					companyId: parsed.data.companyId,
					name: parsed.data.name,
					expiresInDays:
						parsed.data.expiresInDays ?? LEDGER_KEY_DEFAULT_EXPIRES_IN_DAYS,
					sessionHeaders: caller.sessionHeaders,
				});
				if (!result.ok) return refusal(set, result);
				set.status = 201;
				return ok(result.key);
			} catch (error: unknown) {
				set.status = 500;
				return unexpectedError(error, "issue");
			}
		},
		{
			detail: {
				tags: ["Ledger Keys"],
				summary: "Issue a read-only ledger key (shown once)",
			},
		},
	)
	.get(
		"/",
		async ({ headers, query, set }) => {
			const caller = await resolveSessionCaller(headers);
			if (!caller.ok) return refusal(set, caller);

			const parsed = listLedgerKeysQuerySchema.safeParse(query);
			if (!parsed.success) {
				set.status = 400;
				return fail(firstIssueMessage(parsed.error), "VALIDATION_ERROR");
			}

			try {
				const result = await listLedgerKeys({
					userId: caller.userId,
					companyId: parsed.data.companyId,
					sessionHeaders: caller.sessionHeaders,
				});
				if (!result.ok) return refusal(set, result);
				return ok(result.keys);
			} catch (error: unknown) {
				set.status = 500;
				return unexpectedError(error, "list");
			}
		},
		{
			detail: {
				tags: ["Ledger Keys"],
				summary: "List your read-only ledger keys for a company",
			},
		},
	)
	.delete(
		"/:id",
		async ({ headers, params, set }) => {
			const caller = await resolveSessionCaller(headers);
			if (!caller.ok) return refusal(set, caller);

			const keyId = ledgerKeyIdSchema.safeParse(params.id);
			if (!keyId.success) {
				set.status = 404;
				return fail("Ledger key not found", "LEDGER_KEY_NOT_FOUND");
			}

			try {
				const result = await revokeLedgerKey({
					userId: caller.userId,
					keyId: keyId.data,
					sessionHeaders: caller.sessionHeaders,
				});
				if (!result.ok) return refusal(set, result);
				return ok({ id: result.id, revoked: true });
			} catch (error: unknown) {
				set.status = 500;
				return unexpectedError(error, "revoke");
			}
		},
		{
			detail: {
				tags: ["Ledger Keys"],
				summary: "Revoke one of your read-only ledger keys",
			},
		},
	);
