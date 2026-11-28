/**
 * Read-only ledger keys (`drn_ro_…`) over `/api/ledger-keys`.
 *
 * Contract: `docs/04-api/ledger-read-keys.md`. Management is session-only: the
 * web app issues, lists and revokes the signed-in user's own keys for one
 * company. The plaintext key exists only in the issue response; nothing here
 * stores it.
 */

import { api } from "@/lib/api-client";
import { ApiError, extractOkData, unwrap } from "@/lib/api-helpers";

/** Mirrors the API limits in `docs/04-api/ledger-read-keys.md`. */
export const LEDGER_KEY_DEFAULT_EXPIRES_IN_DAYS = 90;
export const LEDGER_KEY_MAX_EXPIRES_IN_DAYS = 365;
export const LEDGER_KEY_NAME_MAX_LENGTH = 64;
export const LEDGER_KEYS_MAX_ACTIVE_PER_COMPANY = 10;

/** A key as listed: never carries the secret. */
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

/** Issue response: the only place where `key` (the plaintext) exists. */
export interface IssuedLedgerKey {
	id: string;
	key: string;
	start: string | null;
	name: string | null;
	companyId: string;
	expiresAt: string | null;
}

export interface IssueLedgerKeyInput {
	companyId: string;
	name: string;
	expiresInDays: number;
}

export type LedgerKeyStatus = "active" | "expired" | "disabled";

/** Eden Treaty turns ISO date strings into `Date`; accept both. */
type WireDate = string | Date | null | undefined;

interface LedgerKeySummaryDto {
	id: string;
	name?: string | null;
	start?: string | null;
	companyId: string;
	createdAt?: WireDate;
	expiresAt?: WireDate;
	lastRequest?: WireDate;
	enabled?: boolean | null;
}

interface IssuedLedgerKeyDto {
	id: string;
	key: string;
	start?: string | null;
	name?: string | null;
	companyId: string;
	expiresAt?: WireDate;
}

function isoOrNull(value: WireDate): string | null {
	if (value instanceof Date) return value.toISOString();
	return typeof value === "string" ? value : null;
}

/** Copies only the listed fields, so an unexpected secret never reaches the UI. */
function toSummary(dto: LedgerKeySummaryDto): LedgerKeySummary {
	return {
		id: dto.id,
		name: dto.name ?? null,
		start: dto.start ?? null,
		companyId: dto.companyId,
		createdAt: isoOrNull(dto.createdAt),
		expiresAt: isoOrNull(dto.expiresAt),
		lastRequest: isoOrNull(dto.lastRequest),
		enabled: dto.enabled === true,
	};
}

function ledgerKeysRoute() {
	return api.api["ledger-keys"];
}

export async function fetchLedgerKeys(
	companyId: string,
): Promise<LedgerKeySummary[]> {
	const envelope = await unwrap(
		ledgerKeysRoute().get({ query: { companyId } }),
	);
	return extractOkData<LedgerKeySummaryDto[]>(
		envelope,
		"No se pudieron cargar las llaves",
	).map(toSummary);
}

export async function issueLedgerKey(
	input: IssueLedgerKeyInput,
): Promise<IssuedLedgerKey> {
	const envelope = await unwrap(
		ledgerKeysRoute().post({
			companyId: input.companyId,
			name: input.name,
			expiresInDays: input.expiresInDays,
		}),
	);
	const dto = extractOkData<IssuedLedgerKeyDto>(
		envelope,
		"No se pudo emitir la llave",
	);
	return {
		id: dto.id,
		key: dto.key,
		start: dto.start ?? null,
		name: dto.name ?? null,
		companyId: dto.companyId,
		expiresAt: isoOrNull(dto.expiresAt),
	};
}

export async function revokeLedgerKey(id: string): Promise<void> {
	const envelope = await unwrap(ledgerKeysRoute()({ id }).delete());
	extractOkData(envelope, "No se pudo revocar la llave");
}

export function ledgerKeyStatus(
	key: Pick<LedgerKeySummary, "enabled" | "expiresAt">,
	now: Date,
): LedgerKeyStatus {
	if (!key.enabled) return "disabled";
	if (key.expiresAt && Date.parse(key.expiresAt) <= now.getTime()) {
		return "expired";
	}
	return "active";
}

const ERROR_MESSAGES: Record<string, string> = {
	LEDGER_KEY_LIMIT_REACHED: `Ya tienes ${LEDGER_KEYS_MAX_ACTIVE_PER_COMPANY} llaves activas para esta empresa. Revoca una antes de emitir otra.`,
	COMPANY_MEMBERSHIP_REQUIRED:
		"No eres miembro activo de esta empresa, así que no puedes gestionar sus llaves.",
	LEDGER_KEY_NOT_FOUND:
		"La llave ya no existe o no es tuya. Actualiza la lista.",
	SESSION_REQUIRED: "Tu sesión expiró. Vuelve a iniciar sesión.",
	API_KEY_NOT_ALLOWED:
		"Las llaves solo se gestionan con tu sesión web, no con otra llave.",
};

/** API failure → message for the user, without exposing anything secret. */
export function describeLedgerKeyError(
	error: unknown,
	fallback: string,
): string {
	if (!(error instanceof ApiError)) return fallback;
	if (error.code === "VALIDATION_ERROR") {
		return `Datos inválidos: ${error.message}`;
	}
	return (error.code && ERROR_MESSAGES[error.code]) || fallback;
}

export interface IssueLedgerKeyFormValues {
	name: string;
	expiresInDays: string;
}

export type IssueLedgerKeyFormResult =
	| { ok: true; value: { name: string; expiresInDays: number } }
	| {
			ok: false;
			errors: Partial<Record<keyof IssueLedgerKeyFormValues, string>>;
	  };

const WHOLE_NUMBER = /^\d+$/;

/** Same rules as the API, checked before the request. */
export function validateIssueLedgerKeyForm(
	values: IssueLedgerKeyFormValues,
): IssueLedgerKeyFormResult {
	const errors: Partial<Record<keyof IssueLedgerKeyFormValues, string>> = {};
	const name = values.name.trim();
	if (name.length === 0) {
		errors.name = "Escribe un nombre para reconocer la llave.";
	} else if (name.length > LEDGER_KEY_NAME_MAX_LENGTH) {
		errors.name = `El nombre admite como máximo ${LEDGER_KEY_NAME_MAX_LENGTH} caracteres.`;
	}

	const rawDays = values.expiresInDays.trim();
	const days = WHOLE_NUMBER.test(rawDays) ? Number(rawDays) : Number.NaN;
	if (!(days >= 1 && days <= LEDGER_KEY_MAX_EXPIRES_IN_DAYS)) {
		errors.expiresInDays = `La vigencia debe ser un número entero entre 1 y ${LEDGER_KEY_MAX_EXPIRES_IN_DAYS} días.`;
	}

	if (errors.name || errors.expiresInDays) return { ok: false, errors };
	return { ok: true, value: { name, expiresInDays: days } };
}
