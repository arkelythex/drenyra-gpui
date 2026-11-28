import { Elysia } from "elysia";
import { resolveCorsOrigins } from "../../config/cors";
import { createLogger } from "../../lib/logger";
import {
	buildForwardedAuthHeaders,
	type HeaderContainer,
	resolveSessionIdentityFromHeaders,
} from "../auth/handlers/session-identity";
import { presentsAnyApiKey } from "../security/read-only-ledger-key";
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
 * Session only: a presented API key (any `x-api-key` value, or
 * `Authorization: Bearer drn_ro_…`) is refused before anything else, and
 * header identity (`x-user-id`, …) is never accepted, in any environment.
 * Any unexpected failure, the session lookup included, answers 500
 * `LEDGER_KEY_ERROR` (logged without secrets).
 * This module does not use `companyScopeGuard`: the company comes from the
 * request and is checked against the session user's active memberships.
 *
 * Browser rules for the cookie-authenticated writes (after the session check):
 * - `POST` and `DELETE` with an `Origin` header outside the API's CORS
 *   allowlist (`config/cors.ts`) → 403 `ORIGIN_NOT_ALLOWED`. Requests without
 *   `Origin` (CLI, server to server) are not affected.
 * - `POST` needs `Content-Type: application/json` → 415
 *   `UNSUPPORTED_MEDIA_TYPE` otherwise, so a cross-site form post (no CORS
 *   preflight) can never issue a key.
 *
 * Issue and list responses carry `Cache-Control: no-store`: the issue
 * response holds the only copy of the plaintext key.
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
	if (presentsAnyApiKey(headers)) {
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

const NO_STORE = "no-store";

function isJsonContentType(value: string | undefined): boolean {
	const mediaType = (value ?? "").split(";")[0]?.trim().toLowerCase();
	return mediaType === "application/json";
}

/** `scheme://host[:port]` of a URL, or `null` when it is not one. */
function serializedOrigin(value: string): string | null {
	try {
		const origin = new URL(value.trim()).origin;
		return origin === "null" ? null : origin;
	} catch {
		return null;
	}
}

/**
 * `true` when the request has no `Origin` header, or its `Origin` is exactly
 * an origin of the API's CORS allowlist (read per request, so it follows the
 * same configuration as the CORS plugin).
 */
function originAllowed(origin: string | undefined): boolean {
	if (origin === undefined) return true;
	const requested = serializedOrigin(origin);
	if (requested === null || requested !== origin.trim()) return false;
	return resolveCorsOrigins().some(
		(allowed) => serializedOrigin(allowed) === requested,
	);
}

type BrowserRuleRefusal = {
	ok: false;
	status: 403 | 415;
	code: string;
	error: string;
};

const ORIGIN_NOT_ALLOWED: BrowserRuleRefusal = {
	ok: false,
	status: 403,
	code: "ORIGIN_NOT_ALLOWED",
	error: "Origin is not allowed",
};

const UNSUPPORTED_MEDIA_TYPE: BrowserRuleRefusal = {
	ok: false,
	status: 415,
	code: "UNSUPPORTED_MEDIA_TYPE",
	error: "Content-Type must be application/json",
};

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
	failure:
		| LedgerKeyFailure
		| Extract<SessionCaller, { ok: false }>
		| BrowserRuleRefusal,
) {
	set.status = failure.status;
	return fail(failure.error, failure.code);
}

export const ledgerKeysRoutes = new Elysia({ prefix: "/api/ledger-keys" })
	.post(
		"/",
		async ({ headers, body, set }) => {
			set.headers["cache-control"] = NO_STORE;
			try {
				const caller = await resolveSessionCaller(headers);
				if (!caller.ok) return refusal(set, caller);
				if (!originAllowed(headers.origin)) {
					return refusal(set, ORIGIN_NOT_ALLOWED);
				}
				if (!isJsonContentType(headers["content-type"])) {
					return refusal(set, UNSUPPORTED_MEDIA_TYPE);
				}

				const parsed = issueLedgerKeyBodySchema.safeParse(body);
				if (!parsed.success) {
					set.status = 400;
					return fail(firstIssueMessage(parsed.error), "VALIDATION_ERROR");
				}

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
			set.headers["cache-control"] = NO_STORE;
			try {
				const caller = await resolveSessionCaller(headers);
				if (!caller.ok) return refusal(set, caller);

				const parsed = listLedgerKeysQuerySchema.safeParse(query);
				if (!parsed.success) {
					set.status = 400;
					return fail(firstIssueMessage(parsed.error), "VALIDATION_ERROR");
				}

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
			try {
				const caller = await resolveSessionCaller(headers);
				if (!caller.ok) return refusal(set, caller);
				if (!originAllowed(headers.origin)) {
					return refusal(set, ORIGIN_NOT_ALLOWED);
				}

				const keyId = ledgerKeyIdSchema.safeParse(params.id);
				if (!keyId.success) {
					set.status = 404;
					return fail("Ledger key not found", "LEDGER_KEY_NOT_FOUND");
				}

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
