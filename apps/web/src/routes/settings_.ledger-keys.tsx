import { createFileRoute } from "@tanstack/react-router";
import { LedgerKeysSettings } from "@/features/ledger-keys";

/** Settings → read-only ledger keys of the active company (not nested under the /settings placeholder). */
export const Route = createFileRoute("/settings_/ledger-keys")({
	component: LedgerKeysSettings,
});
