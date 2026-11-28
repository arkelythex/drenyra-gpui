/**
 * Agent tool `get_ledger_entry` asks the general ledger for whole fiscal days:
 * `YYYY-MM-DD` (and a UTC-midnight date-time) is a calendar day, and a date
 * that does not exist is refused before any query.
 */
import type { AgentContext } from "@drenyra/agent-runtime";
import { beforeEach, describe, expect, it, vi } from "vitest";

const mocks = vi.hoisted(() => ({ getGeneralLedger: vi.fn() }));

vi.mock("@drenyra/persistence/client", () => ({ db: {}, client: vi.fn() }));

vi.mock("../../../ledger", async (importOriginal) => {
	const actual = await importOriginal<typeof import("../../../ledger")>();
	return {
		...actual,
		LedgerService: { getGeneralLedger: mocks.getGeneralLedger },
	};
});

const { getLedgerTool } = await import("../finance.tools");

const context = { companyId: "cmp-1" } as AgentContext;

describe("get_ledger_entry tool — fiscal-day bounds", () => {
	beforeEach(() => {
		mocks.getGeneralLedger.mockReset().mockResolvedValue([]);
	});

	it("passes YYYY-MM-DD dates as whole calendar days", async () => {
		const result = await getLedgerTool.execute(
			{ startDate: "2026-03-01", endDate: "2026-03-31" },
			context,
		);

		expect(result).toEqual({ entries: [] });
		expect(mocks.getGeneralLedger).toHaveBeenCalledExactlyOnceWith(
			"cmp-1",
			{ kind: "calendar-day", day: "2026-03-01" },
			{ kind: "calendar-day", day: "2026-03-31" },
		);
	});

	it("reads a date-time at exactly 00:00:00.000Z as that calendar day", async () => {
		await getLedgerTool.execute(
			{ startDate: "2026-03-01", endDate: "2026-03-31T00:00:00.000Z" },
			context,
		);

		const [, , end] = mocks.getGeneralLedger.mock.calls[0] ?? [];
		expect(end).toEqual({ kind: "calendar-day", day: "2026-03-31" });
	});

	it("refuses a date that does not exist without querying the ledger", async () => {
		await expect(
			getLedgerTool.execute(
				{ startDate: "2026-02-30", endDate: "2026-03-31" },
				context,
			),
		).rejects.toThrow("startDate/endDate invalid. Use YYYY-MM-DD.");
		expect(mocks.getGeneralLedger).not.toHaveBeenCalled();
	});

	it("still defaults to 1 January of the current year through now", async () => {
		await getLedgerTool.execute({}, context);

		const [, start, end] = mocks.getGeneralLedger.mock.calls[0] ?? [];
		expect(start).toEqual({
			kind: "calendar-day",
			day: `${new Date().getFullYear()}-01-01`,
		});
		expect(end).toMatchObject({ kind: "instant" });
	});
});
