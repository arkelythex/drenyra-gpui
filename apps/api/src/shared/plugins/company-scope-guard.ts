/**
 * Company Scope Guard — Elysia Plugin
 *
 * Wraps `resolveSessionContext()` from the security module and injects
 * `companyContext: CompanyContext` into the request store via Elysia's
 * `derive`.
 *
 * ## Usage
 *
 * ```ts
 * import { companyScopeGuard } from "../../shared/plugins/company-scope-guard";
 *
 * new Elysia({ prefix: "/api/analytics" })
 *   .use(companyScopeGuard())
 *   .get("/", ({ companyContext }) => {
 *     // companyContext.companyId is available
 *   });
 * ```
 *
 * ## Migration
 *
 * During Phase 3 migration, pass `allowHeaderFallback: true` for backward
 * compatibility with features still using `requireCompanyIdFromHeaders`:
 *
 * ```ts
 * .use(companyScopeGuard({ allowHeaderFallback: true }))
 * ```
 *
 * Phase 4 will remove the fallback and mandate session-only context.
 *
 * ## Read-only ledger keys
 *
 * Modules that serve ledger reads opt in with `allowReadOnlyLedgerKey: true`.
 * A `drn_ro_…` key is then accepted on GET/HEAD for its own company only,
 * and `companyContext.readOnly` is `true`. A refused key always fails closed
 * (401/403, 429 with `Retry-After` when rate limited, or 503 with
 * `Retry-After` when the key cannot be verified right now), even with
 * `allowHeaderFallback`. A `?companyId=` other than the key's company is
 * refused with 403 `API_KEY_TENANT_MISMATCH`; for sessions and header
 * fallback the query is not read here (unchanged behavior).
 *
 * @module company-scope-guard
 */

import type { Elysia } from "elysia";
import { isReadOnlyLedgerKeyRefusal } from "../../features/security/read-only-ledger-key";
import {
	resolveSessionContext,
	type SessionContext,
} from "../../features/security/session-context";
import { fail } from "../../features/shared/api-response";

/**
 * Company-scoped context injected into Elysia's store by the guard.
 *
 * Subset of `SessionContext` — only the fields relevant for tenant scoping.
 */
export interface CompanyContext {
	/** Canonical user identifier (Better Auth) */
	userId: string;
	/** Authenticated user identifier (alias for userId) */
	authUserId: string;
	/** Legacy user identifier (pre-Better Auth) */
	legacyUserId: string | null;
	/** User role for RBAC decisions */
	role: string;
	/** Tenant (company) identifier — used for all query scoping */
	companyId: string;
	/**
	 * `true` only for a read-only ledger key caller: it may read, never write.
	 * Absent for every other caller.
	 */
	readOnly?: true;
}

/**
 * Options for configuring the company scope guard plugin.
 */
export interface CompanyScopeGuardOptions {
	/**
	 * If `true`, allows header-based company resolution as a fallback when
	 * an active Better Auth session is not present.
	 *
	 * Default: `false`
	 *
	 * Use `true` during Phase 3 migration for backward compatibility.
	 * Phase 4 will require session-only context.
	 */
	allowHeaderFallback?: boolean;
	/**
	 * If `true`, a read-only ledger key (`drn_ro_…`) is accepted for GET/HEAD
	 * requests of this module, scoped to the key's company. Writes with a key
	 * are refused with 403. Only for modules whose GET routes are ledger reads.
	 *
	 * For a key caller, `?companyId=` must be the key's company (403
	 * `API_KEY_TENANT_MISMATCH` otherwise). A request body is not read: a key
	 * request is GET/HEAD (no body), and any other method is refused with
	 * `API_KEY_READ_ONLY` before tenant checks.
	 *
	 * Default: `false` (keys are ignored).
	 */
	allowReadOnlyLedgerKey?: boolean;
}

/**
 * Sends `Retry-After` when the refusal carries one (rate-limited key, or key
 * verification unavailable).
 */
function applyRetryAfter(
	headers: Record<string, string | number>,
	result: { retryAfterSeconds?: number },
): void {
	if (result.retryAfterSeconds !== undefined) {
		headers["retry-after"] = String(result.retryAfterSeconds);
	}
}

/**
 * Every `companyId` value in the query (`?companyId=a&companyId=b` gives
 * both), or none when the parameter is absent. Checked only for read-only
 * ledger key callers.
 */
function requestedCompanyIdsOf(query: unknown): unknown[] {
	if (!query || typeof query !== "object") return [];
	if (!Object.hasOwn(query, "companyId")) return [];
	const value = (query as { companyId?: unknown }).companyId;
	return Array.isArray(value) ? value : [value];
}

/**
 * Header-fallback modules pass a failed resolution through as "no context",
 * except a refused read-only key: that request is answered with the refusal.
 */
function passesThroughWithoutContext(
	allowHeaderFallback: boolean,
	code: string,
): boolean {
	return allowHeaderFallback && !isReadOnlyLedgerKeyRefusal(code);
}

function toCompanyContext(context: SessionContext): CompanyContext {
	return {
		userId: context.userId,
		authUserId: context.authUserId,
		legacyUserId: context.legacyUserId,
		role: context.role,
		companyId: context.companyId,
		...(context.readOnly ? { readOnly: true as const } : {}),
	};
}

/**
 * Reusable Elysia plugin that resolves company context from the request
 * and injects it into the Elysia store as `companyContext`.
 *
 * Fail-closed: if session context cannot be resolved, the request is
 * rejected with the appropriate error status (401 or 403).
 *
 * @param options - Configuration options for the guard.
 * @returns An Elysia plugin compatible with `.use()`.
 *
 * @example
 * // Apply at module level — all routes inherit company scoping
 * export const myModule = new Elysia({ prefix: "/api/my-feature" })
 *   .use(companyScopeGuard())
 *   .get("/", ({ companyContext }) => {
 *     // companyContext.companyId is always available here
 *   });
 *
 * @example
 * // With header fallback during migration
 * .use(companyScopeGuard({ allowHeaderFallback: true }))
 */
export function companyScopeGuard(options: CompanyScopeGuardOptions = {}) {
	const { allowHeaderFallback = false, allowReadOnlyLedgerKey = false } =
		options;

	type GuardRequest = {
		headers: Record<string, string | undefined>;
		method: string;
		set: { headers: unknown };
		route?: string;
		requestedCompanyIds?: unknown[];
	};
	type GuardOutcome =
		| { refused: false; companyContext: CompanyContext }
		| {
				refused: true;
				status: 401 | 403 | 429 | 503;
				body: ReturnType<typeof fail>;
		  };

	const passThrough = (): GuardOutcome => ({
		refused: false,
		companyContext: undefined as unknown as CompanyContext,
	});

	const resolveGuard = async (input: GuardRequest): Promise<GuardOutcome> => {
		/**
		 * During Phase 3 migration (allowHeaderFallback: true), do not
		 * require a full session — the guard will first attempt session
		 * resolution and fall back to header-based identity.
		 *
		 * When allowHeaderFallback: false (Phase 4+), session is required.
		 *
		 * Note: resolveSessionContext internally forces allowHeaderFallback
		 * to false when requireSession is true, so we MUST pass
		 * requireSession=false when using the fallback.
		 */
		const requireSession = !allowHeaderFallback;

		try {
			const result = await resolveSessionContext({
				headers: input.headers as Record<string, unknown>,
				requireSession,
				allowHeaderFallback,
				allowReadOnlyLedgerKey,
				method: input.method,
				route: input.route,
				readOnlyLedgerKeyRequestedCompanyIds: input.requestedCompanyIds,
			});

			if (!result.ok) {
				/**
				 * During migration (allowHeaderFallback: true), we log the
				 * rejection but do NOT fail-closed — this preserves backward
				 * compatibility for features that want to handle their own
				 * company scoping while the guard is being adopted.
				 *
				 * In Phase 4 (allowHeaderFallback: false), we fail closed.
				 */
				if (passesThroughWithoutContext(allowHeaderFallback, result.code)) {
					// Pass through — no company context available.
					// The handler can still use its existing scoping mechanism.
					return passThrough();
				}

				applyRetryAfter(
					input.set.headers as Record<string, string | number>,
					result,
				);
				return {
					refused: true,
					status: result.status,
					body: fail(result.error, result.code),
				};
			}

			return {
				refused: false,
				companyContext: toCompanyContext(result.context),
			};
		} catch (error) {
			/**
			 * If resolveSessionContext throws (e.g. DB is unavailable in
			 * test environments or session validation fails unexpectedly),
			 * behave according to the fallback mode.
			 *
			 * With allowHeaderFallback: true, gracefully degrade — the
			 * handler can continue with its existing scoping mechanism.
			 *
			 * With allowHeaderFallback: false, re-throw to fail closed.
			 */
			if (allowHeaderFallback) {
				return passThrough();
			}
			throw error;
		}
	};

	// Two derive functions on purpose: Elysia infers from the parameters a
	// derive reads which request parts to parse, so modules that do not opt
	// in to keys keep exactly the previous inference (no `query`, no `route`).
	// A refusal is answered with `status()` here; a thrown error would reach
	// the app's error handler and become a 500.
	if (allowReadOnlyLedgerKey) {
		return (app: Elysia) =>
			app.derive(async ({ headers, request, set, status, query, route }) => {
				const outcome = await resolveGuard({
					headers,
					method: request.method,
					set,
					route,
					requestedCompanyIds: requestedCompanyIdsOf(query),
				});
				if (outcome.refused) return status(outcome.status, outcome.body);
				return { companyContext: outcome.companyContext };
			});
	}

	return (app: Elysia) =>
		app.derive(async ({ headers, request, set, status }) => {
			const outcome = await resolveGuard({
				headers,
				method: request.method,
				set,
			});
			if (outcome.refused) return status(outcome.status, outcome.body);
			return { companyContext: outcome.companyContext };
		});
}

/**
 * `onBeforeHandle` hook for modules that take `companyId` in the query or
 * body: it must be the session's company. Without it, any signed-in user
 * could read or change another company's data by changing that parameter.
 *
 * @example
 * new Elysia({ prefix: "/api/x" })
 *   .use(companyScopeGuard())
 *   .onBeforeHandle(rejectForeignCompanyId)
 */
export function rejectForeignCompanyId(context: {
	query?: unknown;
	body?: unknown;
	companyContext?: CompanyContext;
	set: { status?: number | string };
}) {
	const requested =
		(context.query as { companyId?: unknown } | undefined)?.companyId ??
		(context.body as { companyId?: unknown } | null | undefined)?.companyId;
	if (requested === undefined) return;
	if (requested !== context.companyContext?.companyId) {
		context.set.status = 403;
		return fail("Company scope mismatch", "COMPANY_SCOPE_MISMATCH");
	}
	return undefined;
}
