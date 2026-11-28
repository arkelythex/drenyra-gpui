import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useActiveCompanyContext } from "@/lib/use-active-company-context";
import {
	fetchLedgerKeys,
	issueLedgerKey,
	revokeLedgerKey,
} from "../services/ledger-keys";

export const ledgerKeysQueryKey = (companyId: string) =>
	["ledger-keys", companyId] as const;

/**
 * The signed-in user's read-only ledger keys for the active company.
 *
 * The issue mutation is the only holder of the plaintext key: `gcTime: 0`
 * drops it from the mutation cache as soon as the dialog resets it, and the
 * list query only ever holds summaries without secrets.
 */
export function useLedgerKeys() {
	const queryClient = useQueryClient();
	const { companyContext } = useActiveCompanyContext();
	const companyId = companyContext.companyId ?? "";
	const queryKey = ledgerKeysQueryKey(companyId);

	const list = useQuery({
		queryKey,
		queryFn: () => fetchLedgerKeys(companyId),
		enabled: Boolean(companyId),
	});

	const refreshList = () => queryClient.invalidateQueries({ queryKey });

	const issue = useMutation({
		mutationFn: (input: { name: string; expiresInDays: number }) =>
			issueLedgerKey({ companyId, ...input }),
		gcTime: 0,
		onSuccess: refreshList,
	});

	const revoke = useMutation({
		mutationFn: (id: string) => revokeLedgerKey(id),
		onSuccess: refreshList,
	});

	return {
		companyContext,
		hasCompany: Boolean(companyId),
		keys: list.data ?? [],
		isLoading: list.isLoading,
		listError: list.error,
		issue,
		revoke,
	};
}

export type LedgerKeysController = ReturnType<typeof useLedgerKeys>;
