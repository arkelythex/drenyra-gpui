/**
 * Read-only ledger keys — real Better Auth api-key plugin + drizzle adapter
 * round trip against Postgres.
 *
 * Proves, with no mocks: a key issued through the service is stored hashed;
 * it authenticates a ledger GET as its own company only; `key-identity`
 * answers company A's id, RUC and name; the session gets 403 there; another
 * company's `x-company-id` is refused; list never shows the key; once revoked through
 * the route, the same key is refused (401). The 10-key cap holds under
 * concurrency (per user and company advisory lock), and issuance waits for, or
 * gives up on (409 LEDGER_KEY_ISSUE_BUSY), a lock held by another transaction.
 *
 * Runs only with RUN_DB_TESTS=1 and only against a DISPOSABLE database:
 *
 *   docker exec <pg> psql -U postgres -c "create database <db>"
 *   for f in 0000_canonical_baseline.sql 0035_auth_api_keys.sql; do
 *     sed 's/--> statement-breakpoint//' packages/infrastructure/drizzle/$f \
 *       | docker exec -i <pg> psql -U postgres -d <db> -v ON_ERROR_STOP=1 -q
 *   done
 *   docker exec -i <pg> psql -U postgres -d <db> -v ON_ERROR_STOP=1 -q \
 *     < apps/api/test/db/ledger-keys-db-test-prereqs.sql
 *   cd apps/api && RUN_DB_TESTS=1 DATABASE_URL=<disposable url> \
 *     BETTER_AUTH_SECRET=<throwaway, 32+ chars> \
 *     bunx vitest run src/features/ledger-keys/__tests__/integration
 *
 * The session cookie is signed with BETTER_AUTH_SECRET exactly as Better Auth
 * signs it, so `getSession` resolves a real session row.
 *
 * @module ledger-keys/__tests__/integration/ledger-keys.integration.test
 */
import { randomBytes, randomUUID } from "node:crypto";
import { db } from "@drenyra/persistence/client";
import { eq, inArray, sql } from "@drenyra/persistence/query";
import {
	authApiKeys,
	authSessions,
	authUserCompanies,
	authUsers,
	categories,
	companies,
	users,
} from "@drenyra/persistence/schema";
import { makeSignature } from "better-auth/crypto";
import { Elysia } from "elysia";
import { afterAll, beforeAll, describe, expect, it } from "vitest";
import { READ_ONLY_LEDGER_KEY_PREFIX } from "../../../auth/lib/read-only-ledger-key";
import { ledgerModule } from "../../../ledger/routes";
import { ledgerKeysRoutes } from "../../ledger-keys.routes";
import { issueLedgerKey } from "../../ledger-keys.service";
import {
	LEDGER_KEY_ISSUE_LOCK_TIMEOUT_MS,
	ledgerKeyIssueLockId,
} from "../../ledger-keys-issue-lock";

const describeDb = process.env.RUN_DB_TESTS === "1" ? describe : describe.skip;

const PREREQUISITE_COLUMNS = [
	["auth_users", "totp_enabled"],
	["auth_sessions", "mfa_verified"],
	["auth_user_companies", "membership_status"],
] as const;

type Fixture = {
	ownerLegacyUserId: string;
	authUserId: string;
	companyA: string;
	companyB: string;
	rucA: string;
	businessNameA: string;
	categoryA: string;
	categoryB: string;
	sessionId: string;
	cookie: string;
};

function randomRuc(): string {
	const digits = Array.from(randomBytes(9), (byte) => byte % 10).join("");
	return `20${digits}`;
}

async function assertPrerequisites(): Promise<void> {
	const secret = process.env.BETTER_AUTH_SECRET ?? "";
	if (secret.length < 32) {
		throw new Error(
			"BETTER_AUTH_SECRET (throwaway, 32+ chars) is required to sign the test session cookie",
		);
	}
	for (const [table, column] of PREREQUISITE_COLUMNS) {
		const rows = await db.execute(
			sql`select 1 from information_schema.columns where table_name = ${table} and column_name = ${column}`,
		);
		if (rows.length === 0) {
			throw new Error(
				`Missing ${table}.${column}: apply apps/api/test/db/ledger-keys-db-test-prereqs.sql to this disposable database first`,
			);
		}
	}
}

async function createFixture(): Promise<Fixture> {
	const suffix = randomUUID().slice(0, 8);
	const ownerLegacyUserId = randomUUID();
	await db.insert(users).values({
		id: ownerLegacyUserId,
		email: `ledger-keys-owner-${suffix}@example.test`,
		password: "not-a-real-password-hash",
		name: "Ledger keys owner",
	});

	// Raw SQL on purpose: drizzle inserts every schema column, and the
	// baseline `companies` table lacks the newer settings_* columns (drift).
	const rucA = randomRuc();
	const businessNameA = `Empresa A ${suffix}`;
	const companyRows = await db.execute(sql`
		insert into companies (owner_id, ruc, business_name)
		values
			(${ownerLegacyUserId}, ${rucA}, ${businessNameA}),
			(${ownerLegacyUserId}, ${randomRuc()}, ${`Empresa B ${suffix}`})
		returning id
	`);
	const [companyA, companyB] = Array.from(companyRows) as Array<{
		id: string;
	}>;
	if (!companyA || !companyB) throw new Error("company fixture not created");

	const authUserId = `ledger-keys-user-${suffix}`;
	await db.insert(authUsers).values({
		id: authUserId,
		name: "Ledger keys user",
		email: `ledger-keys-user-${suffix}@example.test`,
		emailVerified: true,
	});
	// Active member of company A only.
	await db.insert(authUserCompanies).values({
		id: randomUUID(),
		userId: authUserId,
		companyId: companyA.id,
		membershipRole: "OWNER",
		isDefault: true,
		membershipStatus: "active",
	});

	const [categoryA, categoryB] = await db
		.insert(categories)
		.values([
			{ companyId: companyA.id, name: `Ventas A ${suffix}`, type: "INCOME" },
			{ companyId: companyB.id, name: `Ventas B ${suffix}`, type: "INCOME" },
		])
		.returning({ id: categories.id });
	if (!categoryA || !categoryB) throw new Error("category fixture not created");

	const token = randomBytes(24).toString("hex");
	const sessionId = randomUUID();
	await db.insert(authSessions).values({
		id: sessionId,
		token,
		userId: authUserId,
		expiresAt: new Date(Date.now() + 60 * 60 * 1000),
	});
	const signature = await makeSignature(
		token,
		process.env.BETTER_AUTH_SECRET ?? "",
	);
	const cookie = `better-auth.session_token=${encodeURIComponent(`${token}.${signature}`)}`;

	return {
		ownerLegacyUserId,
		authUserId,
		companyA: companyA.id,
		companyB: companyB.id,
		rucA,
		businessNameA,
		categoryA: categoryA.id,
		categoryB: categoryB.id,
		sessionId,
		cookie,
	};
}

async function dropFixture(fixture: Fixture): Promise<void> {
	// auth_users cascades to sessions, memberships and api keys.
	await db.delete(authUsers).where(eq(authUsers.id, fixture.authUserId));
	await db
		.delete(categories)
		.where(inArray(categories.id, [fixture.categoryA, fixture.categoryB]));
	await db
		.delete(companies)
		.where(inArray(companies.id, [fixture.companyA, fixture.companyB]));
	await db.delete(users).where(eq(users.id, fixture.ownerLegacyUserId));
}

describeDb("read-only ledger keys (DB integration, real plugin)", () => {
	const app = new Elysia().use(ledgerModule).use(ledgerKeysRoutes);
	let fixture: Fixture | null = null;

	beforeAll(async () => {
		await assertPrerequisites();
		fixture = await createFixture();
	});

	afterAll(async () => {
		if (fixture) await dropFixture(fixture);
	});

	function trialBalance(key: string, headers: Record<string, string> = {}) {
		return app.handle(
			new Request(
				"http://localhost/api/ledger/trial-balance?asOfDate=2026-12-31",
				{ headers: { authorization: `Bearer ${key}`, ...headers } },
			),
		);
	}

	it("issues, uses, scopes, lists and revokes a key end to end", async () => {
		if (!fixture) throw new Error("fixture missing");
		const sessionHeaders = new Headers({ cookie: fixture.cookie });

		// Issue through the service (real plugin, real drizzle adapter).
		const issued = await issueLedgerKey({
			userId: fixture.authUserId,
			companyId: fixture.companyA,
			name: "db integration",
			expiresInDays: 1,
			sessionHeaders,
		});
		expect(issued.ok).toBe(true);
		if (!issued.ok) return;
		const plaintext = issued.key.key;
		expect(plaintext.startsWith(READ_ONLY_LEDGER_KEY_PREFIX)).toBe(true);
		expect(issued.key.companyId).toBe(fixture.companyA);
		expect(issued.key.start).toBe(plaintext.slice(0, 11));

		// Stored hashed: the key column is not the plaintext.
		const [stored] = await db
			.select()
			.from(authApiKeys)
			.where(eq(authApiKeys.id, issued.key.id));
		expect(stored).toBeDefined();
		expect(stored?.key).not.toBe(plaintext);
		expect(stored?.key).not.toContain(plaintext.slice(7));
		expect(stored?.referenceId).toBe(fixture.authUserId);
		expect(stored?.prefix).toBe(READ_ONLY_LEDGER_KEY_PREFIX);
		expect(JSON.parse(stored?.permissions ?? "null")).toEqual({
			ledger: ["read"],
		});
		expect(String(stored?.metadata)).toContain(fixture.companyA);

		// Authenticates a ledger GET as company A only.
		const ok = await trialBalance(plaintext);
		expect(ok.status).toBe(200);
		const okPayload = (await ok.json()) as {
			success: boolean;
			data: { accounts: Array<{ name: string }> };
		};
		expect(okPayload.success).toBe(true);
		const names = okPayload.data.accounts.map((account) => account.name);
		expect(names.some((name) => name.startsWith("Ventas A"))).toBe(true);
		expect(names.some((name) => name.startsWith("Ventas B"))).toBe(false);

		// key-identity names company A, read from the companies table.
		const identity = await app.handle(
			new Request("http://localhost/api/ledger/key-identity", {
				headers: { authorization: `Bearer ${plaintext}` },
			}),
		);
		expect(identity.status).toBe(200);
		expect(await identity.json()).toEqual({
			success: true,
			data: {
				companyId: fixture.companyA,
				ruc: fixture.rucA,
				businessName: fixture.businessNameA,
			},
		});

		// Only key callers get an identity: the session is refused.
		const sessionIdentity = await app.handle(
			new Request("http://localhost/api/ledger/key-identity", {
				headers: { cookie: fixture.cookie },
			}),
		);
		expect(sessionIdentity.status).toBe(403);
		expect(((await sessionIdentity.json()) as { code: string }).code).toBe(
			"KEY_IDENTITY_REQUIRES_API_KEY",
		);

		// Another company's tenant header is refused.
		const foreign = await trialBalance(plaintext, {
			"x-company-id": fixture.companyB,
		});
		expect(foreign.status).toBe(403);
		expect(((await foreign.json()) as { code: string }).code).toBe(
			"API_KEY_TENANT_MISMATCH",
		);

		// A key cannot manage keys.
		const keyManaging = await app.handle(
			new Request(
				`http://localhost/api/ledger-keys?companyId=${fixture.companyA}`,
				{ headers: { authorization: `Bearer ${plaintext}` } },
			),
		);
		expect(keyManaging.status).toBe(403);

		// The session lists the key without any secret.
		const list = await app.handle(
			new Request(
				`http://localhost/api/ledger-keys?companyId=${fixture.companyA}`,
				{ headers: { cookie: fixture.cookie } },
			),
		);
		expect(list.status).toBe(200);
		const listText = await list.text();
		expect(listText).not.toContain(plaintext);
		expect(listText).not.toContain(stored?.key ?? "missing-hash");
		const listed = (JSON.parse(listText) as { data: Array<{ id: string }> })
			.data;
		expect(listed.map((key) => key.id)).toEqual([issued.key.id]);

		// Revoke through the route, with the session.
		const revoke = await app.handle(
			new Request(`http://localhost/api/ledger-keys/${issued.key.id}`, {
				method: "DELETE",
				headers: { cookie: fixture.cookie },
			}),
		);
		expect(revoke.status).toBe(200);
		const remaining = await db
			.select({ id: authApiKeys.id })
			.from(authApiKeys)
			.where(eq(authApiKeys.id, issued.key.id));
		expect(remaining).toEqual([]);

		// The same key is now refused.
		const revoked = await trialBalance(plaintext);
		expect(revoked.status).toBe(401);
		expect(((await revoked.json()) as { code: string }).code).toBe(
			"API_KEY_INVALID",
		);
	});

	describe("issue cap under concurrency (advisory lock per user and company)", () => {
		async function deleteAllKeys(userId: string) {
			await db.delete(authApiKeys).where(eq(authApiKeys.referenceId, userId));
		}

		async function activeKeyCount(userId: string, companyId: string) {
			const rows = await db.execute(sql`
				select count(*)::int as count from auth_api_keys
				where reference_id = ${userId}
					and enabled = true
					and (expires_at is null or expires_at > now())
					and metadata like ${`%${companyId}%`}`);
			return Number((Array.from(rows)[0] as { count: number }).count);
		}

		function issueOverHttp(cookie: string, companyId: string, name: string) {
			return app.handle(
				new Request("http://localhost/api/ledger-keys", {
					method: "POST",
					headers: { cookie, "content-type": "application/json" },
					body: JSON.stringify({ companyId, name }),
				}),
			);
		}

		it("lets exactly one of 6 parallel issues through when 9 keys are active", async () => {
			if (!fixture) throw new Error("fixture missing");
			const { authUserId, companyA, cookie } = fixture;
			await deleteAllKeys(authUserId);
			try {
				for (let index = 0; index < 9; index++) {
					const issued = await issueLedgerKey({
						userId: authUserId,
						companyId: companyA,
						name: `existing ${index}`,
						expiresInDays: 1,
						sessionHeaders: new Headers({ cookie }),
					});
					expect(issued.ok).toBe(true);
				}
				expect(await activeKeyCount(authUserId, companyA)).toBe(9);

				const responses = await Promise.all(
					Array.from({ length: 6 }, (_, index) =>
						issueOverHttp(cookie, companyA, `parallel ${index}`),
					),
				);
				const statuses = responses.map((response) => response.status).sort();
				expect(statuses).toEqual([201, 409, 409, 409, 409, 409]);
				for (const response of responses) {
					if (response.status !== 409) continue;
					expect(((await response.json()) as { code: string }).code).toBe(
						"LEDGER_KEY_LIMIT_REACHED",
					);
				}
				expect(await activeKeyCount(authUserId, companyA)).toBe(10);
			} finally {
				await deleteAllKeys(authUserId);
			}
		}, 30_000);

		it("waits while another transaction holds the (user, company) advisory lock", async () => {
			if (!fixture) throw new Error("fixture missing");
			const { authUserId, companyA, cookie } = fixture;
			await deleteAllKeys(authUserId);
			const lockId = ledgerKeyIssueLockId(authUserId, companyA).toString();
			let settled = false;
			let pending: ReturnType<typeof issueLedgerKey> | null = null;
			try {
				await db.transaction(async (tx) => {
					await tx.execute(
						sql`select pg_advisory_xact_lock(${lockId}::bigint)`,
					);
					pending = issueLedgerKey({
						userId: authUserId,
						companyId: companyA,
						name: "waits for the lock",
						expiresInDays: 1,
						sessionHeaders: new Headers({ cookie }),
					});
					void pending.then(() => {
						settled = true;
					});
					await new Promise((resolve) => setTimeout(resolve, 500));
					expect(settled).toBe(false);
					expect(await activeKeyCount(authUserId, companyA)).toBe(0);
				});
				const issued = await (pending as ReturnType<
					typeof issueLedgerKey
				> | null);
				expect(issued?.ok).toBe(true);
				expect(await activeKeyCount(authUserId, companyA)).toBe(1);
			} finally {
				await deleteAllKeys(authUserId);
			}
		}, 30_000);

		it("gives up after the lock timeout with 409 LEDGER_KEY_ISSUE_BUSY and creates nothing", async () => {
			if (!fixture) throw new Error("fixture missing");
			const { authUserId, companyA, cookie } = fixture;
			await deleteAllKeys(authUserId);
			const lockId = ledgerKeyIssueLockId(authUserId, companyA).toString();
			try {
				await db.transaction(async (tx) => {
					await tx.execute(
						sql`select pg_advisory_xact_lock(${lockId}::bigint)`,
					);
					const started = Date.now();
					const response = await issueOverHttp(cookie, companyA, "busy");
					expect(Date.now() - started).toBeGreaterThanOrEqual(
						LEDGER_KEY_ISSUE_LOCK_TIMEOUT_MS - 100,
					);
					expect(response.status).toBe(409);
					expect(await response.json()).toEqual({
						success: false,
						error: "Another ledger key is being issued for this company; retry",
						code: "LEDGER_KEY_ISSUE_BUSY",
					});
				});
				expect(await activeKeyCount(authUserId, companyA)).toBe(0);
			} finally {
				await deleteAllKeys(authUserId);
			}
		}, 30_000);
	});
});
