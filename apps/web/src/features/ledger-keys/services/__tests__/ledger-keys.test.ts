import { beforeEach, describe, expect, it, vi } from "vitest";
import { ApiError } from "@/lib/api-helpers";

const http = vi.hoisted(() => ({
	get: vi.fn(),
	post: vi.fn(),
	byId: vi.fn(),
	del: vi.fn(),
}));

/** Eden Treaty tree for `/api/ledger-keys` and `/api/ledger-keys/:id`. */
vi.mock("@/lib/api-client", () => {
	const ledgerKeys = Object.assign(
		(params: { id: string }) => {
			http.byId(params);
			return { delete: http.del };
		},
		{ get: http.get, post: http.post },
	);
	return { api: { api: { "ledger-keys": ledgerKeys } } };
});

import {
	describeLedgerKeyError,
	fetchLedgerKeys,
	issueLedgerKey,
	type LedgerKeySummary,
	ledgerKeyStatus,
	revokeLedgerKey,
	validateIssueLedgerKeyForm,
} from "../ledger-keys";

const COMPANY = "8a1f7a8e-3c2b-4d5e-9f00-112233445566";
const SECRET = "drn_ro_AbCdEfGhIjKlMnOpQrStUvWxYz0123456789";

const ok = <T>(data: T) => ({ data: { success: true, data }, error: null });
const failure = (status: number, code: string, error: string) => ({
	data: null,
	error: { status, value: { success: false, error, code } },
});

const summary = (
	overrides: Partial<LedgerKeySummary> = {},
): LedgerKeySummary => ({
	id: "key-1",
	name: "GPUI laptop",
	start: "drn_ro_AbCd",
	companyId: COMPANY,
	createdAt: "2026-11-01T10:00:00.000Z",
	expiresAt: "2027-01-30T10:00:00.000Z",
	lastRequest: null,
	enabled: true,
	...overrides,
});

describe("ledger key HTTP contract", () => {
	beforeEach(() => {
		vi.clearAllMocks();
	});

	it("lists the company's keys with GET /api/ledger-keys?companyId= and keeps only safe fields", async () => {
		http.get.mockResolvedValue(
			ok([
				{
					...summary(),
					createdAt: new Date("2026-11-01T10:00:00.000Z"),
					key: SECRET,
					hash: "stored-hash",
				},
			]),
		);

		const keys = await fetchLedgerKeys(COMPANY);

		expect(http.get).toHaveBeenCalledWith({ query: { companyId: COMPANY } });
		expect(keys).toEqual([summary()]);
		expect(JSON.stringify(keys)).not.toContain(SECRET);
		expect(JSON.stringify(keys)).not.toContain("stored-hash");
	});

	it("issues with POST /api/ledger-keys sending only company, name and lifetime", async () => {
		http.post.mockResolvedValue(
			ok({
				id: "key-2",
				key: SECRET,
				start: "drn_ro_AbCd",
				name: "CLI",
				companyId: COMPANY,
				expiresAt: "2027-02-25T10:00:00.000Z",
			}),
		);

		const issued = await issueLedgerKey({
			companyId: COMPANY,
			name: "CLI",
			expiresInDays: 90,
		});

		expect(http.post).toHaveBeenCalledWith({
			companyId: COMPANY,
			name: "CLI",
			expiresInDays: 90,
		});
		expect(issued).toEqual({
			id: "key-2",
			key: SECRET,
			start: "drn_ro_AbCd",
			name: "CLI",
			companyId: COMPANY,
			expiresAt: "2027-02-25T10:00:00.000Z",
		});
	});

	it("revokes with DELETE /api/ledger-keys/:id", async () => {
		http.del.mockResolvedValue(ok({ id: "key-1", revoked: true }));

		await revokeLedgerKey("key-1");

		expect(http.byId).toHaveBeenCalledWith({ id: "key-1" });
		expect(http.del).toHaveBeenCalledTimes(1);
	});

	it("turns the API failure envelope into an ApiError with its code", async () => {
		http.post.mockResolvedValue(
			failure(
				409,
				"LEDGER_KEY_LIMIT_REACHED",
				"At most 10 active ledger keys per company; revoke one first",
			),
		);

		await expect(
			issueLedgerKey({ companyId: COMPANY, name: "CLI", expiresInDays: 90 }),
		).rejects.toMatchObject({
			name: "ApiError",
			code: "LEDGER_KEY_LIMIT_REACHED",
		});
	});
});

describe("ledgerKeyStatus", () => {
	const now = new Date("2026-12-01T00:00:00.000Z");

	it("is active while enabled and not expired", () => {
		expect(ledgerKeyStatus(summary(), now)).toBe("active");
		expect(ledgerKeyStatus(summary({ expiresAt: null }), now)).toBe("active");
	});

	it("is expired once the expiry date has passed", () => {
		expect(
			ledgerKeyStatus(summary({ expiresAt: "2026-11-30T23:59:59.000Z" }), now),
		).toBe("expired");
	});

	it("is disabled when the key is not enabled", () => {
		expect(ledgerKeyStatus(summary({ enabled: false }), now)).toBe("disabled");
	});
});

describe("describeLedgerKeyError", () => {
	it("explains each API error code in the app's language", () => {
		expect(
			describeLedgerKeyError(
				new ApiError("limit", "LEDGER_KEY_LIMIT_REACHED"),
				"x",
			),
		).toBe(
			"Ya tienes 10 llaves activas para esta empresa. Revoca una antes de emitir otra.",
		);
		expect(
			describeLedgerKeyError(
				new ApiError("member", "COMPANY_MEMBERSHIP_REQUIRED"),
				"x",
			),
		).toBe(
			"No eres miembro activo de esta empresa, así que no puedes gestionar sus llaves.",
		);
		expect(
			describeLedgerKeyError(
				new ApiError("name is required", "VALIDATION_ERROR"),
				"x",
			),
		).toBe("Datos inválidos: name is required");
		expect(
			describeLedgerKeyError(new ApiError("gone", "LEDGER_KEY_NOT_FOUND"), "x"),
		).toBe("La llave ya no existe o no es tuya. Actualiza la lista.");
		expect(
			describeLedgerKeyError(new ApiError("no", "SESSION_REQUIRED"), "x"),
		).toBe("Tu sesión expiró. Vuelve a iniciar sesión.");
	});

	it("falls back to the operation message for unknown failures", () => {
		expect(
			describeLedgerKeyError(new Error("boom"), "No se pudo emitir la llave."),
		).toBe("No se pudo emitir la llave.");
	});
});

describe("validateIssueLedgerKeyForm", () => {
	it("trims the name and defaults nothing silently", () => {
		expect(
			validateIssueLedgerKeyForm({ name: "  CLI  ", expiresInDays: "90" }),
		).toEqual({ ok: true, value: { name: "CLI", expiresInDays: 90 } });
	});

	it("accepts the 1 and 365 day boundaries", () => {
		expect(
			validateIssueLedgerKeyForm({ name: "a", expiresInDays: "1" }).ok,
		).toBe(true);
		expect(
			validateIssueLedgerKeyForm({ name: "a", expiresInDays: "365" }).ok,
		).toBe(true);
	});

	it("rejects an empty or too long name and an out-of-range lifetime", () => {
		expect(
			validateIssueLedgerKeyForm({ name: "   ", expiresInDays: "0" }),
		).toEqual({
			ok: false,
			errors: {
				name: "Escribe un nombre para reconocer la llave.",
				expiresInDays:
					"La vigencia debe ser un número entero entre 1 y 365 días.",
			},
		});
		expect(
			validateIssueLedgerKeyForm({
				name: "x".repeat(65),
				expiresInDays: "366",
			}),
		).toEqual({
			ok: false,
			errors: {
				name: "El nombre admite como máximo 64 caracteres.",
				expiresInDays:
					"La vigencia debe ser un número entero entre 1 y 365 días.",
			},
		});
		expect(
			validateIssueLedgerKeyForm({ name: "a", expiresInDays: "1.5" }).ok,
		).toBe(false);
	});
});
