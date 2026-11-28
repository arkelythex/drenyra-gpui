import type { UnifiedActor, UnifiedRole } from "@drenyra/security/rbac";
import { resolveAuthenticatedCaller } from "./authenticated-caller";

export const SESSION_SECURITY_PROFILE = {
	DEFAULT: "default",
	SENSITIVE_WRITE: "sensitive-write",
} as const;

export type SessionSecurityProfile =
	(typeof SESSION_SECURITY_PROFILE)[keyof typeof SESSION_SECURITY_PROFILE];

/**
 * Tenant-scoped identity extracted from request headers and validated against Better Auth.
 *
 * @example
 * ```ts
 * const ctx: SessionContext = { userId: 'usr_1', role: 'admin', companyId: 'cmp_1' };
 * ```
 */
export interface SessionContext {
	userId: string;
	authUserId: string;
	legacyUserId: string | null;
	role: string;
	companyId: string;
	/**
	 * `true` only when the caller is a read-only ledger key: it may read, never
	 * write. Absent for sessions, machine callers and header fallback.
	 */
	readOnly?: true;
}

/**
 * Bridges the existing `SessionContext` to the unified `UnifiedActor` type
 * used by `@drenyra/security/rbac`. This allows incremental adoption of the
 * unified guard without breaking the 15+ call sites that depend on
 * `SessionContext`.
 *
 * @example
 * ```ts
 * const actor = toUnifiedActor(sessionContext);
 * requireBusinessPermission(actor, BusinessPermission.CompanyRead);
 * ```
 */
export function toUnifiedActor(ctx: SessionContext): UnifiedActor {
	return {
		userId: ctx.userId,
		authUserId: ctx.authUserId,
		legacyUserId: ctx.legacyUserId,
		role: ctx.role as UnifiedRole,
		companyId: ctx.companyId,
	};
}

/**
 * Result of resolving the caller session context for a protected API operation.
 *
 * @example
 * ```ts
 * const result: SessionContextResult = {
 *   ok: true,
 *   context: { userId: 'usr_1', role: 'admin', companyId: 'cmp_1' },
 * };
 * ```
 */
type SessionContextRefusal<Status extends number> = {
	ok: false;
	status: Status;
	code: string;
	error: string;
	/** Seconds for `Retry-After`; only on a rate-limited or unverifiable key. */
	retryAfterSeconds?: number;
};

export type SessionContextResult =
	| { ok: true; context: SessionContext }
	| SessionContextRefusal<401 | 403>;

/**
 * Result when the route opts in to read-only ledger keys: a key may also be
 * refused with 429 (rate limited) or 503 (verification unavailable).
 */
export type ReadOnlyLedgerKeySessionContextResult =
	| SessionContextResult
	| SessionContextRefusal<429 | 503>;

interface ResolveSessionContextInput {
	headers: Record<string, unknown>;
	requestedCompanyId?: string;
	requireSession?: boolean;
	allowHeaderFallback?: boolean;
	allowMachineCaller?: boolean;
	machineCallerAllowlist?: readonly string[];
	securityProfile?: SessionSecurityProfile;
	/** Route opt-in for read-only ledger keys; see `authenticated-caller.ts`. */
	allowReadOnlyLedgerKey?: boolean | undefined;
	/** HTTP method of the request (read-only keys require GET or HEAD). */
	method?: string | undefined;
	/** Registered route, for the read-only key refusal log only. */
	route?: string | undefined;
	/** Requested company ids checked only for read-only key callers. */
	readOnlyLedgerKeyRequestedCompanyIds?: readonly unknown[] | undefined;
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

function resolveAllowHeaderFallback(
	input: ResolveSessionContextInput,
	requireSession: boolean,
): boolean {
	if (requireSession) return false;

	if (typeof input.allowHeaderFallback === "boolean") {
		return input.allowHeaderFallback;
	}

	if (input.securityProfile === SESSION_SECURITY_PROFILE.SENSITIVE_WRITE) {
		return false;
	}

	return true;
}

/**
 * Resolves the authenticated caller context from headers and optional Better Auth session state.
 *
 * @param input - Raw request headers plus optional tenant and session enforcement options.
 * @returns The validated session context or a normalized authorization error payload.
 * @example
 * ```ts
 * const result = await resolveSessionContext({
 *   headers: {
 *     'x-user-id': 'usr_1',
 *     'x-user-role': 'admin',
 *     'x-company-id': 'cmp_1',
 *   },
 *   requireSession: false,
 * });
 * ```
 */
export function resolveSessionContext(
	input: ResolveSessionContextInput & {
		allowReadOnlyLedgerKey?: false | undefined;
	},
): Promise<SessionContextResult>;
export function resolveSessionContext(
	input: ResolveSessionContextInput,
): Promise<ReadOnlyLedgerKeySessionContextResult>;
export async function resolveSessionContext(
	input: ResolveSessionContextInput,
): Promise<ReadOnlyLedgerKeySessionContextResult> {
	const requireSession = shouldRequireSession(input.requireSession);
	const allowHeaderFallback = resolveAllowHeaderFallback(input, requireSession);
	const caller = await resolveAuthenticatedCaller({
		headers: input.headers,
		requestedCompanyId: input.requestedCompanyId,
		requireSession,
		allowHeaderFallback,
		allowMachineCaller: input.allowMachineCaller,
		machineCallerAllowlist: input.machineCallerAllowlist,
		requireTenant: true,
		requireRole: true,
		allowReadOnlyLedgerKey: input.allowReadOnlyLedgerKey,
		method: input.method,
		route: input.route,
		readOnlyLedgerKeyRequestedCompanyIds:
			input.readOnlyLedgerKeyRequestedCompanyIds,
	});

	if (caller.ok === false) {
		return {
			ok: false,
			status: caller.status,
			code: caller.code,
			error: caller.error,
			...(caller.retryAfterSeconds === undefined
				? {}
				: { retryAfterSeconds: caller.retryAfterSeconds }),
		};
	}

	return {
		ok: true,
		context: {
			userId: caller.caller.userId,
			authUserId: caller.caller.authUserId,
			legacyUserId: caller.caller.legacyUserId,
			role: caller.caller.role,
			companyId: caller.caller.companyId ?? "",
			...(caller.caller.readOnly ? { readOnly: true as const } : {}),
		},
	};
}
