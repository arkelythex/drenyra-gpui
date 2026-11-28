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
 * A thrown error while verifying the key or reading the membership is not a
 * verdict on the key: it answers 503 `API_KEY_VERIFICATION_UNAVAILABLE` with
 * `Retry-After`, so a client keeps a key that may still be valid.
 * Routes opt in through `allowReadOnlyLedgerKey`; see
 * `authenticated-caller.ts`, which also logs every refusal once
 * (`logReadOnlyLedgerKeyRefusal`).
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

/**
 * Refusal code when the key could not be verified because a dependency
 * failed (HTTP 503 with `Retry-After`); distinct from `API_KEY_INVALID`.
 */
export const READ_ONLY_LEDGER_KEY_VERIFICATION_UNAVAILABLE =
	"API_KEY_VERIFICATION_UNAVAILABLE";

/** `Retry-After` seconds sent with `API_KEY_VERIFICATION_UNAVAILABLE`. */
export const READ_ONLY_LEDGER_KEY_UNAVAILABLE_RETRY_AFTER_SECONDS = 5;

const READ_METHODS: ReadonlySet<string> = new Set(["GET", "HEAD"]);
const RATE_LIMIT_PLUGIN_CODES: ReadonlySet<string> = new Set([
	"RATE_LIMITED",
	"USAGE_EXCEEDED",
]);

export type ReadOnlyLedgerKeyRefusal = {
	ok: false;
	status: 401 | 403 | 429 | 503;
	code: string;
	error: string;
	/**
	 * Seconds until the key may retry; present only on
	 * `API_KEY_RATE_LIMITED` and `API_KEY_VERIFICATION_UNAVAILABLE`, and sent
	 * as the `Retry-After` header.
	 */
	retryAfterSeconds?: number;
	/** Id of the verified key, for the refusal log only (never sent). */
	apiKeyId?: string;
	/** Non-secret failure kind, for the refusal log only (never sent). */
	errorName?: string;
	/** Non-secret failure code (e.g. `ECONNREFUSED`), for the log only. */
	errorCode?: string;
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
 * Reads a presented read-only ledger key: `x-api-key: drn_ro_…` or
 * `Authorization: Bearer drn_ro_…`. Values without the `drn_ro_` prefix in
 * either header are not ledger keys and are ignored, as if absent.
 */
export function readPresentedLedgerKey(
	headers: HeaderContainer,
): string | null {
	const apiKeyHeader = readHeaderValue(headers, "x-api-key");
	if (apiKeyHeader.startsWith(READ_ONLY_LEDGER_KEY_PREFIX)) return apiKeyHeader;

	const authorization = readHeaderValue(headers, "authorization");
	const match = /^Bearer\s+(\S+)$/i.exec(authorization);
	const token = match?.[1];
	if (token?.startsWith(READ_ONLY_LEDGER_KEY_PREFIX)) return token;

	return null;
}

/**
 * `true` when the request presents any API key: a non-empty `x-api-key`
 * (whatever its value) or a `drn_ro_` bearer token. Used by the session-only
 * key management routes, which refuse every API key.
 */
export function presentsAnyApiKey(headers: HeaderContainer): boolean {
	return (
		readHeaderValue(headers, "x-api-key").length > 0 ||
		readPresentedLedgerKey(headers) !== null
	);
}

function refuse(
	status: 401 | 403 | 429 | 503,
	code: string,
	error: string,
): ReadOnlyLedgerKeyRefusal {
	return { ok: false, status, code, error };
}

const SAFE_ERROR_CODE = /^[A-Za-z0-9_.-]{1,64}$/;

function safeErrorCode(error: unknown): string | undefined {
	if (!error || typeof error !== "object") return undefined;
	const code = (error as { code?: unknown }).code;
	if (typeof code === "string" && SAFE_ERROR_CODE.test(code)) return code;
	return undefined;
}

/**
 * 503 refusal for a failed dependency. Carries only the error's name and a
 * short code (its own or its cause's), never its message: messages may hold
 * query text or values.
 */
function verificationUnavailable(error: unknown): ReadOnlyLedgerKeyRefusal {
	const errorCode =
		safeErrorCode(error) ??
		safeErrorCode(error instanceof Error ? error.cause : undefined);
	return {
		...refuse(
			503,
			READ_ONLY_LEDGER_KEY_VERIFICATION_UNAVAILABLE,
			"API key verification is temporarily unavailable; retry later",
		),
		retryAfterSeconds: READ_ONLY_LEDGER_KEY_UNAVAILABLE_RETRY_AFTER_SECONDS,
		errorName: error instanceof Error ? error.name : typeof error,
		...(errorCode === undefined ? {} : { errorCode }),
	};
}

/**
 * Logs one refused key request: refusal code, status, route, method, the key
 * id when it was verified, and for 503 the non-secret error name and code.
 * Never the key, its hash, headers or query values. Rate-limited refusals
 * log at `info` (expected under load), every other refusal at `warn`.
 */
export function logReadOnlyLedgerKeyRefusal(input: {
	refusal: ReadOnlyLedgerKeyRefusal;
	route: string | undefined;
	method: string | undefined;
}): void {
	const { refusal } = input;
	const entry = {
		event: "read_only_ledger_key_refused",
		code: refusal.code,
		status: refusal.status,
		...(refusal.apiKeyId === undefined ? {} : { apiKeyId: refusal.apiKeyId }),
		route: input.route ?? "",
		method: (input.method ?? "").toUpperCase(),
		...(refusal.errorName === undefined
			? {}
			: { errorName: refusal.errorName }),
		...(refusal.errorCode === undefined
			? {}
			: { errorCode: refusal.errorCode }),
	};
	if (refusal.code === READ_ONLY_LEDGER_KEY_RATE_LIMITED) {
		logger.info(entry, "Read-only ledger key refused");
		return;
	}
	logger.warn(entry, "Read-only ledger key refused");
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
						429,
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
		const ownerId =
			typeof key.referenceId === "string" ? key.referenceId.trim() : "";
		if (!ownerId) return INVALID_KEY;

		const apiKeyId = key.id;
		if (!grantsLedgerRead(key.permissions)) {
			return {
				...refuse(
					403,
					"API_KEY_FORBIDDEN",
					"API key does not grant ledger:read",
				),
				apiKeyId,
			};
		}

		const companyId = readCompanyId(key.metadata);
		if (!companyId) {
			return {
				...refuse(
					403,
					"API_KEY_NOT_COMPANY_BOUND",
					"API key is not bound to a company",
				),
				apiKeyId,
			};
		}

		const memberships = await listUserCompanyMemberships(ownerId);
		const isActiveMember = memberships.some(
			(membership) => membership.companyId === companyId,
		);
		if (!isActiveMember) {
			return {
				...refuse(
					403,
					"API_KEY_MEMBERSHIP_INACTIVE",
					"API key owner is not an active member of the key company",
				),
				apiKeyId,
			};
		}

		return { ok: true, ownerId, companyId, apiKeyId };
	} catch (error) {
		// A dependency failed (key store, membership): not a verdict on the
		// key. The caller logs the refusal with the error's name and code.
		return verificationUnavailable(error);
	}
}
