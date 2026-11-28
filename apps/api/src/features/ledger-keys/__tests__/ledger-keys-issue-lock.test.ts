/**
 * Ledger key issue lock id and in-process bounds (fake `db.transaction`).
 * The lock itself needs Postgres and is covered by
 * `integration/ledger-keys.integration.test.ts` (RUN_DB_TESTS=1).
 */
import { createHash } from "node:crypto";
import { describe, expect, it, vi } from "vitest";

const open = { now: 0, max: 0 };
vi.mock("@drenyra/persistence/client", () => ({
	db: {
		async transaction<T>(run: (tx: { execute(): void }) => Promise<T>) {
			open.max = Math.max(open.max, ++open.now);
			return run({ execute: () => undefined }).finally(() => open.now--);
		},
	},
}));

import {
	LEDGER_KEY_ISSUE_LOCK_TIMEOUT_MS,
	LedgerKeyIssueLockTimeoutError,
	ledgerKeyIssueLockId,
	withLedgerKeyIssueLock,
} from "../ledger-keys-issue-lock";

const sleep = (ms: number) => new Promise((done) => setTimeout(done, ms));

const INT64_MIN = -(2n ** 63n);
const INT64_MAX = 2n ** 63n - 1n;
const COMPANY = "0b7e6d4c-1f43-4b2a-9d1e-3c5a7f9e2b10";

describe("ledgerKeyIssueLockId", () => {
	it("is the first 8 bytes of SHA-256 over userId:companyId, signed", () => {
		const expected = createHash("sha256")
			.update(`user-1:${COMPANY}`)
			.digest()
			.readBigInt64BE(0);
		expect(ledgerKeyIssueLockId("user-1", COMPANY)).toBe(expected);
	});

	it("is stable for the same pair and differs across users and companies", () => {
		const id = ledgerKeyIssueLockId("user-1", COMPANY);
		expect(ledgerKeyIssueLockId("user-1", COMPANY)).toBe(id);
		expect(ledgerKeyIssueLockId("user-2", COMPANY)).not.toBe(id);
		expect(
			ledgerKeyIssueLockId("user-1", "1c8f7e5d-2a54-4c3b-8e2f-4d6b8a0f3c21"),
		).not.toBe(id);
	});

	it("stays inside the Postgres bigint range", () => {
		for (let index = 0; index < 200; index++) {
			const id = ledgerKeyIssueLockId(`user-${index}`, COMPANY);
			expect(id >= INT64_MIN && id <= INT64_MAX).toBe(true);
		}
	});

	it("bounds the wait for the lock", () => {
		expect(LEDGER_KEY_ISSUE_LOCK_TIMEOUT_MS).toBe(5_000);
	});
});

describe("withLedgerKeyIssueLock bounds", () => {
	const lockFor = (userId: string) => ({ userId, companyId: COMPANY });
	const issue = (id: string, timeoutMs?: number) =>
		withLedgerKeyIssueLock(
			lockFor(id),
			() => sleep(10).then(() => id),
			timeoutMs,
		);

	it("keeps at most 2 lock transactions open for 10 distinct pairs, and all complete", async () => {
		const ids = Array.from({ length: 10 }, (_, index) => `user-${index}`);
		await expect(Promise.all(ids.map((id) => issue(id)))).resolves.toEqual(ids);
		expect([open.max, open.now]).toEqual([2, 0]);
	});

	it("gives a same-pair waiter BUSY after the deadline instead of waiting forever", async () => {
		let release = (): void => undefined;
		const held = new Promise<void>((done) => (release = done));
		const first = withLedgerKeyIssueLock(lockFor("user-busy"), () => held);
		const outcome = await Promise.race([
			issue("user-busy", 50).catch((error: unknown) => error),
			sleep(500).then(() => "still waiting"),
		]);
		release();
		await first;
		expect(outcome).toBeInstanceOf(LedgerKeyIssueLockTimeoutError);
	});

	it("releases the queue and the holder slots after work throws", async () => {
		const fail = () => Promise.reject(new Error("boom"));
		const boom = () => withLedgerKeyIssueLock(lockFor("user-throws"), fail);
		const failed = await Promise.allSettled([boom(), boom(), boom()]);
		expect(failed.map((result) => result.status)).not.toContain("fulfilled");
		const ids = ["user-throws", "user-x", "user-y"];
		const after = ids.map((id) => issue(id, 200));
		await expect(Promise.all(after)).resolves.toEqual(ids);
		expect(open.now).toBe(0);
	});
});
