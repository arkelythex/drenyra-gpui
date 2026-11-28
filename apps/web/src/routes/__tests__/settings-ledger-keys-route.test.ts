import { describe, expect, it } from "vitest";
import { LedgerKeysSettings } from "@/features/ledger-keys";
import { routeTree } from "@/routeTree.gen";
import { Route } from "../settings_.ledger-keys";

/** Top-level routes of the generated tree, by full path. */
function topLevelRoutes(): Map<string, { options: { component?: unknown } }> {
	const children = (routeTree as unknown as { children?: unknown }).children;
	const routes = (
		Array.isArray(children)
			? children
			: Object.values((children ?? {}) as Record<string, unknown>)
	) as Array<{ options: { path: string; component?: unknown } }>;
	return new Map(routes.map((route) => [route.options.path, route]));
}

describe("/settings/ledger-keys", () => {
	it("is registered as its own screen, not inside the /settings placeholder", () => {
		const routes = topLevelRoutes();
		expect(routes.has("/settings/ledger-keys")).toBe(true);
		expect(routes.has("/settings")).toBe(true);
	});

	it("renders the ledger keys settings page", () => {
		expect(Route.options.component).toBe(LedgerKeysSettings);
	});
});
