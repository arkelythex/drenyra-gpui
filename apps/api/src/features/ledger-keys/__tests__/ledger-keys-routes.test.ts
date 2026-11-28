/**
 * Ledger key management routes (`/api/ledger-keys`).
 *
 * Issue, list and revoke the caller's own read-only ledger keys for one
 * company. Only a Better Auth session may call these routes: a ledger key,
 * header identity or no identity are refused before anything is read or
 * written. `auth.api.*` and the membership lookup are mocked; the plugin's
 * list/get/delete semantics (own keys only, someone else's key is NOT_FOUND)
 * are modelled by an in-memory store. The real plugin round trip is covered by
 * `integration/ledger-keys.integration.test.ts` (RUN_DB_TESTS=1).
 */
import { randomUUID } from "node:crypto";
import { APIError } from "better-auth/api";
import { Elysia } from "elysia";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

type LogEntry = {
	bindings: Record<string, unknown>;
	level: string;
	payload: unknown;
};
const logEntries = vi.hoisted((): LogEntry[] => []);
const RECORDED_LEVELS = new Set(["debug", "info", "warn", "error"]);

vi.mock("../../../lib/logger", async (importOriginal) => {
	const actual = await importOriginal<typeof import("../../../lib/logger")>();
	return {
		...actual,
		createLogger: (bindings: Record<string, unknown>) =>
			new Proxy(actual.createLogger(bindings), {
				get(target, property, receiver) {
					if (typeof property === "string" && RECORDED_LEVELS.has(property)) {
						return (payload: unknown) => {
							logEntries.push({ bindings, level: property, payload });
						};
					}
					return Reflect.get(target, property, receiver);
				},
			}),
	};
});

// Lets a test make the session lookup itself throw; otherwise the real
// resolver runs (backed by the mocked `auth.api.getSession`).
const sessionLookup = vi.hoisted(() => ({ failure: null as Error | null }));
vi.mock("../../auth/handlers/session-identity", async (importOriginal) => {
	const actual =
		await importOriginal<
			typeof import("../../auth/handlers/session-identity")
		>();
	return {
		...actual,
		resolveSessionIdentityFromHeaders: async (
			headers: Parameters<typeof actual.resolveSessionIdentityFromHeaders>[0],
		) => {
			if (sessionLookup.failure) throw sessionLookup.failure;
			return actual.resolveSessionIdentityFromHeaders(headers);
		},
	};
});

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
	let getSpy: ReturnType<typeof vi.fn>;
	let deleteSpy: ReturnType<typeof vi.fn>;
	let verifySpy: ReturnType<typeof vi.fn>;

	beforeEach(() => {
		vi.restoreAllMocks();
		vi.clearAllMocks();
		process.env = { ...originalEnv, NODE_ENV: "test" };
		delete process.env.SECURITY_ENFORCE_TEST_SESSION;
		store = [];
		sessionUserId = OWNER;
		sessionLookup.failure = null;
		logEntries.length = 0;
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
		getSpy = vi.fn(async (input: unknown) => {
			const userId = requireSessionUser(input);
			const id = (input as { query: { id: string } }).query.id;
			const found = store.find((key) => key.id === id);
			if (!found || found.referenceId !== userId) {
				throw new APIError("NOT_FOUND", { message: "API Key not found" });
			}
			const { key: _hash, ...returned } = found;
			return returned;
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
		vi.spyOn(auth.api, "getApiKey").mockImplementation(
			getSpy as unknown as typeof auth.api.getApiKey,
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

		it("still refuses any x-api-key value, even one without the drn_ro_ prefix", async () => {
			for (const build of ROUTES) {
				const req = build();
				req.headers.set("x-api-key", "sk_live_unrelated");
				const res = await app.handle(req);
				expect(res.status, req.method).toBe(403);
				expect((await readJson(res)).code).toBe("API_KEY_NOT_ALLOWED");
			}
			expect(createSpy).not.toHaveBeenCalled();
			expect(listSpy).not.toHaveBeenCalled();
			expect(deleteSpy).not.toHaveBeenCalled();
		});

		it("answers a failing session lookup with the LEDGER_KEY_ERROR envelope on every route", async () => {
			sessionLookup.failure = new Error("session store unreachable");
			for (const build of ROUTES) {
				const req = build();
				const res = await app.handle(req);
				expect(res.status, req.method).toBe(500);
				expect(await readJson(res)).toEqual({
					success: false,
					error: "Could not complete the ledger key operation",
					code: "LEDGER_KEY_ERROR",
				});
				if (req.method !== "DELETE") {
					expect(res.headers.get("cache-control"), req.method).toBe("no-store");
				}
			}
			expect(
				logEntries
					.filter((entry) => entry.level === "error")
					.map((entry) => entry.payload),
			).toEqual([
				{ operation: "issue", errorName: "Error" },
				{ operation: "list", errorName: "Error" },
				{ operation: "revoke", errorName: "Error" },
			]);
			expect(createSpy).not.toHaveBeenCalled();
			expect(listSpy).not.toHaveBeenCalled();
			expect(deleteSpy).not.toHaveBeenCalled();
		});
	});

	describe("listing the plugin's keys", () => {
		/** A plugin that honours `limit`/`offset` with pages of `pageSize`. */
		function pagedList(pageSize: number) {
			return async (input: unknown) => {
				const userId = requireSessionUser(input);
				const query =
					(input as { query?: { limit?: number; offset?: number } }).query ??
					{};
				const own = store.filter((key) => key.referenceId === userId);
				const offset = query.offset ?? 0;
				const limit = Math.min(query.limit ?? own.length, pageSize);
				return {
					apiKeys: own.slice(offset, offset + limit),
					total: own.length,
					limit,
					offset,
				};
			};
		}

		it("pages through listApiKeys until total is reached for the list", async () => {
			const keys = Array.from({ length: 5 }, (_, index) =>
				fakeKey({ name: `key ${index}` }),
			);
			store = [...keys];
			listSpy.mockImplementation(pagedList(2));

			const list = await app.handle(
				request(`/api/ledger-keys?companyId=${COMPANY_A}`),
			);
			expect(list.status).toBe(200);
			expect(
				((await readJson(list)).data as Array<{ id: string }>).map(
					(key) => key.id,
				),
			).toEqual(keys.map((key) => key.id));
			expect(
				listSpy.mock.calls.map(
					(call) => (call[0] as { query?: { offset?: number } }).query?.offset,
				),
			).toEqual([0, 2, 4]);
		});

		it("counts keys beyond the first page toward the cap (409, nothing issued)", async () => {
			store = Array.from({ length: LEDGER_KEYS_MAX_ACTIVE_PER_COMPANY }, () =>
				fakeKey(),
			);
			listSpy.mockImplementation(pagedList(3));

			const res = await app.handle(
				request("/api/ledger-keys", { method: "POST", body: validBody() }),
			);
			expect(res.status).toBe(409);
			expect((await readJson(res)).code).toBe("LEDGER_KEY_LIMIT_REACHED");
			expect(createSpy).not.toHaveBeenCalled();
		});

		const UNEXPECTED_RESPONSES: ReadonlyArray<[string, unknown]> = [
			["null", null],
			["no apiKeys", { total: 0 }],
			["apiKeys not an array", { apiKeys: "nope", total: 1 }],
			["no total", { apiKeys: [] }],
			["total not a number", { apiKeys: [], total: "1" }],
			["negative total", { apiKeys: [], total: -1 }],
			[
				"ledger key record without id",
				{ apiKeys: [{ name: "x", prefix: "drn_ro_" }], total: 1 },
			],
			["empty page before total", { apiKeys: [], total: 3 }],
		];

		it.each(UNEXPECTED_RESPONSES)(
			"answers 500 LEDGER_KEY_ERROR to list and issue, and logs, when listApiKeys returns %s",
			async (_label, response) => {
				const own = fakeKey();
				store = [own];
				listSpy.mockResolvedValue(response);

				const responses = [
					await app.handle(request(`/api/ledger-keys?companyId=${COMPANY_A}`)),
					await app.handle(
						request("/api/ledger-keys", { method: "POST", body: validBody() }),
					),
				];
				for (const res of responses) {
					expect(res.status).toBe(500);
					expect(await readJson(res)).toEqual({
						success: false,
						error: "Could not complete the ledger key operation",
						code: "LEDGER_KEY_ERROR",
					});
				}
				expect(createSpy).not.toHaveBeenCalled();
				expect(deleteSpy).not.toHaveBeenCalled();
				expect(store).toEqual([own]);
				expect(
					logEntries
						.filter((entry) => entry.level === "error")
						.map((entry) => entry.payload),
				).toEqual([
					{ operation: "list", errorName: "LedgerKeyListError" },
					{ operation: "issue", errorName: "LedgerKeyListError" },
				]);
			},
		);

		it("skips keys of other configurations (null prefix, no string id) instead of failing", async () => {
			const own = fakeKey({ name: "own" });
			const foreignConfig = {
				configId: "partner-integrations",
				prefix: null,
				name: "partner",
				referenceId: OWNER,
				enabled: true,
				metadata: { companyId: COMPANY_A },
			};
			const malformedForeign = { id: 42, prefix: "sk_live_", enabled: true };
			listSpy.mockResolvedValue({
				apiKeys: [foreignConfig, own, malformedForeign, null],
				total: 4,
			});
			store = [own];

			const list = await app.handle(
				request(`/api/ledger-keys?companyId=${COMPANY_A}`),
			);
			expect(list.status).toBe(200);
			expect(
				((await readJson(list)).data as Array<{ id: string }>).map(
					(key) => key.id,
				),
			).toEqual([own.id]);

			const issue = await app.handle(
				request("/api/ledger-keys", { method: "POST", body: validBody() }),
			);
			expect(issue.status).toBe(201);
			expect(createSpy).toHaveBeenCalledTimes(1);
			expect(logEntries.filter((entry) => entry.level === "error")).toEqual([]);
		});

		it("answers 500 instead of a silently truncated list when the plugin may have cut it at 100 rows", async () => {
			store = Array.from({ length: 100 }, (_, index) =>
				fakeKey({
					metadata: { companyId: index < 5 ? COMPANY_A : randomUUID() },
				}),
			);

			const list = await app.handle(
				request(`/api/ledger-keys?companyId=${COMPANY_A}`),
			);
			const issue = await app.handle(
				request("/api/ledger-keys", { method: "POST", body: validBody() }),
			);
			expect(list.status).toBe(500);
			expect(issue.status).toBe(500);
			expect((await readJson(list)).code).toBe("LEDGER_KEY_ERROR");
			expect(createSpy).not.toHaveBeenCalled();
		});

		it("still revokes a key when the caller holds more keys than the plugin can list", async () => {
			store = Array.from({ length: 150 }, () => fakeKey());
			const leaked = store[149] as FakeKey;

			const res = await app.handle(
				request(`/api/ledger-keys/${leaked.id}`, { method: "DELETE" }),
			);
			expect(res.status).toBe(200);
			expect(await readJson(res)).toEqual({
				success: true,
				data: { id: leaked.id, revoked: true },
			});
			expect(listSpy).not.toHaveBeenCalled();
			expect(getSpy).toHaveBeenCalledTimes(1);
			const call = getSpy.mock.calls[0]?.[0] as { query: { id: string } };
			expect(call.query).toEqual({ id: leaked.id });
			expect(sessionHeadersOf(call)?.get("cookie")).toBe(SESSION_COOKIE);
			expect(store).toHaveLength(149);
			expect(store.some((key) => key.id === leaked.id)).toBe(false);
		});

		it("still lists 99 keys in one page", async () => {
			store = Array.from({ length: 99 }, () => fakeKey());
			const list = await app.handle(
				request(`/api/ledger-keys?companyId=${COMPANY_A}`),
			);
			expect(list.status).toBe(200);
			expect((await readJson(list)).data).toHaveLength(99);
			expect(listSpy).toHaveBeenCalledTimes(1);
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

		it("answers byte-identical 404s for another user's key and an unknown id, deleting nothing", async () => {
			const foreign = fakeKey({ referenceId: OTHER_USER });
			store = [...Array.from({ length: 150 }, () => fakeKey()), foreign];
			const before = [...store];

			const foreignRes = await app.handle(
				request(`/api/ledger-keys/${foreign.id}`, { method: "DELETE" }),
			);
			const unknownRes = await app.handle(
				request("/api/ledger-keys/key-does-not-exist", { method: "DELETE" }),
			);
			expect(foreignRes.status).toBe(404);
			expect(unknownRes.status).toBe(404);
			expect([...foreignRes.headers.entries()]).toEqual([
				...unknownRes.headers.entries(),
			]);
			const foreignText = await foreignRes.text();
			expect(foreignText).toBe(await unknownRes.text());
			expect(JSON.parse(foreignText)).toEqual({
				success: false,
				error: "Ledger key not found",
				code: "LEDGER_KEY_NOT_FOUND",
			});
			expect(listSpy).not.toHaveBeenCalled();
			expect(deleteSpy).not.toHaveBeenCalled();
			expect(store).toEqual(before);
		});

		it("answers the same 404 for an own key of another configuration (no drn_ro_ prefix) and keeps it", async () => {
			const unknown = await app.handle(
				request("/api/ledger-keys/key-does-not-exist", { method: "DELETE" }),
			);
			const unknownText = await unknown.text();
			for (const prefix of ["sk_live_", null]) {
				const other = fakeKey({ prefix });
				store = [other];
				const res = await app.handle(
					request(`/api/ledger-keys/${other.id}`, { method: "DELETE" }),
				);
				expect(res.status, String(prefix)).toBe(404);
				expect(await res.text()).toBe(unknownText);
				expect(store).toEqual([other]);
			}
			expect(deleteSpy).not.toHaveBeenCalled();
		});

		it("answers the same 404 if the plugin ever returns a key owned by someone else", async () => {
			const foreign = fakeKey({ referenceId: OTHER_USER });
			store = [foreign];
			const { key: _hash, ...returned } = foreign;
			getSpy.mockResolvedValueOnce(returned);
			const res = await app.handle(
				request(`/api/ledger-keys/${foreign.id}`, { method: "DELETE" }),
			);
			expect(res.status).toBe(404);
			expect((await readJson(res)).code).toBe("LEDGER_KEY_NOT_FOUND");
			expect(deleteSpy).not.toHaveBeenCalled();
			expect(store).toEqual([foreign]);
		});

		it("answers 500 LEDGER_KEY_ERROR and deletes nothing when the key lookup fails", async () => {
			const own = fakeKey();
			store = [own];
			getSpy.mockRejectedValueOnce(new Error("database unreachable"));
			const res = await app.handle(
				request(`/api/ledger-keys/${own.id}`, { method: "DELETE" }),
			);
			expect(res.status).toBe(500);
			expect((await readJson(res)).code).toBe("LEDGER_KEY_ERROR");
			expect(deleteSpy).not.toHaveBeenCalled();
			expect(store).toEqual([own]);
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
