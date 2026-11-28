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
 * Three safeguards keep the lock from starving the connection pool:
 * - Inside one process, callers for the same pair queue in memory first, so
 *   at most one pooled connection per pair waits on Postgres.
 * - At most `LEDGER_KEY_ISSUE_MAX_HOLDERS` callers per process hold the lock.
 * - One deadline (`LEDGER_KEY_ISSUE_LOCK_TIMEOUT_MS`) bounds the queue, the
 *   holder wait and `lock_timeout` together; past it the caller gets
 *   `LedgerKeyIssueLockTimeoutError` and nothing is created.
 *
 * @module ledger-keys/issue-lock
 */

/** Longest total wait (queue, holder slot, advisory lock) before giving up. */
export const LEDGER_KEY_ISSUE_LOCK_TIMEOUT_MS = 5_000;

/** Most lock holders per process; each uses ≤ 2 pooled connections (≤ 4 of 10). */
export const LEDGER_KEY_ISSUE_MAX_HOLDERS = 2;

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
let activeHolders = 0;
let holderFreed = signal();

function signal(): { promise: Promise<void>; fire: () => void } {
	let fire = (): void => undefined;
	const promise = new Promise<void>((resolve) => (fire = resolve));
	return { promise, fire };
}

/** Resolves with `ready`, or throws the lock timeout at `deadline`. */
function waitUntil(ready: Promise<void>, deadline: number): Promise<void> {
	let timer: ReturnType<typeof setTimeout> | undefined;
	const late = new Promise<never>((_, reject) => {
		const timedOut = () => reject(new LedgerKeyIssueLockTimeoutError());
		timer = setTimeout(timedOut, deadline - Date.now());
	});
	return Promise.race([ready, late]).finally(() => clearTimeout(timer));
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
	timeoutMs = LEDGER_KEY_ISSUE_LOCK_TIMEOUT_MS,
): Promise<T> {
	const deadline = Date.now() + timeoutMs;
	const lockId = ledgerKeyIssueLockId(lock.userId, lock.companyId).toString();
	const previous = inProcessTails.get(lockId) ?? Promise.resolve();
	const done = signal();
	// Successors also wait for the predecessor, even if this caller gives up.
	const tail = previous.then(() => done.promise);
	inProcessTails.set(lockId, tail);
	let holding = false;
	try {
		await waitUntil(previous, deadline);
		while (activeHolders >= LEDGER_KEY_ISSUE_MAX_HOLDERS) {
			await waitUntil(holderFreed.promise, deadline);
		}
		activeHolders++;
		holding = true;
		// `lock_timeout` 0 means no bound, so keep at least 1 ms.
		const remaining = `${Math.max(1, deadline - Date.now())}ms`;
		return await db.transaction(async (tx) => {
			await tx.execute(
				sql`select set_config('lock_timeout', ${remaining}, true)`,
			);
			await tx.execute(sql`select pg_advisory_xact_lock(${lockId}::bigint)`);
			return work();
		});
	} catch (error) {
		if (isLockTimeout(error)) throw new LedgerKeyIssueLockTimeoutError();
		throw error;
	} finally {
		if (holding) {
			activeHolders--;
			holderFreed.fire();
			holderFreed = signal();
		}
		done.fire();
		if (inProcessTails.get(lockId) === tail) inProcessTails.delete(lockId);
	}
}
