/**
 * Time zone names Postgres knows (`pg_timezone_names`), cached per process.
 *
 * The scan is a full function scan (~13-50 ms) and its answer only changes
 * when the server's tzdata changes, so ledger reports read it at most once per
 * TTL instead of once per call. Concurrent callers share one scan; an empty
 * answer is never cached (it would send every company to the fallback zone).
 */
import { db as globalDb } from "@drenyra/persistence/client";
import { sql } from "@drenyra/persistence/query";

/** How long a scan of `pg_timezone_names` is reused: 1 hour. */
export const KNOWN_TIME_ZONES_TTL_MS = 3_600_000;

let cached: { names: ReadonlySet<string>; expiresAt: number } | null = null;
let pending: Promise<ReadonlySet<string>> | null = null;

async function scanTimeZoneNames(): Promise<ReadonlySet<string>> {
	const rows = await globalDb.execute<{ name: unknown }>(
		sql`SELECT name FROM pg_catalog.pg_timezone_names`,
	);
	const names = new Set<string>();
	for (const row of rows) {
		if (typeof row.name === "string") names.add(row.name);
	}
	return names;
}

/**
 * Names in `pg_timezone_names`, from the per-process cache when it is fresh.
 *
 * @returns Exact (case-sensitive) zone names, e.g. `America/Lima`.
 */
export function knownTimeZoneNames(): Promise<ReadonlySet<string>> {
	if (cached && cached.expiresAt > Date.now()) {
		return Promise.resolve(cached.names);
	}
	if (!pending) {
		pending = scanTimeZoneNames()
			.then((names) => {
				if (names.size > 0) {
					cached = { names, expiresAt: Date.now() + KNOWN_TIME_ZONES_TTL_MS };
				}
				return names;
			})
			.finally(() => {
				pending = null;
			});
	}
	return pending;
}

/** Forget the cached names (tests). */
export function resetKnownTimeZoneNamesCache(): void {
	cached = null;
	pending = null;
}
