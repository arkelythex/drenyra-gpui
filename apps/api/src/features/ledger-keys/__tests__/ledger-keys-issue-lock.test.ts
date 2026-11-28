/**
 * Ledger key issue lock id: a stable signed 64-bit value per (user, company).
 * The lock itself needs Postgres and is covered by
 * `integration/ledger-keys.integration.test.ts` (RUN_DB_TESTS=1).
 */
import { createHash } from "node:crypto";
import { describe, expect, it } from "vitest";
import {
	LEDGER_KEY_ISSUE_LOCK_TIMEOUT_MS,
	ledgerKeyIssueLockId,
} from "../ledger-keys-issue-lock";

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
