import { createLogger } from "../../lib/logger";
import { auth } from "../auth/auth.config";
import { listUserCompanyMemberships } from "../auth/handlers/company-membership";
import type { HeaderContainer } from "../auth/handlers/session-identity";
import {
	READ_ONLY_LEDGER_KEY_ACTION,
	READ_ONLY_LEDGER_KEY_PREFIX,
	READ_ONLY_LEDGER_KEY_RATE_LIMIT,
	READ_ONLY_LEDGER_KEY_RESOURCE,
	readOnlyLedgerKeyPermissions,
} from "../auth/lib/read-only-ledger-key";
import { readHeaderValue } from "./tenant-assertions";

/**
 * Read-only ledger keys — verification for the caller resolver.
 *
 * A key is accepted only when every check holds, in this order:
 * 1. The request method is GET or HEAD (checked before the key is verified,
 *    so a write never even reaches the key store).
 * 2. The key has the `drn_ro_` prefix.
 * 3. Better Auth `verifyApiKey` accepts it with `ledger:read` (hashed lookup,
 *    enabled, not expired, rate limit).
 * 4. The verified key itself carries `ledger:read` (defense in depth).
 * 5. Its metadata binds it to exactly one company (`metadata.companyId`).
 * 6. Its owner is still an active member of that company right now.
 *
 * Anything else is refused with a code starting with `API_KEY_` and no data.
 * Routes opt in through `allowReadOnlyLedgerKey`; see
 * `authenticated-caller.ts`.
 *
 * @module security/read-only-ledger-key
 */

const logger = createLogger({
	feature: "security",
	handler: "read-only-ledger-key",
});

/** Role given to key callers: least privilege, never the owner's role. */
export const READ_ONLY_LEDGER_KEY_ROLE = "viewer";

/** Every refusal from this module uses a code with this prefix. */
export const READ_ONLY_LEDGER_KEY_ERROR_PREFIX = "API_KEY_";

/** Refusal code for a key that exceeded its rate limit (HTTP 429). */
export const READ_ONLY_LEDGER_KEY_RATE_LIMITED = "API_KEY_RATE_LIMITED";

const READ_METHODS: ReadonlySet<string> = new Set(["GET", "HEAD"]);
const RATE_LIMIT_PLUGIN_CODES: ReadonlySet<string> = new Set([
	"RATE_LIMITED",
	"USAGE_EXCEEDED",
]);

export type ReadOnlyLedgerKeyRefusal = {
	ok: false;
	status: 401 | 403;
	code: string;
	error: string;
	/**
	 * Seconds until the key may retry; present only on
	 * `API_KEY_RATE_LIMITED` and sent as the `Retry-After` header.
	 */
	retryAfterSeconds?: number;
};

export type ReadOnlyLedgerKeyResult =
	| {
			ok: true;
			ownerId: string;
			companyId: string;
			apiKeyId: string;
	  }
	| ReadOnlyLedgerKeyRefusal;

/**
 * Returns `true` when a refusal came from the read-only key path. Callers
 * that otherwise degrade open (header-fallback routes) must fail closed for
 * these codes.
 */
export function isReadOnlyLedgerKeyRefusal(code: string): boolean {
	return code.startsWith(READ_ONLY_LEDGER_KEY_ERROR_PREFIX);
}

/**
 * Reads a presented key: `x-api-key` (any value), or
 * `Authorization: Bearer drn_ro_…`. Other bearer tokens are not keys.
 */
export function readPresentedLedgerKey(
	headers: HeaderContainer,
): string | null {
	const apiKeyHeader = readHeaderValue(headers, "x-api-key");
	if (apiKeyHeader) return apiKeyHeader;

	const authorization = readHeaderValue(headers, "authorization");
	const match = /^Bearer\s+(\S+)$/i.exec(authorization);
	const token = match?.[1];
	if (token?.startsWith(READ_ONLY_LEDGER_KEY_PREFIX)) return token;

	return null;
}

function refuse(
	status: 401 | 403,
	code: string,
	error: string,
): ReadOnlyLedgerKeyRefusal {
	return { ok: false, status, code, error };
}

const INVALID_KEY = refuse(
	401,
	"API_KEY_INVALID",
	"API key is invalid, expired or revoked",
);

/**
 * Whole seconds (at least 1) until a rate-limited key may retry: the plugin's
 * `details.tryAgainIn` (milliseconds) when present, else the key's window.
 */
function retryAfterSecondsOf(details: unknown): number {
	const tryAgainIn =
		details && typeof details === "object"
			? (details as { tryAgainIn?: unknown }).tryAgainIn
			: undefined;
	const milliseconds =
		typeof tryAgainIn === "number" && Number.isFinite(tryAgainIn)
			? tryAgainIn
			: READ_ONLY_LEDGER_KEY_RATE_LIMIT.timeWindowMs;
	return Math.max(1, Math.ceil(milliseconds / 1000));
}

function readCompanyId(metadata: unknown): string | null {
	if (!metadata || typeof metadata !== "object") return null;
	const companyId = (metadata as { companyId?: unknown }).companyId;
	if (typeof companyId !== "string") return null;
	const trimmed = companyId.trim();
	return trimmed.length > 0 ? trimmed : null;
}

function grantsLedgerRead(permissions: unknown): boolean {
	if (!permissions || typeof permissions !== "object") return false;
	const actions = (permissions as Record<string, unknown>)[
		READ_ONLY_LEDGER_KEY_RESOURCE
	];
	return (
		Array.isArray(actions) && actions.includes(READ_ONLY_LEDGER_KEY_ACTION)
	);
}

/**
 * Verifies a presented read-only ledger key for one request.
 *
 * @param input.key - The presented key.
 * @param input.method - HTTP method of the request; missing means refused.
 */
export async function verifyReadOnlyLedgerKey(input: {
	key: string;
	method?: string | undefined;
}): Promise<ReadOnlyLedgerKeyResult> {
	const method = (input.method ?? "").toUpperCase();
	if (!READ_METHODS.has(method)) {
		return refuse(
			403,
			"API_KEY_READ_ONLY",
			"Read-only API keys cannot perform write operations",
		);
	}

	if (!input.key.startsWith(READ_ONLY_LEDGER_KEY_PREFIX)) {
		return INVALID_KEY;
	}

	try {
		const verification = await auth.api.verifyApiKey({
			body: { key: input.key, permissions: readOnlyLedgerKeyPermissions() },
		});

		if (!verification.valid || !verification.key) {
			const pluginCode = verification.error?.code ?? "";
			if (RATE_LIMIT_PLUGIN_CODES.has(pluginCode)) {
				return {
					...refuse(
						403,
						READ_ONLY_LEDGER_KEY_RATE_LIMITED,
						"API key rate limit exceeded; retry later",
					),
					retryAfterSeconds: retryAfterSecondsOf(
						(verification.error as { details?: unknown } | null)?.details,
					),
				};
			}
			return INVALID_KEY;
		}

		const key = verification.key;
		const ownerId = key.referenceId.trim();
		if (!ownerId) return INVALID_KEY;

		if (!grantsLedgerRead(key.permissions)) {
			return refuse(
				403,
				"API_KEY_FORBIDDEN",
				"API key does not grant ledger:read",
			);
		}

		const companyId = readCompanyId(key.metadata);
		if (!companyId) {
			return refuse(
				403,
				"API_KEY_NOT_COMPANY_BOUND",
				"API key is not bound to a company",
			);
		}

		const memberships = await listUserCompanyMemberships(ownerId);
		const isActiveMember = memberships.some(
			(membership) => membership.companyId === companyId,
		);
		if (!isActiveMember) {
			return refuse(
				403,
				"API_KEY_MEMBERSHIP_INACTIVE",
				"API key owner is not an active member of the key company",
			);
		}

		return { ok: true, ownerId, companyId, apiKeyId: key.id };
	} catch (error) {
		// Log the failure kind only: never the key, its hash or query values.
		logger.warn(
			{ errorName: error instanceof Error ? error.name : typeof error },
			"Read-only ledger key verification failed; refusing the request",
		);
		return INVALID_KEY;
	}
}
