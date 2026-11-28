import { isAPIError } from "better-auth/api";
import { createLogger } from "../../lib/logger";
import { auth } from "../auth/auth.config";
import { listUserCompanyMemberships } from "../auth/handlers/company-membership";
import {
	READ_ONLY_LEDGER_KEY_PREFIX,
	readOnlyLedgerKeyPermissions,
} from "../auth/lib/read-only-ledger-key";

/**
 * Ledger key management — issue, list and revoke the caller's own read-only
 * ledger keys for one company.
 *
 * Every operation goes through the Better Auth api-key plugin (`auth.api.*`),
 * which owns hashing, storage and ownership checks:
 * - Issue calls `createApiKey` server-side (no request headers) so the server
 *   alone sets owner, prefix, `ledger:read` and the company binding.
 * - List and revoke call `listApiKeys` / `deleteApiKey` with the caller's
 *   session headers, so the plugin itself only ever sees the session user's
 *   keys and answers NOT_FOUND for anyone else's.
 *
 * The plaintext key exists only in the issue response. Logs carry ids, never
 * the key or its hash.
 *
 * Known limit: the 10-key cap is checked before creating, so two concurrent
 * issue requests can end one key above the cap.
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

const NOT_FOUND: LedgerKeyFailure = {
	ok: false,
	status: 404,
	code: "LEDGER_KEY_NOT_FOUND",
	error: "Ledger key not found",
};

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

/**
 * The session user's read-only ledger keys, as the plugin stores them.
 * `sessionHeaders` must carry the caller's Better Auth session.
 */
async function listOwnLedgerKeyRecords(
	sessionHeaders: Headers,
): Promise<PluginKeyRecord[]> {
	const result: unknown = await auth.api.listApiKeys({
		headers: sessionHeaders,
	});
	const apiKeys = (result as { apiKeys?: unknown } | null)?.apiKeys;
	if (!Array.isArray(apiKeys)) return [];
	return apiKeys
		.map(toPluginKeyRecord)
		.filter(
			(record): record is PluginKeyRecord =>
				record !== null && record.prefix === READ_ONLY_LEDGER_KEY_PREFIX,
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
 * (409) when the user already holds the maximum of active keys there.
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
 * key and an unknown id get the same 404, so existence never leaks.
 * Membership is not required: a former member may still revoke their keys.
 */
export async function revokeLedgerKey(input: {
	userId: string;
	keyId: string;
	sessionHeaders: Headers;
}): Promise<{ ok: true; id: string } | LedgerKeyFailure> {
	const own = (await listOwnLedgerKeyRecords(input.sessionHeaders)).some(
		(record) => record.id === input.keyId,
	);
	if (!own) return NOT_FOUND;

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
