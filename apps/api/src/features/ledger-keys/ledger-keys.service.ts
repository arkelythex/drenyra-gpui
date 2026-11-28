import { isAPIError } from "better-auth/api";
import { createLogger } from "../../lib/logger";
import { auth } from "../auth/auth.config";
import { listUserCompanyMemberships } from "../auth/handlers/company-membership";
import {
	READ_ONLY_LEDGER_KEY_PREFIX,
	readOnlyLedgerKeyPermissions,
} from "../auth/lib/read-only-ledger-key";
import {
	LedgerKeyIssueLockTimeoutError,
	withLedgerKeyIssueLock,
} from "./ledger-keys-issue-lock";

/**
 * Ledger key management — issue, list and revoke the caller's own read-only
 * ledger keys for one company.
 *
 * Every operation goes through the Better Auth api-key plugin (`auth.api.*`),
 * which owns hashing, storage and ownership checks:
 * - Issue calls `createApiKey` server-side (no request headers) so the server
 *   alone sets owner, prefix, `ledger:read` and the company binding.
 * - List, revoke and its ownership check call `listApiKeys` / `getApiKey` /
 *   `deleteApiKey` with the caller's session headers, so the plugin itself
 *   only ever sees the session user's keys and answers NOT_FOUND for anyone
 *   else's.
 *
 * The plaintext key exists only in the issue response. Logs carry ids, never
 * the key or its hash.
 *
 * The 10-key cap holds under concurrency: counting and creating run under a
 * per (user, company) Postgres advisory lock (`ledger-keys-issue-lock.ts`).
 *
 * Listing the caller's keys (for the cap and the list) pages through
 * `listApiKeys` until its `total`, and throws `LedgerKeyListError` (the
 * routes answer 500 `LEDGER_KEY_ERROR`) on any unexpected response instead of
 * acting on a partial or empty list. Rows without the `drn_ro_` prefix (keys
 * of other configurations) are skipped before any validation, so they never
 * lock the user out. Revoke never lists: it looks up exactly that key, so it
 * keeps working however many keys the user holds.
 *
 * @module ledger-keys/service
 */

const logger = createLogger({ feature: "ledger-keys", handler: "service" });

/** Most active (enabled, unexpired) keys one user may hold per company. */
export const LEDGER_KEYS_MAX_ACTIVE_PER_COMPANY = 10;

const SECONDS_PER_DAY = 24 * 60 * 60;

/** What the issue response returns; the only place `key` ever appears. */
export interface IssuedLedgerKey {
	id: string;
	key: string;
	start: string | null;
	name: string | null;
	companyId: string;
	expiresAt: string | null;
}

/** A listed key: no plaintext, no hash, no permissions, no metadata. */
export interface LedgerKeySummary {
	id: string;
	name: string | null;
	start: string | null;
	companyId: string;
	createdAt: string | null;
	expiresAt: string | null;
	lastRequest: string | null;
	enabled: boolean;
}

export type LedgerKeyFailure = {
	ok: false;
	status: 403 | 404 | 409;
	code: string;
	error: string;
};

const MEMBERSHIP_REQUIRED: LedgerKeyFailure = {
	ok: false,
	status: 403,
	code: "COMPANY_MEMBERSHIP_REQUIRED",
	error: "You are not an active member of this company",
};

const LIMIT_REACHED: LedgerKeyFailure = {
	ok: false,
	status: 409,
	code: "LEDGER_KEY_LIMIT_REACHED",
	error: `At most ${LEDGER_KEYS_MAX_ACTIVE_PER_COMPANY} active ledger keys per company; revoke one first`,
};

const ISSUE_BUSY: LedgerKeyFailure = {
	ok: false,
	status: 409,
	code: "LEDGER_KEY_ISSUE_BUSY",
	error: "Another ledger key is being issued for this company; retry",
};

const NOT_FOUND: LedgerKeyFailure = {
	ok: false,
	status: 404,
	code: "LEDGER_KEY_NOT_FOUND",
	error: "Ledger key not found",
};

/**
 * Rows `@better-auth/api-key@1.6.23` can return from one `listApiKeys` call:
 * its database read passes no limit, so the adapter's `defaultFindManyLimit`
 * (100) applies, and `total` counts only the rows read (`limit`/`offset` then
 * slice those rows in memory). A `total` at this bound may hide more keys.
 */
export const LEDGER_KEY_LIST_PLUGIN_READ_LIMIT = 100;

/** Page size requested from `listApiKeys`. */
const LEDGER_KEY_LIST_PAGE_SIZE = 100;

/**
 * `listApiKeys` answered something this module cannot trust to be the
 * caller's complete key list. The message names the problem only.
 */
export class LedgerKeyListError extends Error {
	constructor(message: string) {
		super(message);
		this.name = "LedgerKeyListError";
	}
}

/** Fields this module reads from a plugin key record. */
interface PluginKeyRecord {
	id: string;
	name: string | null;
	start: string | null;
	prefix: string | null;
	enabled: boolean;
	createdAt: string | null;
	expiresAt: string | null;
	lastRequest: string | null;
	companyId: string | null;
}

function toIsoOrNull(value: unknown): string | null {
	if (value instanceof Date) {
		return Number.isNaN(value.getTime()) ? null : value.toISOString();
	}
	if (typeof value === "string" && value.length > 0) {
		const parsed = new Date(value);
		return Number.isNaN(parsed.getTime()) ? null : parsed.toISOString();
	}
	return null;
}

function stringOrNull(value: unknown): string | null {
	return typeof value === "string" ? value : null;
}

function readMetadataCompanyId(metadata: unknown): string | null {
	if (!metadata || typeof metadata !== "object") return null;
	const companyId = (metadata as { companyId?: unknown }).companyId;
	return typeof companyId === "string" && companyId.length > 0
		? companyId
		: null;
}

function toPluginKeyRecord(value: unknown): PluginKeyRecord | null {
	if (!value || typeof value !== "object") return null;
	const record = value as Record<string, unknown>;
	if (typeof record.id !== "string" || record.id.length === 0) return null;
	return {
		id: record.id,
		name: stringOrNull(record.name),
		start: stringOrNull(record.start),
		prefix: stringOrNull(record.prefix),
		enabled: record.enabled === true,
		createdAt: toIsoOrNull(record.createdAt),
		expiresAt: toIsoOrNull(record.expiresAt),
		lastRequest: toIsoOrNull(record.lastRequest),
		companyId: readMetadataCompanyId(record.metadata),
	};
}

function isActive(record: PluginKeyRecord, now: number): boolean {
	if (!record.enabled) return false;
	if (record.expiresAt === null) return true;
	return new Date(record.expiresAt).getTime() > now;
}

async function isActiveMember(
	userId: string,
	companyId: string,
): Promise<boolean> {
	const memberships = await listUserCompanyMemberships(userId);
	return memberships.some((membership) => membership.companyId === companyId);
}

/** One `listApiKeys` page, checked: an array of keys and a whole `total`. */
function readListPage(result: unknown): {
	apiKeys: unknown[];
	total: number;
} {
	if (!result || typeof result !== "object") {
		throw new LedgerKeyListError("listApiKeys returned no object");
	}
	const { apiKeys, total } = result as { apiKeys?: unknown; total?: unknown };
	if (!Array.isArray(apiKeys)) {
		throw new LedgerKeyListError("listApiKeys returned no apiKeys array");
	}
	if (typeof total !== "number" || !Number.isInteger(total) || total < 0) {
		throw new LedgerKeyListError("listApiKeys returned no valid total");
	}
	return { apiKeys, total };
}

/** Whether a raw `listApiKeys` row carries the read-only ledger key prefix. */
function hasLedgerKeyPrefix(row: unknown): boolean {
	return (
		typeof row === "object" &&
		row !== null &&
		(row as { prefix?: unknown }).prefix === READ_ONLY_LEDGER_KEY_PREFIX
	);
}

/**
 * The session user's read-only ledger keys, as the plugin stores them.
 * `sessionHeaders` must carry the caller's Better Auth session.
 *
 * Pages through `listApiKeys` until `total` keys are read, then keeps only
 * rows with the `drn_ro_` prefix; other rows are skipped unvalidated. Throws
 * `LedgerKeyListError` on an unexpected response (no array, no valid total,
 * a ledger key without id, an empty page before `total`) and when `total`
 * reaches `LEDGER_KEY_LIST_PLUGIN_READ_LIMIT`, where the plugin may have cut
 * the list.
 */
async function listOwnLedgerKeyRecords(
	sessionHeaders: Headers,
): Promise<PluginKeyRecord[]> {
	const rows: unknown[] = [];
	let total = 0;
	do {
		const page = readListPage(
			await auth.api.listApiKeys({
				headers: sessionHeaders,
				query: { limit: LEDGER_KEY_LIST_PAGE_SIZE, offset: rows.length },
			}),
		);
		total = page.total;
		if (total >= LEDGER_KEY_LIST_PLUGIN_READ_LIMIT) {
			throw new LedgerKeyListError(
				"listApiKeys total reached the plugin read limit; the list may be incomplete",
			);
		}
		if (page.apiKeys.length === 0 && rows.length < total) {
			throw new LedgerKeyListError("listApiKeys returned an empty page");
		}
		rows.push(...page.apiKeys);
	} while (rows.length < total);

	const records: PluginKeyRecord[] = [];
	for (const row of rows) {
		if (!hasLedgerKeyPrefix(row)) continue;
		const record = toPluginKeyRecord(row);
		if (record === null) {
			throw new LedgerKeyListError(
				"listApiKeys returned a ledger key without id",
			);
		}
		records.push(record);
	}
	return records;
}

/**
 * Whether `keyId` is one of the session user's read-only ledger keys.
 *
 * Looks up exactly that key with the plugin's `getApiKey`, called with the
 * caller's session headers so the plugin itself answers NOT_FOUND for a key
 * of another user (and for an unknown id). The record must also name the
 * session user as owner and carry the `drn_ro_` prefix. Any other plugin
 * failure is rethrown, so nothing is deleted on doubt.
 */
async function isOwnLedgerKey(input: {
	userId: string;
	keyId: string;
	sessionHeaders: Headers;
}): Promise<boolean> {
	let found: unknown;
	try {
		found = await auth.api.getApiKey({
			query: { id: input.keyId },
			headers: input.sessionHeaders,
		});
	} catch (error) {
		if (isAPIError(error) && error.statusCode === 404) return false;
		throw error;
	}
	if (!found || typeof found !== "object") return false;
	const record = found as Record<string, unknown>;
	return (
		record.id === input.keyId &&
		record.referenceId === input.userId &&
		record.prefix === READ_ONLY_LEDGER_KEY_PREFIX
	);
}

function toSummary(
	record: PluginKeyRecord & { companyId: string },
): LedgerKeySummary {
	return {
		id: record.id,
		name: record.name,
		start: record.start,
		companyId: record.companyId,
		createdAt: record.createdAt,
		expiresAt: record.expiresAt,
		lastRequest: record.lastRequest,
		enabled: record.enabled,
	};
}

/**
 * Issues a read-only ledger key for `companyId`, owned by `userId`.
 *
 * Refuses (403) when the user is not an active member of the company and
 * (409) when the user already holds the maximum of active keys there, or when
 * another issue for the same user and company holds the lock past its
 * timeout.
 */
export async function issueLedgerKey(input: {
	userId: string;
	companyId: string;
	name: string;
	expiresInDays: number;
	sessionHeaders: Headers;
}): Promise<{ ok: true; key: IssuedLedgerKey } | LedgerKeyFailure> {
	if (!(await isActiveMember(input.userId, input.companyId))) {
		return MEMBERSHIP_REQUIRED;
	}

	try {
		return await withLedgerKeyIssueLock(
			{ userId: input.userId, companyId: input.companyId },
			() => countAndCreate(input),
		);
	} catch (error) {
		if (error instanceof LedgerKeyIssueLockTimeoutError) {
			logger.warn(
				{ userId: input.userId, companyId: input.companyId },
				"Ledger key issue busy: lock not taken in time",
			);
			return ISSUE_BUSY;
		}
		throw error;
	}
}

/** Cap check and creation; callers must hold the issue lock. */
async function countAndCreate(input: {
	userId: string;
	companyId: string;
	name: string;
	expiresInDays: number;
	sessionHeaders: Headers;
}): Promise<{ ok: true; key: IssuedLedgerKey } | LedgerKeyFailure> {
	const now = Date.now();
	const activeForCompany = (
		await listOwnLedgerKeyRecords(input.sessionHeaders)
	).filter(
		(record) => record.companyId === input.companyId && isActive(record, now),
	);
	if (activeForCompany.length >= LEDGER_KEYS_MAX_ACTIVE_PER_COMPANY) {
		return LIMIT_REACHED;
	}

	// No headers on purpose: with headers the plugin treats the call as a
	// client request and refuses server-only properties such as permissions.
	const created = await auth.api.createApiKey({
		body: {
			userId: input.userId,
			name: input.name,
			prefix: READ_ONLY_LEDGER_KEY_PREFIX,
			permissions: readOnlyLedgerKeyPermissions(),
			metadata: { companyId: input.companyId },
			expiresIn: input.expiresInDays * SECONDS_PER_DAY,
		},
	});

	const issued: IssuedLedgerKey = {
		id: created.id,
		key: created.key,
		start: stringOrNull(created.start),
		name: stringOrNull(created.name),
		companyId: input.companyId,
		expiresAt: toIsoOrNull(created.expiresAt),
	};
	logger.info(
		{
			userId: input.userId,
			companyId: input.companyId,
			keyId: issued.id,
			expiresAt: issued.expiresAt,
		},
		"Read-only ledger key issued",
	);
	return { ok: true, key: issued };
}

/**
 * Lists the session user's ledger keys for `companyId`, without secrets.
 * Refuses (403) when the user is not an active member of the company.
 */
export async function listLedgerKeys(input: {
	userId: string;
	companyId: string;
	sessionHeaders: Headers;
}): Promise<{ ok: true; keys: LedgerKeySummary[] } | LedgerKeyFailure> {
	if (!(await isActiveMember(input.userId, input.companyId))) {
		return MEMBERSHIP_REQUIRED;
	}

	const keys = (await listOwnLedgerKeyRecords(input.sessionHeaders))
		.filter(
			(record): record is PluginKeyRecord & { companyId: string } =>
				record.companyId === input.companyId,
		)
		.map(toSummary);
	return { ok: true, keys };
}

/**
 * Revokes (deletes) one of the session user's ledger keys. Someone else's
 * key, a key of another configuration and an unknown id get the same 404, so
 * existence never leaks. Ownership is checked on that key alone, never on the
 * full list, so a user with many keys can always revoke a leaked one.
 * Membership is not required: a former member may still revoke their keys.
 */
export async function revokeLedgerKey(input: {
	userId: string;
	keyId: string;
	sessionHeaders: Headers;
}): Promise<{ ok: true; id: string } | LedgerKeyFailure> {
	if (!(await isOwnLedgerKey(input))) return NOT_FOUND;

	try {
		await auth.api.deleteApiKey({
			body: { keyId: input.keyId },
			headers: input.sessionHeaders,
		});
	} catch (error) {
		if (isAPIError(error) && error.statusCode === 404) return NOT_FOUND;
		throw error;
	}

	logger.info(
		{ userId: input.userId, keyId: input.keyId },
		"Read-only ledger key revoked",
	);
	return { ok: true, id: input.keyId };
}
