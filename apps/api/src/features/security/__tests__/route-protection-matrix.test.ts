import { readdirSync, readFileSync } from "node:fs";
import { relative, resolve } from "node:path";
import { describe, expect, it } from "vitest";

import {
	API_MODULES_ROUTE_PROTECTION_MATRIX,
	APP_CORE_MOUNTED_SURFACE_EXPORTS,
	PUBLIC_ROUTE_SURFACE_IDS,
	READ_ONLY_LEDGER_KEY_AUTH_MODE,
	ROUTE_PROTECTION_MATRIX,
	type RouteProtectionMatrixRow,
} from "../route-protection-matrix";

const repoRoot = resolve(__dirname, "../../../../../..");
const appCoreSource = readFileSync(
	resolve(repoRoot, "apps/api/src/app-core.ts"),
	"utf8",
);
const apiModuleSurfaceSource = readFileSync(
	resolve(repoRoot, "apps/api/src/api-module-surface.ts"),
	"utf8",
);
const routeMatrixDoc = readFileSync(
	resolve(repoRoot, "docs/04-api/hardening/route-protection-matrix.md"),
	"utf8",
);

function extractMountedSurfaceExports(source: string): string[] {
	const directUseNames = Array.from(
		source.matchAll(/\.use\(\s*([A-Za-z_$][\w$]*)/g),
	).map((match) => match[1]);
	const conditionalUseNames = source.includes(
		"ledgerMvpEnabled ? ledgerMvpModule",
	)
		? ["ledgerMvpModule"]
		: [];

	return [...new Set([...directUseNames, ...conditionalUseNames])]
		.filter(
			(name) =>
				![
					"cors",
					"swagger",
					"helmet",
					"ledgerMvpEnabled",
					"metricsMiddleware",
					"apiModules",
					"backwardCompatRedirects",
				].includes(name),
		)
		.sort();
}

const ALL_ROWS: readonly RouteProtectionMatrixRow[] = [
	...ROUTE_PROTECTION_MATRIX,
	...API_MODULES_ROUTE_PROTECTION_MATRIX,
];

function listSourceFiles(dir: string): string[] {
	return readdirSync(dir, { withFileTypes: true }).flatMap((entry) => {
		const path = resolve(dir, entry.name);
		if (entry.isDirectory()) {
			return entry.name === "__tests__" || entry.name === "node_modules"
				? []
				: listSourceFiles(path);
		}
		return entry.name.endsWith(".ts") ? [path] : [];
	});
}

describe("route protection matrix", () => {
	// TODO: Update mounted surface exports after cleanup — list is stale
	it("covers every mounted app-core surface", () => {
		expect(APP_CORE_MOUNTED_SURFACE_EXPORTS).toEqual(
			extractMountedSurfaceExports(appCoreSource),
		);

		const matrixExports = ROUTE_PROTECTION_MATRIX.map(
			(row) => row.appCoreExportName,
		).sort();

		expect(matrixExports).toEqual(APP_CORE_MOUNTED_SURFACE_EXPORTS);
	});

	it("lists apiModules surfaces that are really mounted there", () => {
		const mounted = Array.from(
			apiModuleSurfaceSource.matchAll(/\.use\(\s*([A-Za-z_$][\w$]*)/g),
		).map((match) => match[1]);

		for (const row of API_MODULES_ROUTE_PROTECTION_MATRIX) {
			expect(mounted).toContain(row.appCoreExportName);
		}
	});

	it("declares every module that accepts read-only ledger keys", () => {
		const apiSrc = resolve(repoRoot, "apps/api/src");
		const optsIn = (file: string) =>
			readFileSync(file, "utf8")
				.split("\n")
				.map((line) => line.trim())
				.filter((line) => !/^(\*|\/\/|\/\*)/.test(line))
				.some((line) => /allowReadOnlyLedgerKey:\s*true/.test(line));
		const optedIn = listSourceFiles(apiSrc)
			.filter(optsIn)
			.map((file) => relative(apiSrc, file))
			.sort();

		expect(optedIn).toEqual([
			"features/journal-entries/api/routes.ts",
			"features/ledger/routes.ts",
		]);

		const declared = Object.fromEntries(
			ALL_ROWS.filter((row) => row.readOnlyLedgerKeyRoutes).map((row) => [
				row.id,
				row.readOnlyLedgerKeyRoutes,
			]),
		);
		expect(declared).toEqual({
			"journal-entries": [
				"GET /api/journal-entries/",
				"GET /api/journal-entries/:id",
			],
			ledger: [
				"GET /api/ledger/accounts",
				"GET /api/ledger/general",
				"GET /api/ledger/trial-balance",
				"GET /api/ledger/key-identity",
			],
		});
		for (const routes of Object.values(declared)) {
			for (const route of routes ?? []) {
				expect(route.startsWith("GET ")).toBe(true);
			}
		}
	});

	it("declares ledger key management as a session-only app-core surface", () => {
		expect(extractMountedSurfaceExports(appCoreSource)).toContain(
			"ledgerKeysRoutes",
		);
		expect(APP_CORE_MOUNTED_SURFACE_EXPORTS).toContain("ledgerKeysRoutes");

		const row = ALL_ROWS.find((candidate) => candidate.id === "ledger-keys");
		expect(row).toMatchObject({
			appCoreExportName: "ledgerKeysRoutes",
			prefix: "/api/ledger-keys",
			authMode: "session",
			tenantSource: "session",
		});
		// A key can never manage keys: the surface accepts none.
		expect(row?.readOnlyLedgerKeyRoutes).toBeUndefined();
		expect(routeMatrixDoc).toContain("| Ledger Keys");
		expect(routeMatrixDoc).toContain("(../ledger-read-keys.md)");
	});

	it("documents the read-only ledger key routes", () => {
		expect(routeMatrixDoc).toContain(`\`${READ_ONLY_LEDGER_KEY_AUTH_MODE}\``);
		for (const row of ALL_ROWS) {
			for (const route of row.readOnlyLedgerKeyRoutes ?? []) {
				expect(routeMatrixDoc).toContain(`\`${route}\``);
			}
		}
	});

	it("documents every mounted runtime surface and prefix", () => {
		for (const row of ALL_ROWS) {
			expect(routeMatrixDoc).toContain(`| ${row.surface}`);
			expect(routeMatrixDoc).toContain(`\`${row.prefix}\``);
			expect(routeMatrixDoc).toContain(`\`${row.authMode}\``);
			expect(routeMatrixDoc).toContain(`\`${row.tenantSource}\``);
		}
	});

	it("keeps protected routes from silently using no tenant source", () => {
		const publicSurfaceIds = new Set(PUBLIC_ROUTE_SURFACE_IDS);
		const unsafeProtectedRows = ROUTE_PROTECTION_MATRIX.filter(
			(row) => !publicSurfaceIds.has(row.id) && row.tenantSource === "none",
		);

		expect(unsafeProtectedRows).toEqual([]);
	});
});
