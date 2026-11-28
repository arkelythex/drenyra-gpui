import { createHash } from "node:crypto";
import { db } from "@drenyra/persistence/client";
import { sql } from "@drenyra/persistence/query";

/**
 * Ledger key issue lock — serializes "count active keys, then create one"
 * per (user, company), so concurrent issue requests cannot exceed the cap.
 *
 * The lock is a Postgres transaction-scoped advisory lock
 * (`pg_advisory_xact_lock`) on a stable 64-bit id derived from
 * `userId:companyId`, so it holds across API instances. It is released when
 * the lock transaction ends (commit or rollback), after the guarded work has
 * finished; the plugin's own writes commit on their own connections before
 * that, so the next holder counts them.
 *
 * Two safeguards keep the lock from starving the connection pool:
 * - Inside one process, callers for the same pair queue in memory first, so
 *   at most one pooled connection per pair waits on Postgres.
 * - The wait on Postgres is bounded by `lock_timeout`
 *   (`LEDGER_KEY_ISSUE_LOCK_TIMEOUT_MS`); past it the caller gets
 *   `LedgerKeyIssueLockTimeoutError` and nothing is created.
 *
 * @module ledger-keys/issue-lock
 */

/** Longest wait for the advisory lock before giving up. */
export const LEDGER_KEY_ISSUE_LOCK_TIMEOUT_MS = 5_000;

/** Postgres SQLSTATE `lock_not_available` (raised when `lock_timeout` hits). */
const LOCK_NOT_AVAILABLE = "55P03";

/** The advisory lock could not be taken within the timeout. */
export class LedgerKeyIssueLockTimeoutError extends Error {
	constructor() {
		super("Timed out waiting for the ledger key issue lock");
		this.name = "LedgerKeyIssueLockTimeoutError";
	}
}

/**
 * Stable signed 64-bit advisory lock id for one (user, company) pair: the
 * first 8 bytes of SHA-256 over `userId:companyId`, read as a signed
 * big-endian integer (the range of Postgres `bigint`).
 */
export function ledgerKeyIssueLockId(
	userId: string,
	companyId: string,
): bigint {
	return createHash("sha256")
		.update(`${userId}:${companyId}`)
		.digest()
		.readBigInt64BE(0);
}

const inProcessTails = new Map<string, Promise<void>>();

async function serializeInProcess<T>(
	key: string,
	work: () => Promise<T>,
): Promise<T> {
	const previous = inProcessTails.get(key) ?? Promise.resolve();
	const run = previous.then(work);
	const tail = run.then(
		() => undefined,
		() => undefined,
	);
	inProcessTails.set(key, tail);
	try {
		return await run;
	} finally {
		if (inProcessTails.get(key) === tail) inProcessTails.delete(key);
	}
}

function isLockTimeout(error: unknown): boolean {
	// Drizzle wraps driver errors; the SQLSTATE may sit on a cause.
	let current: unknown = error;
	for (let depth = 0; depth < 5 && current; depth++) {
		if (
			typeof current === "object" &&
			(current as { code?: unknown }).code === LOCK_NOT_AVAILABLE
		) {
			return true;
		}
		current =
			typeof current === "object"
				? (current as { cause?: unknown }).cause
				: undefined;
	}
	return false;
}

/**
 * Runs `work` while holding the issue lock for `(userId, companyId)`.
 *
 * @throws LedgerKeyIssueLockTimeoutError when the lock is not available
 *   within `LEDGER_KEY_ISSUE_LOCK_TIMEOUT_MS`; `work` has not run then.
 */
export async function withLedgerKeyIssueLock<T>(
	lock: { userId: string; companyId: string },
	work: () => Promise<T>,
): Promise<T> {
	const lockId = ledgerKeyIssueLockId(lock.userId, lock.companyId).toString();
	return serializeInProcess(lockId, async () => {
		try {
			return await db.transaction(async (tx) => {
				await tx.execute(
					sql`select set_config('lock_timeout', ${`${LEDGER_KEY_ISSUE_LOCK_TIMEOUT_MS}ms`}, true)`,
				);
				await tx.execute(sql`select pg_advisory_xact_lock(${lockId}::bigint)`);
				return work();
			});
		} catch (error) {
			if (isLockTimeout(error)) throw new LedgerKeyIssueLockTimeoutError();
			throw error;
		}
	});
}
