import { createHmac, timingSafeEqual } from "node:crypto";
import {
	type HeaderContainer,
	resolveSessionIdentityFromHeaders,
} from "../auth/handlers/session-identity";
import {
	logReadOnlyLedgerKeyRefusal,
	READ_ONLY_LEDGER_KEY_ROLE,
	type ReadOnlyLedgerKeyRefusal,
	readPresentedLedgerKey,
	verifyReadOnlyLedgerKey,
} from "./read-only-ledger-key";
import { readHeaderValue, resolveTenantAssertion } from "./tenant-assertions";

export const AUTHENTICATED_CALLER_KIND = {
	SESSION: "session",
	MACHINE: "machine",
	HEADER_FALLBACK: "header-fallback",
	API_KEY: "api-key",
} as const;

export type AuthenticatedCallerKind =
	(typeof AUTHENTICATED_CALLER_KIND)[keyof typeof AUTHENTICATED_CALLER_KIND];

export interface AuthenticatedCaller {
	kind: AuthenticatedCallerKind;
	userId: string;
	authUserId: string;
	legacyUserId: string | null;
	role: string;
	companyId: string | null;
	sessionId: string | null;
	serviceId: string | null;
	/**
	 * `true` only for read-only ledger key callers (`kind: "api-key"`). Such a
	 * caller may read, never write. Absent for every other caller kind.
	 */
	readOnly?: true;
	/** Id of the read-only ledger key; present only when `kind` is `"api-key"`. */
	apiKeyId?: string;
}

type CallerRefusal<Status extends number> = {
	ok: false;
	status: Status;
	code: string;
	error: string;
	/**
	 * Seconds for `Retry-After`; only on a rate-limited key or when key
	 * verification is unavailable.
	 */
	retryAfterSeconds?: number;
};

/** Result for callers that do not opt in to read-only ledger keys. */
export type AuthenticatedCallerResult =
	| {
			ok: true;
			caller: AuthenticatedCaller;
	  }
	| CallerRefusal<401 | 403>;

/**
 * Result when the route opts in to read-only ledger keys: besides 401/403, a
 * key may be refused with 429 (rate limited) or 503 (verification
 * unavailable). Only the key branch produces those two statuses.
 */
export type ReadOnlyLedgerKeyCallerResult =
	| AuthenticatedCallerResult
	| CallerRefusal<429 | 503>;

export interface ResolveAuthenticatedCallerInput {
	headers: HeaderContainer;
	requestedCompanyId?: string;
	requireSession?: boolean;
	allowHeaderFallback?: boolean;
	allowMachineCaller?: boolean;
	machineCallerAllowlist?: readonly string[];
	requireTenant?: boolean;
	requireRole?: boolean;
	/**
	 * Route opt-in for read-only ledger keys (`drn_ro_…`). Only when set, and
	 * only when no session resolved, a presented key is verified; the request
	 * is then answered by the key alone (never by header identity). Off by
	 * default: other routes ignore keys.
	 */
	allowReadOnlyLedgerKey?: boolean | undefined;
	/** HTTP method of the request; read-only keys require GET or HEAD. */
	method?: string | undefined;
	/**
	 * Route of the request (the registered path, e.g. `/api/ledger/accounts`),
	 * used only in the read-only key refusal log.
	 */
	route?: string | undefined;
	/**
	 * Company ids the request asks for (e.g. `?companyId=`), checked only for
	 * a read-only ledger key caller: any value other than the key's company is
	 * refused with 403 `API_KEY_TENANT_MISMATCH`. Sessions, machine callers and
	 * header fallback never read it, so their behavior does not change.
	 */
	readOnlyLedgerKeyRequestedCompanyIds?: readonly unknown[] | undefined;
}

interface SignedMachineCaller {
	serviceId: string;
	role: string;
	companyId: string | null;
	authUserId: string;
}

export interface TrustedMachineCallerAllowlistInput {
	allowlist?: readonly string[];
	envVarName?: string;
}

function normalizeRole(value: string): string {
	return value.trim().toLowerCase();
}

function normalizeMachineIdentity(value: string): string {
	return value.trim().toLowerCase();
}

function parseMachineCallerAllowlist(raw: string): string[] {
	return raw
		.split(",")
		.map((serviceId) => normalizeMachineIdentity(serviceId))
		.filter((serviceId) => serviceId.length > 0);
}

export function resolveTrustedMachineCallerAllowlist(
	input?: TrustedMachineCallerAllowlistInput,
): readonly string[] {
	const fromInput = (input?.allowlist ?? [])
		.map((serviceId) => normalizeMachineIdentity(serviceId))
		.filter((serviceId) => serviceId.length > 0);
	if (fromInput.length > 0) return fromInput;

	const envVarName =
		input?.envVarName?.trim() || "ARKELYTHEX_MACHINE_CALLER_ALLOWLIST";
	return parseMachineCallerAllowlist(process.env[envVarName] ?? "");
}

function resolveMachineCallerAllowlist(
	inputAllowlist?: readonly string[],
): ReadonlySet<string> {
	return new Set(
		resolveTrustedMachineCallerAllowlist({
			allowlist: inputAllowlist,
			envVarName: "ARKELYTHEX_MACHINE_CALLER_ALLOWLIST",
		}),
	);
}

function shouldRequireSession(requireSessionOverride?: boolean): boolean {
	if (typeof requireSessionOverride === "boolean")
		return requireSessionOverride;

	const env = (process.env.NODE_ENV ?? "").toLowerCase();
	if (env !== "test") return true;

	const enforceInTests = (
		process.env.SECURITY_ENFORCE_TEST_SESSION ?? ""
	).toLowerCase();
	return enforceInTests === "1" || enforceInTests === "true";
}

function validateTenantScope(
	callerCompanyId: string | null,
	assertedCompanyId: string | null,
	requestedCompanyId: string | undefined,
	requireTenant: boolean,
): AuthenticatedCallerResult | null {
	if (
		assertedCompanyId &&
		callerCompanyId &&
		assertedCompanyId !== callerCompanyId
	) {
		return {
			ok: false,
			status: 403,
			code: "AUTH_CONTEXT_MISMATCH",
			error: "Authenticated tenant does not match the tenant assertion headers",
		};
	}

	if (
		requestedCompanyId &&
		callerCompanyId &&
		requestedCompanyId !== callerCompanyId
	) {
		return {
			ok: false,
			status: 403,
			code: "TENANT_SCOPE_VIOLATION",
			error: "Requested companyId does not match caller tenant scope",
		};
	}

	if ((requestedCompanyId || assertedCompanyId) && !callerCompanyId) {
		return {
			ok: false,
			status: 403,
			code: "TENANT_SCOPE_VIOLATION",
			error: "Authenticated caller does not have a tenant scope",
		};
	}

	if (requireTenant && !callerCompanyId) {
		return {
			ok: false,
			status: 401,
			code: "TENANT_REQUIRED",
			error: "Active tenant context is required for this operation",
		};
	}

	return null;
}

function validateRoleRequirement(
	role: string,
	requireRole: boolean,
): AuthenticatedCallerResult | null {
	if (requireRole && role.length === 0) {
		return {
			ok: false,
			status: 401,
			code: "AUTH_REQUIRED",
			error: "Missing auth context headers: x-user-role",
		};
	}

	return null;
}

function resolveMachineSignaturePayload(
	headers: HeaderContainer,
): SignedMachineCaller | null {
	const serviceId = readHeaderValue(headers, "x-ark-service-id");
	const role = normalizeRole(
		readHeaderValue(headers, "x-ark-service-role") || "service",
	);
	const companyId =
		readHeaderValue(headers, "x-ark-service-company-id") || null;

	if (!serviceId) return null;

	return {
		serviceId,
		role,
		companyId,
		authUserId: `service:${serviceId}`,
	};
}

function resolveSignedMachineCaller(
	headers: HeaderContainer,
	machineCallerAllowlist: ReadonlySet<string>,
): AuthenticatedCallerResult | null {
	const machineCaller = resolveMachineSignaturePayload(headers);
	if (!machineCaller) return null;

	if (machineCallerAllowlist.size === 0) {
		return {
			ok: false,
			status: 401,
			code: "MACHINE_CALLER_DISABLED",
			error:
				"Signed machine callers require an explicit allowlist for this route",
		};
	}

	if (
		!machineCallerAllowlist.has(
			normalizeMachineIdentity(machineCaller.serviceId),
		)
	) {
		return {
			ok: false,
			status: 403,
			code: "MACHINE_CALLER_FORBIDDEN",
			error: "Machine caller is not allowlisted for this route",
		};
	}

	const secret = (process.env.ARKELYTHEX_MACHINE_CALLER_SECRET ?? "").trim();
	if (!secret) {
		return {
			ok: false,
			status: 401,
			code: "MACHINE_CALLER_DISABLED",
			error: "Signed machine callers are not enabled for this environment",
		};
	}

	const timestamp = readHeaderValue(headers, "x-ark-service-timestamp");
	const signature = readHeaderValue(headers, "x-ark-service-signature");
	if (!timestamp || !signature) {
		return {
			ok: false,
			status: 401,
			code: "MACHINE_AUTH_REQUIRED",
			error: "Signed machine caller headers are incomplete",
		};
	}

	const timestampMs = Number.parseInt(timestamp, 10);
	if (!Number.isFinite(timestampMs)) {
		return {
			ok: false,
			status: 401,
			code: "MACHINE_AUTH_INVALID",
			error: "Machine caller timestamp is invalid",
		};
	}

	const maxSkewMs = 5 * 60 * 1000;
	if (Math.abs(Date.now() - timestampMs) > maxSkewMs) {
		return {
			ok: false,
			status: 401,
			code: "MACHINE_AUTH_EXPIRED",
			error: "Machine caller signature has expired",
		};
	}

	const payload = [
		machineCaller.serviceId,
		timestamp,
		machineCaller.companyId ?? "",
		machineCaller.role,
	].join(".");
	const expected = createHmac("sha256", secret).update(payload).digest("hex");

	const provided = signature.startsWith("sha256=")
		? signature.slice("sha256=".length)
		: signature;

	const expectedBuffer = Buffer.from(expected, "utf8");
	const providedBuffer = Buffer.from(provided, "utf8");
	if (
		expectedBuffer.length !== providedBuffer.length ||
		!timingSafeEqual(expectedBuffer, providedBuffer)
	) {
		return {
			ok: false,
			status: 401,
			code: "MACHINE_AUTH_INVALID",
			error: "Machine caller signature is invalid",
		};
	}

	return {
		ok: true,
		caller: {
			kind: AUTHENTICATED_CALLER_KIND.MACHINE,
			userId: machineCaller.authUserId,
			authUserId: machineCaller.authUserId,
			legacyUserId: null,
			role: machineCaller.role,
			companyId: machineCaller.companyId,
			sessionId: null,
			serviceId: machineCaller.serviceId,
		},
	};
}

function hasSpoofableHeaderContext(headers: HeaderContainer): boolean {
	const candidateHeaders = [
		"x-auth-user-id",
		"x-user-id",
		"x-user-role",
		"x-company-id",
		"x-active-company-id",
	];

	return candidateHeaders.some(
		(headerName) => readHeaderValue(headers, headerName).length > 0,
	);
}

function resolveHeaderFallbackCaller(
	headers: HeaderContainer,
	companyId: string | null,
	requireRole: boolean,
): AuthenticatedCallerResult {
	const authUserId = readHeaderValue(headers, "x-auth-user-id");
	const legacyUserId = readHeaderValue(headers, "x-user-id") || null;
	const role = normalizeRole(readHeaderValue(headers, "x-user-role"));
	const userId = authUserId || legacyUserId || "";

	if (!userId || (requireRole && role.length === 0)) {
		return {
			ok: false,
			status: 401,
			code: "AUTH_REQUIRED",
			error:
				"Missing auth context headers: x-user-id, x-auth-user-id, x-user-role, x-company-id or x-active-company-id",
		};
	}

	return {
		ok: true,
		caller: {
			kind: AUTHENTICATED_CALLER_KIND.HEADER_FALLBACK,
			userId,
			authUserId: authUserId || userId,
			legacyUserId,
			role,
			companyId,
			sessionId: null,
			serviceId: null,
		},
	};
}

function apiKeyTenantMismatch(apiKeyId?: string): ReadOnlyLedgerKeyRefusal {
	return {
		ok: false,
		status: 403,
		code: "API_KEY_TENANT_MISMATCH",
		error: "API key company does not match the requested company",
		...(apiKeyId === undefined ? {} : { apiKeyId }),
	};
}

/**
 * Logs a refused key request once and returns the refusal as the caller
 * result, without the log-only fields (key id, error name and code).
 */
function refuseReadOnlyLedgerKey(
	input: ResolveAuthenticatedCallerInput,
	refusal: ReadOnlyLedgerKeyRefusal,
): ReadOnlyLedgerKeyCallerResult {
	logReadOnlyLedgerKeyRefusal({
		refusal,
		route: input.route,
		method: input.method,
	});
	return {
		ok: false,
		status: refusal.status,
		code: refusal.code,
		error: refusal.error,
		...(refusal.retryAfterSeconds === undefined
			? {}
			: { retryAfterSeconds: refusal.retryAfterSeconds }),
	};
}

/**
 * Resolves a read-only ledger key caller. The key alone decides identity and
 * tenant: identity headers are ignored, and a tenant assertion
 * (`x-company-id` / `x-active-company-id`) or requested company (argument or
 * `?companyId=`) must equal the key's company.
 */
async function resolveReadOnlyLedgerKeyCaller(input: {
	key: string;
	method: string | undefined;
	assertedCompanyId: string | null;
	requestedCompanyId: string | undefined;
	requestedCompanyIds: readonly unknown[];
}): Promise<ReadOnlyLedgerKeyCallerResult | ReadOnlyLedgerKeyRefusal> {
	const verified = await verifyReadOnlyLedgerKey({
		key: input.key,
		method: input.method,
	});
	if (!verified.ok) return verified;

	const foreignAssertion =
		input.assertedCompanyId !== null &&
		input.assertedCompanyId !== verified.companyId;
	const foreignRequest =
		(input.requestedCompanyId !== undefined &&
			input.requestedCompanyId !== verified.companyId) ||
		input.requestedCompanyIds.some(
			(requested) => requested !== verified.companyId,
		);
	if (foreignAssertion || foreignRequest) {
		return apiKeyTenantMismatch(verified.apiKeyId);
	}

	return {
		ok: true,
		caller: {
			kind: AUTHENTICATED_CALLER_KIND.API_KEY,
			userId: verified.ownerId,
			authUserId: verified.ownerId,
			legacyUserId: null,
			role: READ_ONLY_LEDGER_KEY_ROLE,
			companyId: verified.companyId,
			sessionId: null,
			serviceId: null,
			readOnly: true,
			apiKeyId: verified.apiKeyId,
		},
	};
}

/**
 * Returns the key decision when the route opts in and a key is presented,
 * or `null` so resolution continues (machine caller, header fallback).
 */
async function resolveOptedInReadOnlyLedgerKey(
	input: ResolveAuthenticatedCallerInput,
	assertedCompanyId: string | null,
): Promise<ReadOnlyLedgerKeyCallerResult | null> {
	if (!input.allowReadOnlyLedgerKey) return null;
	const presentedKey = readPresentedLedgerKey(input.headers);
	if (presentedKey === null) return null;
	const result = await resolveReadOnlyLedgerKeyCaller({
		key: presentedKey,
		method: input.method,
		assertedCompanyId,
		requestedCompanyId: input.requestedCompanyId,
		requestedCompanyIds: input.readOnlyLedgerKeyRequestedCompanyIds ?? [],
	});
	if (result.ok) return result;
	return refuseReadOnlyLedgerKey(input, result);
}

/**
 * Identity headers (x-user-id, x-user-role, x-company-id) are whatever the
 * caller sends, so production never accepts them as proof of who the caller
 * is: only a Better Auth session or a signed machine caller counts there.
 */
function headerIdentityAllowed(allowHeaderFallback?: boolean): boolean {
	if (!allowHeaderFallback) return false;
	return (process.env.NODE_ENV ?? "").toLowerCase() !== "production";
}

/**
 * Refusal for conflicting `x-company-id` / `x-active-company-id`. The two
 * headers can never both be a key's company, so a key on an opted-in route is
 * refused as a tenant mismatch (fail closed, before the key is looked up)
 * instead of the generic conflict, which header-fallback routes would pass
 * through without context.
 */
function tenantConflictRefusal(
	input: ResolveAuthenticatedCallerInput,
	conflict: { status: 403; code: string; error: string },
): ReadOnlyLedgerKeyCallerResult {
	if (
		input.allowReadOnlyLedgerKey &&
		readPresentedLedgerKey(input.headers) !== null
	) {
		return refuseReadOnlyLedgerKey(input, apiKeyTenantMismatch());
	}
	return {
		ok: false,
		status: conflict.status,
		code: conflict.code,
		error: conflict.error,
	};
}

/**
 * Resolves the caller. Without `allowReadOnlyLedgerKey` the result is a
 * session, machine or header-fallback caller, or a 401/403 refusal; with it,
 * a read-only ledger key may also be refused with 429 or 503.
 */
export function resolveAuthenticatedCaller(
	input: ResolveAuthenticatedCallerInput & {
		allowReadOnlyLedgerKey?: false | undefined;
	},
): Promise<AuthenticatedCallerResult>;
export function resolveAuthenticatedCaller(
	input: ResolveAuthenticatedCallerInput,
): Promise<ReadOnlyLedgerKeyCallerResult>;
export async function resolveAuthenticatedCaller(
	input: ResolveAuthenticatedCallerInput,
): Promise<ReadOnlyLedgerKeyCallerResult> {
	const requireSession = shouldRequireSession(input.requireSession);
	const requireTenant = input.requireTenant ?? true;
	const requireRole = input.requireRole ?? true;
	const tenantAssertion = resolveTenantAssertion(input.headers);
	if (tenantAssertion.ok === false) {
		return tenantConflictRefusal(input, tenantAssertion);
	}

	const assertedCompanyId = tenantAssertion.assertion.companyId;
	const assertedAuthUserId = readHeaderValue(input.headers, "x-auth-user-id");
	const assertedLegacyUserId =
		readHeaderValue(input.headers, "x-user-id") || null;
	const assertedRole = normalizeRole(
		readHeaderValue(input.headers, "x-user-role"),
	);

	const sessionIdentity = await resolveSessionIdentityFromHeaders(
		input.headers,
	);
	if (sessionIdentity.authUserId) {
		if (
			assertedAuthUserId &&
			sessionIdentity.authUserId !== assertedAuthUserId
		) {
			return {
				ok: false,
				status: 403,
				code: "AUTH_CONTEXT_MISMATCH",
				error: "Session user does not match x-auth-user-id header",
			};
		}

		if (
			assertedLegacyUserId &&
			sessionIdentity.legacyUserId &&
			sessionIdentity.legacyUserId !== assertedLegacyUserId
		) {
			return {
				ok: false,
				status: 403,
				code: "AUTH_CONTEXT_MISMATCH",
				error: "Session legacy user does not match x-user-id header",
			};
		}

		const effectiveRole = normalizeRole(sessionIdentity.role ?? assertedRole);
		if (assertedRole && effectiveRole && assertedRole !== effectiveRole) {
			return {
				ok: false,
				status: 403,
				code: "AUTH_CONTEXT_MISMATCH",
				error: "Session role does not match x-user-role header",
			};
		}

		const tenantValidation = validateTenantScope(
			sessionIdentity.activeCompanyId || sessionIdentity.companyId,
			assertedCompanyId,
			input.requestedCompanyId,
			requireTenant,
		);
		if (tenantValidation) {
			return tenantValidation;
		}

		const roleValidation = validateRoleRequirement(effectiveRole, requireRole);
		if (roleValidation) {
			return roleValidation;
		}

		return {
			ok: true,
			caller: {
				kind: AUTHENTICATED_CALLER_KIND.SESSION,
				userId: sessionIdentity.authUserId,
				authUserId: sessionIdentity.authUserId,
				legacyUserId: sessionIdentity.legacyUserId,
				role: effectiveRole,
				companyId: sessionIdentity.activeCompanyId || sessionIdentity.companyId,
				sessionId: sessionIdentity.sessionId,
				serviceId: null,
			},
		};
	}

	const readOnlyKeyCaller = await resolveOptedInReadOnlyLedgerKey(
		input,
		assertedCompanyId,
	);
	if (readOnlyKeyCaller) return readOnlyKeyCaller;

	if (input.allowMachineCaller) {
		const machineCallerAllowlist = resolveMachineCallerAllowlist(
			input.machineCallerAllowlist,
		);
		const machineCaller = resolveSignedMachineCaller(
			input.headers,
			machineCallerAllowlist,
		);
		if (machineCaller) {
			if (!machineCaller.ok) return machineCaller;

			const roleValidation = validateRoleRequirement(
				machineCaller.caller.role,
				requireRole,
			);
			if (roleValidation) {
				return roleValidation;
			}

			const tenantValidation = validateTenantScope(
				machineCaller.caller.companyId,
				assertedCompanyId,
				input.requestedCompanyId,
				requireTenant,
			);
			if (tenantValidation) {
				return tenantValidation;
			}

			return machineCaller;
		}
	}

	if (requireSession) {
		return {
			ok: false,
			status: 401,
			code: "SESSION_REQUIRED",
			error: "Active BetterAuth session is required for this operation",
		};
	}

	if (!headerIdentityAllowed(input.allowHeaderFallback)) {
		if (hasSpoofableHeaderContext(input.headers)) {
			return {
				ok: false,
				status: 403,
				code: "SPOOFABLE_HEADER_CONTEXT",
				error:
					"Header-only caller context is not allowed for this route. Use a session or a signed machine caller.",
			};
		}

		return {
			ok: false,
			status: 401,
			code: "AUTH_REQUIRED",
			error: "No active session. Sign in and retry.",
		};
	}

	const headerFallback = resolveHeaderFallbackCaller(
		input.headers,
		assertedCompanyId,
		requireRole,
	);
	if (!headerFallback.ok) return headerFallback;

	const tenantValidation = validateTenantScope(
		headerFallback.caller.companyId,
		assertedCompanyId,
		input.requestedCompanyId,
		requireTenant,
	);
	if (tenantValidation) {
		return tenantValidation;
	}

	return headerFallback;
}
