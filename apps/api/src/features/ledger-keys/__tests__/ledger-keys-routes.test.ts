/**
 * Ledger key management routes (`/api/ledger-keys`).
 *
 * Issue, list and revoke the caller's own read-only ledger keys for one
 * company. Only a Better Auth session may call these routes: a ledger key,
 * header identity or no identity are refused before anything is read or
 * written. `auth.api.*` and the membership lookup are mocked; the plugin's
 * list/delete semantics (own keys only, someone else's key is NOT_FOUND) are
 * modelled by an in-memory store. The real plugin round trip is covered by
 * `integration/ledger-keys.integration.test.ts` (RUN_DB_TESTS=1).
 */
import { randomUUID } from "node:crypto";
import { APIError } from "better-auth/api";
import { Elysia } from "elysia";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("../../auth/handlers/company-membership", async (importOriginal) => {
	const actual =
		await importOriginal<
			typeof import("../../auth/handlers/company-membership")
		>();
	return { ...actual, listUserCompanyMemberships: vi.fn() };
});

// The Postgres advisory lock needs a database; here it only records the
// (user, company) pair and runs the guarded work. The real lock is covered by
// `integration/ledger-keys.integration.test.ts` (RUN_DB_TESTS=1).
const lockCalls: Array<{ userId: string; companyId: string }> = [];
let lockBusy = false;
vi.mock("../ledger-keys-issue-lock", async (importOriginal) => {
	const actual =
		await importOriginal<typeof import("../ledger-keys-issue-lock")>();
	return {
		...actual,
		withLedgerKeyIssueLock: vi.fn(
			async <T>(
				lock: { userId: string; companyId: string },
				work: () => Promise<T>,
			): Promise<T> => {
				lockCalls.push({ userId: lock.userId, companyId: lock.companyId });
				if (lockBusy) throw new actual.LedgerKeyIssueLockTimeoutError();
				return work();
			},
		),
	};
});

const { auth } = await import("../../auth/auth.config");
const { listUserCompanyMemberships } = await import(
	"../../auth/handlers/company-membership"
);
const { ledgerKeysRoutes } = await import("../index");
const { LEDGER_KEYS_MAX_ACTIVE_PER_COMPANY } = await import(
	"../ledger-keys.service"
);

const OWNER = "auth-user-owner";
const OTHER_USER = "auth-user-other";
const COMPANY_A = randomUUID();
const COMPANY_B = randomUUID();
const SESSION_COOKIE = "better-auth.session_token=signed-session-token";
const PRESENTED_KEY = "drn_ro_presentedKeyAAAAAAAAAAAAAAAAAAAAAAAAAAAA";
const DAY_SECONDS = 24 * 60 * 60;

type FakeKey = {
	id: string;
	name: string | null;
	start: string | null;
	prefix: string | null;
	key: string;
	referenceId: string;
	enabled: boolean;
	createdAt: Date;
	updatedAt: Date;
	expiresAt: Date | null;
	lastRequest: Date | null;
	metadata: Record<string, unknown> | null;
	permissions: Record<string, string[]> | null;
};

let store: FakeKey[] = [];
let sessionUserId: string | null = OWNER;

function fakeKey(overrides: Partial<FakeKey> = {}): FakeKey {
	const now = new Date("2026-11-27T10:00:00.000Z");
	return {
		id: `key-${randomUUID()}`,
		name: "laptop",
		start: "drn_ro_abcd",
		prefix: "drn_ro_",
		key: "stored-hash-must-never-leave",
		referenceId: OWNER,
		enabled: true,
		createdAt: now,
		updatedAt: now,
		expiresAt: new Date(Date.now() + 30 * DAY_SECONDS * 1000),
		lastRequest: null,
		metadata: { companyId: COMPANY_A },
		permissions: { ledger: ["read"] },
		...overrides,
	};
}

function membership(companyId: string) {
	return {
		companyId,
		companyName: "Empresa",
		ruc: "20123456789",
		countryCode: "PE",
		membershipRole: "OWNER",
		isDefault: true,
	};
}

function sessionHeadersOf(input: unknown): Headers | null {
	const headers = (input as { headers?: unknown } | undefined)?.headers;
	return headers instanceof Headers ? headers : null;
}

function requireSessionUser(input: unknown): string {
	const cookie = sessionHeadersOf(input)?.get("cookie") ?? "";
	if (!cookie.includes("better-auth.session_token") || !sessionUserId) {
		throw new APIError("UNAUTHORIZED", { message: "no session" });
	}
	return sessionUserId;
}

function request(
	path: string,
	init: {
		method?: string;
		body?: unknown;
		headers?: Record<string, string>;
		session?: boolean;
	} = {},
): Request {
	const { method = "GET", body, headers = {}, session = true } = init;
	return new Request(`http://localhost${path}`, {
		method,
		headers: {
			...(session ? { cookie: SESSION_COOKIE } : {}),
			...(body !== undefined ? { "content-type": "application/json" } : {}),
			...headers,
		},
		body: body !== undefined ? JSON.stringify(body) : undefined,
	});
}

async function readJson(res: Response): Promise<Record<string, unknown>> {
	return (await res.json()) as Record<string, unknown>;
}

describe("ledger key management routes", () => {
	const app = new Elysia().use(ledgerKeysRoutes);
	const originalEnv = { ...process.env };
	let getSessionSpy: ReturnType<typeof vi.fn>;
	let createSpy: ReturnType<typeof vi.fn>;
	let listSpy: ReturnType<typeof vi.fn>;
	let deleteSpy: ReturnType<typeof vi.fn>;
	let verifySpy: ReturnType<typeof vi.fn>;

	beforeEach(() => {
		vi.restoreAllMocks();
		vi.clearAllMocks();
		process.env = { ...originalEnv, NODE_ENV: "test" };
		delete process.env.SECURITY_ENFORCE_TEST_SESSION;
		store = [];
		sessionUserId = OWNER;
		lockCalls.length = 0;
		lockBusy = false;
		for (const name of [
			"CORS_ALLOWED_ORIGINS",
			"ALLOWED_ORIGINS",
			"FRONTEND_URL",
			"LANDING_URL",
		]) {
			delete process.env[name];
		}

		getSessionSpy = vi.fn(async (input: unknown) => {
			const cookie = sessionHeadersOf(input)?.get("cookie") ?? "";
			if (!cookie.includes("better-auth.session_token") || !sessionUserId) {
				return null;
			}
			return { session: { id: "sess-1" }, user: { id: sessionUserId } };
		});
		createSpy = vi.fn(async (input: unknown) => {
			const body = (input as { body: Record<string, unknown> }).body;
			const created = fakeKey({
				name: String(body.name),
				referenceId: String(body.userId),
				metadata: body.metadata as Record<string, unknown>,
				permissions: body.permissions as Record<string, string[]>,
				expiresAt: new Date(Date.now() + Number(body.expiresIn) * 1000),
			});
			store.push(created);
			return { ...created, key: "drn_ro_plaintextReturnedOnlyOnce0123456789" };
		});
		listSpy = vi.fn(async (input: unknown) => {
			const userId = requireSessionUser(input);
			// Keeps the stored hash on purpose: the routes must strip it even
			// if the plugin ever returned it.
			const apiKeys = store.filter((key) => key.referenceId === userId);
			return { apiKeys, total: apiKeys.length };
		});
		deleteSpy = vi.fn(async (input: unknown) => {
			const userId = requireSessionUser(input);
			const keyId = (input as { body: { keyId: string } }).body.keyId;
			const found = store.find((key) => key.id === keyId);
			if (!found || found.referenceId !== userId) {
				throw new APIError("NOT_FOUND", { message: "API Key not found" });
			}
			store = store.filter((key) => key.id !== keyId);
			return { success: true };
		});
		verifySpy = vi.fn();

		vi.spyOn(auth.api, "getSession").mockImplementation(
			getSessionSpy as unknown as typeof auth.api.getSession,
		);
		vi.spyOn(auth.api, "createApiKey").mockImplementation(
			createSpy as unknown as typeof auth.api.createApiKey,
		);
		vi.spyOn(auth.api, "listApiKeys").mockImplementation(
			listSpy as unknown as typeof auth.api.listApiKeys,
		);
		vi.spyOn(auth.api, "deleteApiKey").mockImplementation(
			deleteSpy as unknown as typeof auth.api.deleteApiKey,
		);
		vi.spyOn(auth.api, "verifyApiKey").mockImplementation(
			verifySpy as unknown as typeof auth.api.verifyApiKey,
		);
		vi.mocked(listUserCompanyMemberships).mockResolvedValue([
			membership(COMPANY_A),
		]);
	});

	afterEach(() => {
		vi.restoreAllMocks();
		process.env = { ...originalEnv };
	});

	const validBody = () => ({ companyId: COMPANY_A, name: "GPUI laptop" });

	describe("only a session may manage keys", () => {
		const ROUTES = [
			() => request("/api/ledger-keys", { method: "POST", body: validBody() }),
			() => request(`/api/ledger-keys?companyId=${COMPANY_A}`),
			() => request("/api/ledger-keys/key-1", { method: "DELETE" }),
		];

		it("refuses every route without a session (401 SESSION_REQUIRED)", async () => {
			for (const build of ROUTES) {
				const req = build();
				req.headers.delete("cookie");
				const res = await app.handle(req);
				expect(res.status, req.method).toBe(401);
				expect(await readJson(res)).toEqual({
					success: false,
					error: "A signed-in session is required to manage ledger keys",
					code: "SESSION_REQUIRED",
				});
			}
			expect(createSpy).not.toHaveBeenCalled();
			expect(listSpy).not.toHaveBeenCalled();
			expect(deleteSpy).not.toHaveBeenCalled();
		});

		it("refuses header identity without a session, in test and production", async () => {
			for (const nodeEnv of ["test", "production"]) {
				process.env.NODE_ENV = nodeEnv;
				for (const build of ROUTES) {
					const req = build();
					req.headers.delete("cookie");
					req.headers.set("x-user-id", OWNER);
					req.headers.set("x-auth-user-id", OWNER);
					req.headers.set("x-user-role", "admin");
					req.headers.set("x-company-id", COMPANY_A);
					const res = await app.handle(req);
					expect(res.status, `${nodeEnv} ${req.method}`).toBe(401);
					expect((await readJson(res)).code).toBe("SESSION_REQUIRED");
				}
			}
			expect(createSpy).not.toHaveBeenCalled();
			expect(listSpy).not.toHaveBeenCalled();
			expect(deleteSpy).not.toHaveBeenCalled();
			expect(store).toEqual([]);
		});

		it("refuses a ledger key on every route, even next to a valid session (403 API_KEY_NOT_ALLOWED)", async () => {
			const keyHeaders = [
				{ authorization: `Bearer ${PRESENTED_KEY}` },
				{ "x-api-key": PRESENTED_KEY },
			];
			for (const headers of keyHeaders) {
				for (const session of [false, true]) {
					for (const build of ROUTES) {
						const req = build();
						if (!session) req.headers.delete("cookie");
						for (const [name, value] of Object.entries(headers)) {
							req.headers.set(name, value);
						}
						const res = await app.handle(req);
						expect(res.status, `${req.method} session=${session}`).toBe(403);
						expect(await readJson(res)).toEqual({
							success: false,
							error:
								"API keys cannot manage ledger keys; sign in with a session",
							code: "API_KEY_NOT_ALLOWED",
						});
					}
				}
			}
			expect(verifySpy).not.toHaveBeenCalled();
			expect(getSessionSpy).not.toHaveBeenCalled();
			expect(createSpy).not.toHaveBeenCalled();
			expect(listSpy).not.toHaveBeenCalled();
			expect(deleteSpy).not.toHaveBeenCalled();
		});
	});

	describe("POST /api/ledger-keys — issue", () => {
		it("issues a key for an active member and returns it once", async () => {
			const res = await app.handle(
				request("/api/ledger-keys", { method: "POST", body: validBody() }),
			);
			expect(res.status).toBe(201);
			const payload = await readJson(res);
			expect(payload.success).toBe(true);
			const data = payload.data as Record<string, unknown>;
			expect(Object.keys(data).sort()).toEqual(
				["companyId", "expiresAt", "id", "key", "name", "start"].sort(),
			);
			expect(data).toMatchObject({
				key: "drn_ro_plaintextReturnedOnlyOnce0123456789",
				name: "GPUI laptop",
				companyId: COMPANY_A,
				start: "drn_ro_abcd",
			});
			expect(typeof data.expiresAt).toBe("string");
			expect(store).toHaveLength(1);
		});

		it("lets the server, never the client, decide owner, prefix, permissions and company", async () => {
			await app.handle(
				request("/api/ledger-keys", {
					method: "POST",
					body: { ...validBody(), expiresInDays: 30 },
				}),
			);
			expect(createSpy).toHaveBeenCalledTimes(1);
			const call = createSpy.mock.calls[0]?.[0] as Record<string, unknown>;
			// No request headers: the plugin treats a call with headers as a
			// client call and would refuse server-only properties.
			expect(call.headers).toBeUndefined();
			expect(call.body).toEqual({
				userId: OWNER,
				name: "GPUI laptop",
				prefix: "drn_ro_",
				permissions: { ledger: ["read"] },
				metadata: { companyId: COMPANY_A },
				expiresIn: 30 * DAY_SECONDS,
			});
		});

		it("refuses client-chosen permissions, metadata, prefix, owner or limits (400)", async () => {
			const injected: Array<Record<string, unknown>> = [
				{ permissions: { ledger: ["read", "write"] } },
				{ metadata: { companyId: COMPANY_B } },
				{ prefix: "drn_rw_" },
				{ userId: OTHER_USER },
				{ rateLimitEnabled: false },
				{ remaining: 5 },
			];
			for (const extra of injected) {
				const res = await app.handle(
					request("/api/ledger-keys", {
						method: "POST",
						body: { ...validBody(), ...extra },
					}),
				);
				expect(res.status, JSON.stringify(extra)).toBe(400);
				expect((await readJson(res)).code).toBe("VALIDATION_ERROR");
			}
			expect(createSpy).not.toHaveBeenCalled();
			expect(store).toEqual([]);
		});

		it("defaults to 90 days and accepts the 1 and 365 day boundaries", async () => {
			for (const [expiresInDays, seconds] of [
				[undefined, 90 * DAY_SECONDS],
				[1, DAY_SECONDS],
				[365, 365 * DAY_SECONDS],
			] as const) {
				createSpy.mockClear();
				const body =
					expiresInDays === undefined
						? validBody()
						: { ...validBody(), expiresInDays };
				const res = await app.handle(
					request("/api/ledger-keys", { method: "POST", body }),
				);
				expect(res.status, String(expiresInDays)).toBe(201);
				const call = createSpy.mock.calls[0]?.[0] as {
					body: { expiresIn: number };
				};
				expect(call.body.expiresIn).toBe(seconds);
			}
		});

		it("validates name, expiry and company (400 VALIDATION_ERROR, nothing issued)", async () => {
			const invalid: Array<Record<string, unknown>> = [
				{ companyId: COMPANY_A },
				{ companyId: COMPANY_A, name: "" },
				{ companyId: COMPANY_A, name: "   " },
				{ companyId: COMPANY_A, name: "x".repeat(65) },
				{ companyId: COMPANY_A, name: "bad\u0000name" },
				{ companyId: COMPANY_A, name: "tab\tname" },
				{ companyId: COMPANY_A, name: 42 },
				{ ...validBody(), expiresInDays: 0 },
				{ ...validBody(), expiresInDays: 366 },
				{ ...validBody(), expiresInDays: 1.5 },
				{ ...validBody(), expiresInDays: "30" },
				{ name: "GPUI laptop" },
				{ companyId: "not-a-uuid", name: "GPUI laptop" },
			];
			for (const body of invalid) {
				const res = await app.handle(
					request("/api/ledger-keys", { method: "POST", body }),
				);
				expect(res.status, JSON.stringify(body)).toBe(400);
				const payload = await readJson(res);
				expect(payload.success).toBe(false);
				expect(payload.code).toBe("VALIDATION_ERROR");
			}
			expect(createSpy).not.toHaveBeenCalled();
			expect(store).toEqual([]);
		});

		it("accepts a 64-character name and trims surrounding spaces", async () => {
			const name64 = "n".repeat(64);
			const res = await app.handle(
				request("/api/ledger-keys", {
					method: "POST",
					body: { companyId: COMPANY_A, name: `  ${name64}  ` },
				}),
			);
			expect(res.status).toBe(201);
			const call = createSpy.mock.calls[0]?.[0] as { body: { name: string } };
			expect(call.body.name).toBe(name64);
		});

		it("requires active membership of the company (403, nothing issued)", async () => {
			const res = await app.handle(
				request("/api/ledger-keys", {
					method: "POST",
					body: { companyId: COMPANY_B, name: "other company" },
				}),
			);
			expect(res.status).toBe(403);
			expect(await readJson(res)).toEqual({
				success: false,
				error: "You are not an active member of this company",
				code: "COMPANY_MEMBERSHIP_REQUIRED",
			});
			expect(listUserCompanyMemberships).toHaveBeenCalledWith(OWNER);
			expect(createSpy).not.toHaveBeenCalled();
			expect(store).toEqual([]);
		});

		it(`allows at most ${10} active keys per user and company (409)`, async () => {
			expect(LEDGER_KEYS_MAX_ACTIVE_PER_COMPANY).toBe(10);
			store = Array.from({ length: 10 }, () => fakeKey());
			const res = await app.handle(
				request("/api/ledger-keys", { method: "POST", body: validBody() }),
			);
			expect(res.status).toBe(409);
			expect(await readJson(res)).toEqual({
				success: false,
				error: "At most 10 active ledger keys per company; revoke one first",
				code: "LEDGER_KEY_LIMIT_REACHED",
			});
			expect(createSpy).not.toHaveBeenCalled();
			expect(store).toHaveLength(10);
		});

		it("counts and creates under the (user, company) issue lock, after the membership check", async () => {
			const res = await app.handle(
				request("/api/ledger-keys", { method: "POST", body: validBody() }),
			);
			expect(res.status).toBe(201);
			expect(lockCalls).toEqual([{ userId: OWNER, companyId: COMPANY_A }]);

			lockCalls.length = 0;
			const refused = await app.handle(
				request("/api/ledger-keys", {
					method: "POST",
					body: { companyId: COMPANY_B, name: "other company" },
				}),
			);
			expect(refused.status).toBe(403);
			expect(lockCalls).toEqual([]);
		});

		it("answers 409 LEDGER_KEY_ISSUE_BUSY when the issue lock cannot be taken in time", async () => {
			lockBusy = true;
			const res = await app.handle(
				request("/api/ledger-keys", { method: "POST", body: validBody() }),
			);
			expect(res.status).toBe(409);
			expect(await readJson(res)).toEqual({
				success: false,
				error: "Another ledger key is being issued for this company; retry",
				code: "LEDGER_KEY_ISSUE_BUSY",
			});
			expect(createSpy).not.toHaveBeenCalled();
			expect(store).toEqual([]);
		});

		it("does not count expired, disabled, other-company or other-user keys toward the cap", async () => {
			store = [
				...Array.from({ length: 9 }, () => fakeKey()),
				fakeKey({ expiresAt: new Date(Date.now() - 1000) }),
				fakeKey({ enabled: false }),
				fakeKey({ metadata: { companyId: COMPANY_B } }),
				fakeKey({ referenceId: OTHER_USER }),
				fakeKey({ prefix: "other_" }),
			];
			const res = await app.handle(
				request("/api/ledger-keys", { method: "POST", body: validBody() }),
			);
			expect(res.status).toBe(201);
			expect(createSpy).toHaveBeenCalledTimes(1);
		});
	});

	describe("GET /api/ledger-keys — list", () => {
		it("lists only the caller's own keys of that company, without secrets", async () => {
			const own = fakeKey({
				name: "own",
				lastRequest: new Date("2026-11-27T11:00:00.000Z"),
			});
			store = [
				own,
				fakeKey({ name: "other company", metadata: { companyId: COMPANY_B } }),
				fakeKey({ name: "other user", referenceId: OTHER_USER }),
				fakeKey({ name: "not a ledger key", prefix: "other_" }),
			];
			const res = await app.handle(
				request(`/api/ledger-keys?companyId=${COMPANY_A}`),
			);
			expect(res.status).toBe(200);
			expect(await readJson(res)).toEqual({
				success: true,
				data: [
					{
						id: own.id,
						name: "own",
						start: "drn_ro_abcd",
						companyId: COMPANY_A,
						createdAt: own.createdAt.toISOString(),
						expiresAt: own.expiresAt?.toISOString(),
						lastRequest: "2026-11-27T11:00:00.000Z",
						enabled: true,
					},
				],
			});
			const call = listSpy.mock.calls[0]?.[0];
			expect(sessionHeadersOf(call)?.get("cookie")).toBe(SESSION_COOKIE);
		});

		it("never returns the plaintext key after issuance", async () => {
			const issued = await app.handle(
				request("/api/ledger-keys", { method: "POST", body: validBody() }),
			);
			const plaintext = ((await readJson(issued)).data as { key: string }).key;
			const res = await app.handle(
				request(`/api/ledger-keys?companyId=${COMPANY_A}`),
			);
			const text = await res.text();
			expect(text).not.toContain(plaintext);
			expect(text).not.toContain("stored-hash-must-never-leave");
			expect(text).not.toContain('"key"');
			expect(text).not.toContain("permissions");
		});

		it("requires a valid companyId and active membership", async () => {
			const missing = await app.handle(request("/api/ledger-keys"));
			expect(missing.status).toBe(400);
			expect((await readJson(missing)).code).toBe("VALIDATION_ERROR");

			const malformed = await app.handle(
				request("/api/ledger-keys?companyId=nope"),
			);
			expect(malformed.status).toBe(400);

			const foreign = await app.handle(
				request(`/api/ledger-keys?companyId=${COMPANY_B}`),
			);
			expect(foreign.status).toBe(403);
			expect((await readJson(foreign)).code).toBe(
				"COMPANY_MEMBERSHIP_REQUIRED",
			);
			expect(listSpy).not.toHaveBeenCalled();
		});
	});

	describe("browser request rules: content type and origin", () => {
		const form = (headers: Record<string, string> = {}) =>
			new Request("http://localhost/api/ledger-keys", {
				method: "POST",
				headers: {
					cookie: SESSION_COOKIE,
					"content-type": "application/x-www-form-urlencoded",
					...headers,
				},
				body: new URLSearchParams({
					companyId: COMPANY_A,
					name: "csrf",
				}).toString(),
			});

		it("refuses a form-urlencoded or non-JSON issue with 415 and issues nothing", async () => {
			const requests = [
				form(),
				new Request("http://localhost/api/ledger-keys", {
					method: "POST",
					headers: { cookie: SESSION_COOKIE, "content-type": "text/plain" },
					body: JSON.stringify(validBody()),
				}),
				new Request("http://localhost/api/ledger-keys", {
					method: "POST",
					headers: { cookie: SESSION_COOKIE },
				}),
				new Request("http://localhost/api/ledger-keys", {
					method: "POST",
					headers: { cookie: SESSION_COOKIE },
					body: (() => {
						const data = new FormData();
						data.set("companyId", COMPANY_A);
						data.set("name", "csrf");
						return data;
					})(),
				}),
			];
			for (const req of requests) {
				const contentType = req.headers.get("content-type");
				const res = await app.handle(req);
				expect(res.status, String(contentType)).toBe(415);
				expect(res.headers.get("cache-control")).toBe("no-store");
				expect(await readJson(res)).toEqual({
					success: false,
					error: "Content-Type must be application/json",
					code: "UNSUPPORTED_MEDIA_TYPE",
				});
			}
			expect(createSpy).not.toHaveBeenCalled();
			expect(listSpy).not.toHaveBeenCalled();
			expect(store).toEqual([]);
		});

		it("accepts application/json with parameters", async () => {
			const res = await app.handle(
				new Request("http://localhost/api/ledger-keys", {
					method: "POST",
					headers: {
						cookie: SESSION_COOKIE,
						"content-type": "application/json; charset=utf-8",
					},
					body: JSON.stringify(validBody()),
				}),
			);
			expect(res.status).toBe(201);
		});

		it("refuses issue and revoke from an origin outside the CORS allowlist (403), in test and production", async () => {
			const own = fakeKey();
			for (const nodeEnv of ["test", "production"]) {
				process.env.NODE_ENV = nodeEnv;
				process.env.CORS_ALLOWED_ORIGINS = "https://app.drenyra.test";
				store = [own];
				for (const origin of [
					"https://evil.example",
					"null",
					"https://app.drenyra.test.evil.example",
					"http://app.drenyra.test",
				]) {
					const issue = await app.handle(
						request("/api/ledger-keys", {
							method: "POST",
							body: validBody(),
							headers: { origin },
						}),
					);
					const revoke = await app.handle(
						request(`/api/ledger-keys/${own.id}`, {
							method: "DELETE",
							headers: { origin },
						}),
					);
					for (const res of [issue, revoke]) {
						expect(res.status, `${nodeEnv} ${origin}`).toBe(403);
						expect(await readJson(res)).toEqual({
							success: false,
							error: "Origin is not allowed",
							code: "ORIGIN_NOT_ALLOWED",
						});
					}
				}
				expect(store).toEqual([own]);
			}
			expect(createSpy).not.toHaveBeenCalled();
			expect(deleteSpy).not.toHaveBeenCalled();
		});

		it("refuses a foreign-origin form post (the CSRF shape) before anything is issued", async () => {
			const res = await app.handle(form({ origin: "https://evil.example" }));
			expect(res.status).toBe(403);
			expect((await readJson(res)).code).toBe("ORIGIN_NOT_ALLOWED");
			expect(createSpy).not.toHaveBeenCalled();
		});

		it("lets the web app's normal requests through (allowlisted origin, JSON)", async () => {
			process.env.CORS_ALLOWED_ORIGINS =
				"https://app.drenyra.test/, https://landing.drenyra.test";
			const origin = "https://app.drenyra.test";
			const issue = await app.handle(
				request("/api/ledger-keys", {
					method: "POST",
					body: validBody(),
					headers: { origin },
				}),
			);
			expect(issue.status).toBe(201);
			const { id } = (await readJson(issue)).data as { id: string };
			const list = await app.handle(
				request(`/api/ledger-keys?companyId=${COMPANY_A}`, {
					headers: { origin },
				}),
			);
			expect(list.status).toBe(200);
			const revoke = await app.handle(
				request(`/api/ledger-keys/${id}`, {
					method: "DELETE",
					headers: { origin },
				}),
			);
			expect(revoke.status).toBe(200);
			expect(store).toEqual([]);
		});

		it("accepts the local dev origin outside production when no allowlist is configured", async () => {
			const res = await app.handle(
				request("/api/ledger-keys", {
					method: "POST",
					body: validBody(),
					headers: { origin: "http://localhost:5173" },
				}),
			);
			expect(res.status).toBe(201);
		});

		it("accepts requests without an Origin header (CLI, server-to-server)", async () => {
			process.env.NODE_ENV = "production";
			const own = fakeKey();
			store = [own];
			const issue = await app.handle(
				request("/api/ledger-keys", { method: "POST", body: validBody() }),
			);
			const revoke = await app.handle(
				request(`/api/ledger-keys/${own.id}`, { method: "DELETE" }),
			);
			expect(issue.status).toBe(201);
			expect(revoke.status).toBe(200);
		});

		it("does not apply the origin rule to listing", async () => {
			process.env.CORS_ALLOWED_ORIGINS = "https://app.drenyra.test";
			const res = await app.handle(
				request(`/api/ledger-keys?companyId=${COMPANY_A}`, {
					headers: { origin: "https://evil.example" },
				}),
			);
			expect(res.status).toBe(200);
		});
	});

	describe("caching", () => {
		it("marks issue and list responses Cache-Control: no-store", async () => {
			const issue = await app.handle(
				request("/api/ledger-keys", { method: "POST", body: validBody() }),
			);
			const list = await app.handle(
				request(`/api/ledger-keys?companyId=${COMPANY_A}`),
			);
			const refusedIssue = await app.handle(
				request("/api/ledger-keys", {
					method: "POST",
					body: { companyId: COMPANY_B, name: "x" },
				}),
			);
			expect(issue.status).toBe(201);
			expect(list.status).toBe(200);
			expect(refusedIssue.status).toBe(403);
			for (const res of [issue, list, refusedIssue]) {
				expect(res.headers.get("cache-control")).toBe("no-store");
			}
		});
	});

	describe("DELETE /api/ledger-keys/:id — revoke", () => {
		it("revokes the caller's own key, which then disappears from the list", async () => {
			const own = fakeKey();
			store = [own];
			const res = await app.handle(
				request(`/api/ledger-keys/${own.id}`, { method: "DELETE" }),
			);
			expect(res.status).toBe(200);
			expect(await readJson(res)).toEqual({
				success: true,
				data: { id: own.id, revoked: true },
			});
			const call = deleteSpy.mock.calls[0]?.[0] as {
				body: { keyId: string };
			};
			expect(call.body).toEqual({ keyId: own.id });
			expect(sessionHeadersOf(call)?.get("cookie")).toBe(SESSION_COOKIE);
			expect(store).toEqual([]);

			const list = await app.handle(
				request(`/api/ledger-keys?companyId=${COMPANY_A}`),
			);
			expect((await readJson(list)).data).toEqual([]);
		});

		it("answers 404 for another user's key and leaves it in place", async () => {
			const foreign = fakeKey({ referenceId: OTHER_USER });
			store = [foreign];
			const res = await app.handle(
				request(`/api/ledger-keys/${foreign.id}`, { method: "DELETE" }),
			);
			expect(res.status).toBe(404);
			expect(await readJson(res)).toEqual({
				success: false,
				error: "Ledger key not found",
				code: "LEDGER_KEY_NOT_FOUND",
			});
			expect(store).toEqual([foreign]);
		});

		it("answers the same 404 for an unknown id", async () => {
			const res = await app.handle(
				request("/api/ledger-keys/key-does-not-exist", { method: "DELETE" }),
			);
			expect(res.status).toBe(404);
			expect(await readJson(res)).toEqual({
				success: false,
				error: "Ledger key not found",
				code: "LEDGER_KEY_NOT_FOUND",
			});
		});

		it("answers 404 when the plugin reports NOT_FOUND (deleted concurrently)", async () => {
			const own = fakeKey();
			store = [own];
			deleteSpy.mockRejectedValueOnce(
				new APIError("NOT_FOUND", { message: "API Key not found" }),
			);
			const res = await app.handle(
				request(`/api/ledger-keys/${own.id}`, { method: "DELETE" }),
			);
			expect(res.status).toBe(404);
			expect((await readJson(res)).code).toBe("LEDGER_KEY_NOT_FOUND");
		});
	});
});
