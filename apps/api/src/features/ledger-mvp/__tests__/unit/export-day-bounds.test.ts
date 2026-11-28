/**
 * General ledger exports pass `YYYY-MM-DD` dates to the general ledger as
 * whole calendar days (fiscal zone), and keep their range/default behavior.
 */
import { beforeEach, describe, expect, it, vi } from "vitest";

const mocks = vi.hoisted(() => ({ getGeneralLedger: vi.fn() }));

vi.mock("../../../ledger/queries/get-general-ledger.query", () => ({
	getGeneralLedger: mocks.getGeneralLedger,
}));

vi.mock("@drenyra/persistence/client", () => ({
	db: {
		select: vi.fn(() => ({
			from: vi.fn(() => ({
				where: vi.fn(() => ({
					limit: vi.fn().mockResolvedValue([]),
				})),
			})),
		})),
	},
	client: vi.fn(),
}));

const { exportGeneralLedgerPdf } = await import(
	"../../application/queries/export-general-ledger-pdf.query"
);
const { exportGeneralLedgerXlsx } = await import(
	"../../application/queries/export-general-ledger-xlsx.query"
);

const exporters = [
	["pdf", exportGeneralLedgerPdf],
	["xlsx", exportGeneralLedgerXlsx],
] as const;

describe.each(exporters)(
	"general ledger %s export — day bounds",
	(_name, exporter) => {
		beforeEach(() => {
			mocks.getGeneralLedger.mockReset().mockResolvedValue([]);
		});

		it("passes YYYY-MM-DD dates as whole calendar days", async () => {
			const result = await exporter({
				companyId: "cmp-1",
				startDate: "2026-03-01",
				endDate: "2026-03-31",
			});

			expect(result.ok).toBe(true);
			expect(mocks.getGeneralLedger).toHaveBeenCalledExactlyOnceWith(
				"cmp-1",
				{ kind: "calendar-day", day: "2026-03-01" },
				{ kind: "calendar-day", day: "2026-03-31" },
			);
		});

		it("still defaults missing dates to the epoch and now", async () => {
			const result = await exporter({ companyId: "cmp-1" });

			expect(result.ok).toBe(true);
			const [, start, end] = mocks.getGeneralLedger.mock.calls[0] ?? [];
			expect(start).toEqual(new Date(0));
			expect(end).toBeInstanceOf(Date);
		});

		it("still rejects startDate after endDate with INVALID_DATE_RANGE", async () => {
			const result = await exporter({
				companyId: "cmp-1",
				startDate: "2026-03-31",
				endDate: "2026-03-01",
			});

			expect(result).toEqual({
				ok: false,
				status: 400,
				message: "startDate must be <= endDate",
				code: "INVALID_DATE_RANGE",
			});
			expect(mocks.getGeneralLedger).not.toHaveBeenCalled();
		});

		it("rejects a calendar day that does not exist with 400 INVALID_DATE", async () => {
			for (const input of [
				{ startDate: "2026-02-30", endDate: "2026-03-31" },
				{ startDate: "2026-03-01", endDate: "2026-02-30" },
			]) {
				const result = await exporter({ companyId: "cmp-1", ...input });
				expect(result).toMatchObject({
					ok: false,
					status: 400,
					code: "INVALID_DATE",
				});
			}
			expect(mocks.getGeneralLedger).not.toHaveBeenCalled();
		});
	},
);
