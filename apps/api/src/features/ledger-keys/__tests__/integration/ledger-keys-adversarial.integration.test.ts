/**
 * Read-only ledger keys — adversarial round trip (real plugin, real Postgres).
 *
 * Same disposable-database setup as `ledger-keys.integration.test.ts`
 * (RUN_DB_TESTS=1, baseline 0000 + 0035 + test prereqs, throwaway secret).
 * Covers S2: plugin HTTP endpoints unreachable, a key is never a session,
 * live membership / disabled / expiry / owner deletion, tenant headers,
 * per-key rate limit (60/min, 429 with Retry-After), content-type and origin
 * rules on management, no caching of issue/list, and the session-only
 * management envelope.
 */
import { createHash, randomBytes, randomUUID } from "node:crypto";
import { db } from "@drenyra/persistence/client";
import { eq, inArray, sql } from "@drenyra/persistence/query";
import {
	authApiKeys,
	authSessions,
	authUserCompanies,
	authUsers,
	users,
} from "@drenyra/persistence/schema";
import { makeSignature } from "better-auth/crypto";
import { Elysia } from "elysia";
import { afterAll, beforeAll, describe, expect, it } from "vitest";
import { auth } from "../../../auth/auth.config";
import { authRoutes } from "../../../auth/auth.routes";
import { ledgerModule } from "../../../ledger/routes";
import { ledgerExportRoutes } from "../../../ledger-mvp/export.routes";
import { ledgerKeysRoutes } from "../../ledger-keys.routes";

const describeDb = process.env.RUN_DB_TESTS === "1" ? describe : describe.skip;
const ACCOUNTS = "/api/ledger/accounts";
const KEYS = "/api/ledger-keys";

describeDb("read-only ledger keys (DB integration, adversarial)", () => {
	const app = new Elysia()
		.use(ledgerModule)
		.use(ledgerExportRoutes)
		.use(ledgerKeysRoutes)
		.use(authRoutes);
	const suffix = randomUUID().slice(0, 8);
	const legacyOwner = randomUUID();
	const owner = `lk-adv-owner-${suffix}`;
	const stranger = `lk-adv-stranger-${suffix}`;
	const doomed = `lk-adv-doomed-${suffix}`;
	let companyA = "";
	let companyB = "";
	const cookies: Record<string, string> = {};

	async function call(
		method: string,
		path: string,
		headers: Record<string, string> = {},
		body?: unknown,
	) {
		const response = await app.handle(
			new Request(`http://localhost${path}`, {
				method,
				headers: {
					...(body === undefined ? {} : { "content-type": "application/json" }),
					...headers,
				},
				...(body === undefined ? {} : { body: JSON.stringify(body) }),
			}),
		);
		const text = await response.text();
		let code: string | null = null;
		let data: unknown = null;
		try {
			const parsed = JSON.parse(text) as { code?: string; data?: unknown };
			code = parsed?.code ?? null;
			data = parsed?.data ?? null;
		} catch {
			// not JSON
		}
		return {
			status: response.status,
			headers: response.headers,
			text,
			code,
			data,
		};
	}

	async function signIn(userId: string): Promise<string> {
		const token = randomBytes(24).toString("hex");
		await db.insert(authSessions).values({
			id: randomUUID(),
			token,
			userId,
			expiresAt: new Date(Date.now() + 60 * 60 * 1000),
		});
		const signature = await makeSignature(
			token,
			process.env.BETTER_AUTH_SECRET ?? "",
		);
		return `better-auth.session_token=${encodeURIComponent(`${token}.${signature}`)}`;
	}

	async function issue(userId: string, companyId: string) {
		const response = await call(
			"POST",
			KEYS,
			{ cookie: cookies[userId] ?? "" },
			{ companyId, name: `adv ${randomUUID().slice(0, 6)}` },
		);
		expect(response.status).toBe(201);
		const { id, key: secret } = response.data as { id: string; key: string };
		return { id, secret };
	}

	const withKey = (secret: string) => ({ authorization: `Bearer ${secret}` });
	// Counts only this file's users' keys, so other DB test files running at
	// the same time against the same database cannot change the count.
	const countKeys = async () =>
		(
			await db
				.select({ id: authApiKeys.id })
				.from(authApiKeys)
				.where(inArray(authApiKeys.referenceId, [owner, stranger, doomed]))
		).length;

	beforeAll(async () => {
		expect((process.env.BETTER_AUTH_SECRET ?? "").length).toBeGreaterThan(31);
		await db.insert(users).values({
			id: legacyOwner,
			email: `lk-adv-${suffix}@example.test`,
			password: "not-a-real-password-hash",
			name: "Ledger keys adversarial",
		});
		const ruc = () =>
			`20${Array.from(randomBytes(9), (byte) => byte % 10).join("")}`;
		const rows = Array.from(
			await db.execute(sql`
				insert into companies (owner_id, ruc, business_name)
				values (${legacyOwner}, ${ruc()}, ${`Adv A ${suffix}`}),
					(${legacyOwner}, ${ruc()}, ${`Adv B ${suffix}`})
				returning id`),
		) as Array<{ id: string }>;
		companyA = rows[0]?.id ?? "";
		companyB = rows[1]?.id ?? "";
		for (const [id, companyId] of [
			[owner, companyA],
			[stranger, companyB],
			[doomed, companyA],
		] as const) {
			await db.insert(authUsers).values({
				id,
				name: id,
				email: `${id}@example.test`,
				emailVerified: true,
			});
			await db.insert(authUserCompanies).values({
				id: randomUUID(),
				userId: id,
				companyId,
				membershipRole: "OWNER",
				isDefault: true,
				membershipStatus: "active",
			});
			cookies[id] = await signIn(id);
		}
	});

	afterAll(async () => {
		for (const id of [owner, stranger, doomed]) {
			await db.delete(authUsers).where(eq(authUsers.id, id));
		}
		await db.execute(
			sql`delete from companies where id in (${companyA}, ${companyB})`,
		);
		await db.delete(users).where(eq(users.id, legacyOwner));
	});

	it("stores only the SHA-256 of the key and never returns it again", async () => {
		const issued = await issue(owner, companyA);
		const [row] = await db
			.select()
			.from(authApiKeys)
			.where(eq(authApiKeys.id, issued.id));
		const { key: storedHash, ...rest } = row ?? { key: "" };
		expect(storedHash).toBe(
			createHash("sha256").update(issued.secret).digest("base64url"),
		);
		expect(JSON.stringify(rest)).not.toContain(issued.secret.slice(11));
		const list = await call("GET", `${KEYS}?companyId=${companyA}`, {
			cookie: cookies[owner] ?? "",
		});
		expect(list.status).toBe(200);
		expect(list.text).not.toContain(issued.secret.slice(11));
		expect(list.text).not.toContain(storedHash);
		expect(list.text).not.toMatch(/permissions|metadata/);
	});

	it("keeps the plugin's HTTP endpoints unreachable, with a session or a key", async () => {
		const issued = await issue(owner, companyA);
		const before = await countKeys();
		const evil = {
			name: "evil",
			prefix: "drn_ro_",
			metadata: { companyId: companyB },
			keyId: issued.id,
		};
		const paths = [
			"/api/auth/api-key/create",
			"/api/auth/api-key/create/",
			"/api/auth//api-key/create",
			"/api/auth/API-KEY/create",
			"/api/auth/api-key/%63reate",
			"/api/auth/api-key/update",
			"/api/auth/api-key/update/",
			"/api/auth/api-key/delete",
			"/api/auth/api-key/verify",
		];
		for (const path of paths) {
			for (const headers of [
				{ cookie: cookies[owner] ?? "", origin: "http://localhost" },
				{ "x-api-key": issued.secret, origin: "http://localhost" },
			]) {
				const response = await call("POST", path, headers, evil);
				expect(response.status, `${path}`).toBe(404);
			}
		}
		for (const path of [
			"/api/auth/api-key/list",
			`/api/auth/api-key/get?id=${issued.id}`,
		]) {
			const response = await call("GET", path, {
				cookie: cookies[owner] ?? "",
			});
			expect(response.status, path).toBe(404);
		}
		expect(await countKeys()).toBe(before);
		const [row] = await db
			.select({ metadata: authApiKeys.metadata })
			.from(authApiKeys)
			.where(eq(authApiKeys.id, issued.id));
		expect(String(row?.metadata)).toContain(companyA);
		expect((await call("GET", ACCOUNTS, withKey(issued.secret))).status).toBe(
			200,
		);
	});

	it("never turns a key into a session", async () => {
		const issued = await issue(owner, companyA);
		expect(
			await auth.api.getSession({
				headers: new Headers({ "x-api-key": issued.secret }),
			}),
		).toBeNull();
		expect(
			await auth.api.getSession({
				headers: new Headers(withKey(issued.secret)),
			}),
		).toBeNull();
		for (const headers of [
			{ "x-api-key": issued.secret },
			withKey(issued.secret),
		]) {
			const response = await call("GET", "/api/auth/get-session", headers);
			expect(response.text).toBe("null");
		}
		for (const [method, path] of [
			["GET", `${KEYS}?companyId=${companyA}`],
			["POST", KEYS],
			["DELETE", `${KEYS}/${issued.id}`],
		] as const) {
			const response = await call(method, path, {
				...withKey(issued.secret),
				cookie: cookies[owner] ?? "",
			});
			expect(response.status).toBe(403);
			expect(response.code).toBe("API_KEY_NOT_ALLOWED");
		}
	});

	it("refuses foreign tenants, writes and non-opted routes in every NODE_ENV", async () => {
		const issued = await issue(owner, companyA);
		const previous = process.env.NODE_ENV;
		try {
			for (const env of ["test", "development", "production"]) {
				process.env.NODE_ENV = env;
				for (const header of ["x-company-id", "x-active-company-id"]) {
					const response = await call("GET", ACCOUNTS, {
						...withKey(issued.secret),
						[header]: companyB,
					});
					expect(response.status, `${env} ${header}`).toBe(403);
					expect(response.code).toBe("API_KEY_TENANT_MISMATCH");
				}
				const foreignQuery = await call(
					"GET",
					`${ACCOUNTS}?companyId=${companyB}`,
					withKey(issued.secret),
				);
				expect(foreignQuery.status, `${env} ?companyId`).toBe(403);
				expect(foreignQuery.code).toBe("API_KEY_TENANT_MISMATCH");
				expect(foreignQuery.data).toBeNull();
				const conflicting = await call("GET", ACCOUNTS, {
					...withKey(issued.secret),
					"x-company-id": companyA,
					"x-active-company-id": companyB,
				});
				expect(conflicting.status, env).toBe(403);
				expect(conflicting.code).toBe("API_KEY_TENANT_MISMATCH");
				expect(conflicting.data).toBeNull();
				const exported = await call(
					"POST",
					"/api/ledger/export/pdf",
					withKey(issued.secret),
					{ startDate: "2026-01-01", endDate: "2026-12-31" },
				);
				expect(exported.status, env).toBe(401);
				const identity = await call("GET", ACCOUNTS, {
					"x-api-key": `${issued.secret}x`,
					"x-user-id": owner,
					"x-user-role": "owner",
					"x-company-id": companyA,
				});
				expect(identity.status, env).toBe(401);
				expect(identity.code).toBe("API_KEY_INVALID");
			}
		} finally {
			process.env.NODE_ENV = previous;
		}
	});

	it("follows membership, enabled flag, expiry and owner deletion live", async () => {
		const issued = await issue(owner, companyA);
		const read = () => call("GET", ACCOUNTS, withKey(issued.secret));
		const setMembership = (status: string) =>
			db.execute(
				sql`update auth_user_companies set membership_status = ${status} where user_id = ${owner} and company_id = ${companyA}`,
			);
		expect((await read()).status).toBe(200);

		await setMembership("inactive");
		const inactive = await read();
		expect(inactive.status).toBe(403);
		expect(inactive.code).toBe("API_KEY_MEMBERSHIP_INACTIVE");
		await setMembership("active");
		expect((await read()).status).toBe(200);

		await db
			.update(authApiKeys)
			.set({ enabled: false })
			.where(eq(authApiKeys.id, issued.id));
		expect((await read()).code).toBe("API_KEY_INVALID");
		await db
			.update(authApiKeys)
			.set({ enabled: true, expiresAt: new Date(Date.now() - 1000) })
			.where(eq(authApiKeys.id, issued.id));
		const expired = await read();
		expect(expired.status).toBe(401);
		expect(expired.code).toBe("API_KEY_INVALID");

		const doomedKey = await issue(doomed, companyA);
		expect(
			(await call("GET", ACCOUNTS, withKey(doomedKey.secret))).status,
		).toBe(200);
		await db.delete(authUsers).where(eq(authUsers.id, doomed));
		const orphans = await db
			.select({ id: authApiKeys.id })
			.from(authApiKeys)
			.where(eq(authApiKeys.referenceId, doomed));
		expect(orphans).toEqual([]);
		expect(
			(await call("GET", ACCOUNTS, withKey(doomedKey.secret))).status,
		).toBe(401);
	});

	it("refuses form posts and foreign origins on management, and never caches issue or list", async () => {
		const before = await countKeys();
		const form = await app.handle(
			new Request(`http://localhost${KEYS}`, {
				method: "POST",
				headers: {
					cookie: cookies[owner] ?? "",
					"content-type": "application/x-www-form-urlencoded",
				},
				body: new URLSearchParams({ companyId: companyA, name: "csrf" }),
			}),
		);
		expect(form.status).toBe(415);
		const foreign = await call(
			"POST",
			KEYS,
			{ cookie: cookies[owner] ?? "", origin: "https://evil.example" },
			{ companyId: companyA, name: "csrf" },
		);
		expect(foreign.status).toBe(403);
		expect(foreign.code).toBe("ORIGIN_NOT_ALLOWED");
		const issued = await issue(owner, companyA);
		const foreignRevoke = await call("DELETE", `${KEYS}/${issued.id}`, {
			cookie: cookies[owner] ?? "",
			origin: "https://evil.example",
		});
		expect(foreignRevoke.status).toBe(403);
		expect(foreignRevoke.code).toBe("ORIGIN_NOT_ALLOWED");
		expect(await countKeys()).toBe(before + 1);

		const issuedResponse = await call(
			"POST",
			KEYS,
			{ cookie: cookies[owner] ?? "" },
			{ companyId: companyA, name: "cache check" },
		);
		const list = await call("GET", `${KEYS}?companyId=${companyA}`, {
			cookie: cookies[owner] ?? "",
		});
		expect(issuedResponse.status).toBe(201);
		expect(issuedResponse.headers.get("cache-control")).toBe("no-store");
		expect(list.headers.get("cache-control")).toBe("no-store");
	});

	it("answers unknown and foreign key ids with the same 404", async () => {
		const issued = await issue(owner, companyA);
		const foreign = await call("DELETE", `${KEYS}/${issued.id}`, {
			cookie: cookies[stranger] ?? "",
		});
		const unknown = await call("DELETE", `${KEYS}/${"Z".repeat(32)}`, {
			cookie: cookies[stranger] ?? "",
		});
		expect(foreign.status).toBe(404);
		expect(foreign.text).toBe(unknown.text);
		expect((await call("GET", ACCOUNTS, withKey(issued.secret))).status).toBe(
			200,
		);
	});

	it("limits one key to 60 verifications per minute and answers 429 with Retry-After", async () => {
		const issued = await issue(owner, companyA);
		const other = await issue(owner, companyA);
		for (let request = 0; request < 60; request++) {
			const response = await call("GET", ACCOUNTS, withKey(issued.secret));
			expect(response.status, `request ${request + 1}`).toBe(200);
		}
		const limited = await call("GET", ACCOUNTS, {
			...withKey(issued.secret),
			"x-user-id": owner,
			"x-user-role": "owner",
			"x-company-id": companyA,
		});
		expect(limited.status).toBe(429);
		const retryAfter = Number(limited.headers.get("retry-after"));
		expect(Number.isInteger(retryAfter)).toBe(true);
		expect(retryAfter).toBeGreaterThanOrEqual(1);
		expect(retryAfter).toBeLessThanOrEqual(60);
		expect(JSON.parse(limited.text)).toEqual({
			success: false,
			error: "API key rate limit exceeded; retry later",
			code: "API_KEY_RATE_LIMITED",
		});
		expect((await call("GET", ACCOUNTS, withKey(other.secret))).status).toBe(
			200,
		);
	}, 60_000);
});
