import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { ApiError } from "@/lib/api-helpers";

const service = vi.hoisted(() => ({
	fetchLedgerKeys: vi.fn(),
	issueLedgerKey: vi.fn(),
	revokeLedgerKey: vi.fn(),
}));

vi.mock("../../services/ledger-keys", async (importOriginal) => ({
	...(await importOriginal<object>()),
	...service,
}));

vi.mock("@/lib/use-active-company-context", () => ({
	useActiveCompanyContext: () => ({
		companyContext: {
			companyId: "cmp-1",
			companyName: "Acme SAC",
			ruc: "20123456789",
		},
	}),
}));

import { LedgerKeysSettings } from "../LedgerKeysSettings";

const SECRET = "drn_ro_AbCdEfGhIjKlMnOpQrStUvWxYz0123456789";

const listedKey = {
	id: "key-1",
	name: "GPUI laptop",
	start: "drn_ro_AbCd",
	companyId: "cmp-1",
	createdAt: "2026-11-01T10:00:00.000Z",
	expiresAt: "2099-01-30T10:00:00.000Z",
	lastRequest: null,
	enabled: true,
};

function renderPage() {
	const client = new QueryClient({
		defaultOptions: { queries: { retry: false } },
	});
	const view = render(
		<QueryClientProvider client={client}>
			<LedgerKeysSettings />
		</QueryClientProvider>,
	);
	return { client, ...view };
}

/** Everything the app could keep around after the dialog closes. */
function persistedState(client: QueryClient): string {
	return JSON.stringify({
		queries: client
			.getQueryCache()
			.getAll()
			.map((query) => query.state.data),
		mutations: client
			.getMutationCache()
			.getAll()
			.map((mutation) => mutation.state.data),
		local: { ...window.localStorage },
		session: { ...window.sessionStorage },
	});
}

describe("LedgerKeysSettings", () => {
	beforeEach(() => {
		vi.clearAllMocks();
		window.localStorage.clear();
		window.sessionStorage.clear();
		service.fetchLedgerKeys.mockResolvedValue([listedKey]);
		service.issueLedgerKey.mockResolvedValue({
			id: "key-2",
			key: SECRET,
			start: "drn_ro_AbCd",
			name: "CLI",
			companyId: "cmp-1",
			expiresAt: "2027-02-25T10:00:00.000Z",
		});
		service.revokeLedgerKey.mockResolvedValue(undefined);
	});

	it("lists the active company's keys without any secret", async () => {
		renderPage();

		const row = await screen.findByRole("row", { name: /GPUI laptop/ });
		expect(service.fetchLedgerKeys).toHaveBeenCalledWith("cmp-1");
		expect(screen.getByText("Acme SAC")).toBeInTheDocument();
		expect(screen.getByText(/20123456789/)).toBeInTheDocument();
		for (const header of [
			"Nombre",
			"Inicio",
			"Creada",
			"Vence",
			"Último uso",
			"Estado",
		]) {
			expect(
				screen.getByRole("columnheader", { name: header }),
			).toBeInTheDocument();
		}
		const cells = within(row);
		expect(cells.getByText("drn_ro_AbCd…")).toBeInTheDocument();
		expect(cells.getByText("Nunca")).toBeInTheDocument();
		expect(cells.getByText("Activa")).toBeInTheDocument();
		expect(document.body.textContent).not.toContain(SECRET);
	});

	it("shows an expired key as expired", async () => {
		service.fetchLedgerKeys.mockResolvedValue([
			{ ...listedKey, expiresAt: "2020-01-01T00:00:00.000Z" },
		]);
		renderPage();

		const row = await screen.findByRole("row", { name: /GPUI laptop/ });
		expect(within(row).getByText("Vencida")).toBeInTheDocument();
	});

	it("shows the issued key once, with a warning, and forgets it when the dialog closes", async () => {
		const user = userEvent.setup();
		const writeText = vi.fn().mockResolvedValue(undefined);
		Object.defineProperty(navigator, "clipboard", {
			configurable: true,
			value: { writeText },
		});
		const { client } = renderPage();
		await screen.findByRole("row", { name: /GPUI laptop/ });

		await user.click(screen.getByRole("button", { name: "Emitir llave" }));
		const dialog = await screen.findByRole("dialog");
		expect(within(dialog).getByLabelText("Vigencia (días)")).toHaveValue(90);
		await user.type(within(dialog).getByLabelText("Nombre"), "CLI");
		await user.click(within(dialog).getByRole("button", { name: "Emitir" }));

		const field = await within(dialog).findByLabelText("Llave emitida");
		expect(service.issueLedgerKey).toHaveBeenCalledWith({
			companyId: "cmp-1",
			name: "CLI",
			expiresInDays: 90,
		});
		expect(field).toHaveValue(SECRET);
		expect(field).toHaveAttribute("readonly");
		expect(
			within(dialog).getByText(
				"Copia esta llave ahora. No se volverá a mostrar.",
			),
		).toBeInTheDocument();

		await user.click(within(dialog).getByRole("button", { name: "Copiar" }));
		expect(writeText).toHaveBeenCalledWith(SECRET);
		expect(await within(dialog).findByText("Copiada")).toBeInTheDocument();

		await user.click(within(dialog).getByRole("button", { name: "Listo" }));
		await waitFor(() => {
			expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
		});
		expect(document.body.innerHTML).not.toContain(SECRET);
		await waitFor(() => {
			expect(persistedState(client)).not.toContain(SECRET);
		});

		await user.click(screen.getByRole("button", { name: "Emitir llave" }));
		const reopened = await screen.findByRole("dialog");
		expect(within(reopened).queryByLabelText("Llave emitida")).toBeNull();
		expect(within(reopened).getByLabelText("Nombre")).toHaveValue("");
		expect(document.body.innerHTML).not.toContain(SECRET);
	});

	it("refuses an out-of-range lifetime before calling the API", async () => {
		const user = userEvent.setup();
		renderPage();
		await screen.findByRole("row", { name: /GPUI laptop/ });

		await user.click(screen.getByRole("button", { name: "Emitir llave" }));
		const dialog = await screen.findByRole("dialog");
		await user.type(within(dialog).getByLabelText("Nombre"), "CLI");
		const days = within(dialog).getByLabelText("Vigencia (días)");
		await user.clear(days);
		await user.type(days, "366");
		await user.click(within(dialog).getByRole("button", { name: "Emitir" }));

		expect(
			await within(dialog).findByText(
				"La vigencia debe ser un número entero entre 1 y 365 días.",
			),
		).toBeInTheDocument();
		expect(service.issueLedgerKey).not.toHaveBeenCalled();
	});

	it("shows the 409 limit error inline in the issue dialog", async () => {
		const user = userEvent.setup();
		service.issueLedgerKey.mockRejectedValue(
			new ApiError(
				"At most 10 active ledger keys per company; revoke one first",
				"LEDGER_KEY_LIMIT_REACHED",
			),
		);
		renderPage();
		await screen.findByRole("row", { name: /GPUI laptop/ });

		await user.click(screen.getByRole("button", { name: "Emitir llave" }));
		const dialog = await screen.findByRole("dialog");
		await user.type(within(dialog).getByLabelText("Nombre"), "CLI");
		await user.click(within(dialog).getByRole("button", { name: "Emitir" }));

		expect(await within(dialog).findByRole("alert")).toHaveTextContent(
			"Ya tienes 10 llaves activas para esta empresa. Revoca una antes de emitir otra.",
		);
		expect(within(dialog).queryByLabelText("Llave emitida")).toBeNull();
	});

	it("shows an API validation error inline in the issue dialog", async () => {
		const user = userEvent.setup();
		service.issueLedgerKey.mockRejectedValue(
			new ApiError(
				"name must contain visible characters only",
				"VALIDATION_ERROR",
			),
		);
		renderPage();
		await screen.findByRole("row", { name: /GPUI laptop/ });

		await user.click(screen.getByRole("button", { name: "Emitir llave" }));
		const dialog = await screen.findByRole("dialog");
		await user.type(within(dialog).getByLabelText("Nombre"), "CLI");
		await user.click(within(dialog).getByRole("button", { name: "Emitir" }));

		expect(await within(dialog).findByRole("alert")).toHaveTextContent(
			"Datos inválidos: name must contain visible characters only",
		);
	});

	it("shows the 403 membership error inline instead of the list", async () => {
		service.fetchLedgerKeys.mockRejectedValue(
			new ApiError(
				"You are not an active member of this company",
				"COMPANY_MEMBERSHIP_REQUIRED",
			),
		);
		renderPage();

		expect(await screen.findByRole("alert")).toHaveTextContent(
			"No eres miembro activo de esta empresa, así que no puedes gestionar sus llaves.",
		);
		expect(screen.queryByRole("row", { name: /GPUI laptop/ })).toBeNull();
	});

	it("revokes only after an explicit confirmation", async () => {
		const user = userEvent.setup();
		renderPage();
		await screen.findByRole("row", { name: /GPUI laptop/ });

		await user.click(
			screen.getByRole("button", { name: "Revocar GPUI laptop" }),
		);
		let confirm = await screen.findByRole("alertdialog");
		expect(
			within(confirm).getByText("¿Revocar la llave «GPUI laptop»?"),
		).toBeInTheDocument();
		expect(service.revokeLedgerKey).not.toHaveBeenCalled();

		await user.click(within(confirm).getByRole("button", { name: "Cancelar" }));
		await waitFor(() => {
			expect(screen.queryByRole("alertdialog")).not.toBeInTheDocument();
		});
		expect(service.revokeLedgerKey).not.toHaveBeenCalled();

		await user.click(
			screen.getByRole("button", { name: "Revocar GPUI laptop" }),
		);
		confirm = await screen.findByRole("alertdialog");
		await user.click(
			within(confirm).getByRole("button", { name: "Revocar llave" }),
		);

		await waitFor(() => {
			expect(screen.queryByRole("alertdialog")).not.toBeInTheDocument();
		});
		expect(service.revokeLedgerKey).toHaveBeenCalledWith("key-1");
		await waitFor(() => {
			expect(service.fetchLedgerKeys).toHaveBeenCalledTimes(2);
		});
	});

	it("keeps the confirmation open and shows the error when revoking fails", async () => {
		const user = userEvent.setup();
		service.revokeLedgerKey.mockRejectedValue(
			new ApiError("Ledger key not found", "LEDGER_KEY_NOT_FOUND"),
		);
		renderPage();
		await screen.findByRole("row", { name: /GPUI laptop/ });

		await user.click(
			screen.getByRole("button", { name: "Revocar GPUI laptop" }),
		);
		const confirm = await screen.findByRole("alertdialog");
		await user.click(
			within(confirm).getByRole("button", { name: "Revocar llave" }),
		);

		expect(await within(confirm).findByRole("alert")).toHaveTextContent(
			"La llave ya no existe o no es tuya. Actualiza la lista.",
		);
		expect(screen.getByRole("alertdialog")).toBeInTheDocument();
	});
});
