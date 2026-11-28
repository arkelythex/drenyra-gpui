import { KeyRound, Plus, ShieldAlert } from "lucide-react";
import { useState } from "react";
import { Button } from "@/components/ui/button";
import { PageHeader } from "@/components/ui/PageHeader";
import { PageShell } from "@/components/ui/PageShell";
import { SettingsSection } from "@/features/settings/components/SettingsPrimitives";
import { useLedgerKeys } from "../hooks/use-ledger-keys";
import {
	describeLedgerKeyError,
	LEDGER_KEYS_MAX_ACTIVE_PER_COMPANY,
	type LedgerKeySummary,
} from "../services/ledger-keys";
import { IssueLedgerKeyDialog } from "./IssueLedgerKeyDialog";
import { LedgerKeysTable } from "./LedgerKeysTable";
import { RevokeLedgerKeyDialog } from "./RevokeLedgerKeyDialog";

/**
 * Settings → read-only ledger keys for the active company: issue (shown
 * once), list without secrets, and revoke with confirmation.
 */
export function LedgerKeysSettings() {
	const controller = useLedgerKeys();
	const { companyContext, hasCompany, issue, revoke } = controller;
	const [issueOpen, setIssueOpen] = useState(false);
	const [revokeTarget, setRevokeTarget] = useState<LedgerKeySummary | null>(
		null,
	);
	const companyName = companyContext.companyName ?? "la empresa activa";

	return (
		<PageShell variant="default" padding="lg" className="h-full">
			<PageHeader
				title="Llaves de lectura del ledger"
				description="Llaves personales de solo lectura para que la app nativa, la CLI o un agente lean el ledger de una empresa sin tu sesión."
				icon={<KeyRound size={18} strokeWidth={1.75} />}
				actions={
					<Button
						variant="primary"
						onClick={() => setIssueOpen(true)}
						disabled={!hasCompany}
					>
						<Plus size={16} strokeWidth={2} aria-hidden="true" />
						Emitir llave
					</Button>
				}
			/>

			<div className="flex flex-wrap items-center gap-x-4 gap-y-1 text-xs text-[var(--text-secondary)]">
				<span>Empresa</span>
				<strong className="text-[var(--text-primary)]">{companyName}</strong>
				{companyContext.ruc ? <span>RUC {companyContext.ruc}</span> : null}
			</div>

			<div className="flex gap-3 rounded-lg border border-[var(--color-warning)]/30 bg-[var(--color-warning)]/10 px-4 py-3 text-sm text-[var(--text-primary)]">
				<ShieldAlert
					size={16}
					aria-hidden="true"
					className="mt-0.5 shrink-0 text-[var(--color-warning)]"
				/>
				<p>
					Una llave lee el plan de cuentas, el libro mayor, el balance de
					comprobación y los asientos de esta empresa hasta que venza o la
					revoques. No puede escribir. Si pierdes membresía en la empresa, sus
					llaves dejan de funcionar.
				</p>
			</div>

			<SettingsSection
				title="Tus llaves para esta empresa"
				description={`Solo ves tus propias llaves. Máximo ${LEDGER_KEYS_MAX_ACTIVE_PER_COMPANY} activas por empresa; la llave completa solo se muestra al emitirla.`}
			>
				<LedgerKeysListBody
					controller={controller}
					onRevoke={setRevokeTarget}
				/>
			</SettingsSection>

			<IssueLedgerKeyDialog
				open={issueOpen}
				onOpenChange={setIssueOpen}
				companyName={companyName}
				issue={issue}
			/>
			<RevokeLedgerKeyDialog
				target={revokeTarget}
				onClose={() => setRevokeTarget(null)}
				revoke={revoke}
			/>
		</PageShell>
	);
}

function LedgerKeysListBody({
	controller,
	onRevoke,
}: {
	controller: ReturnType<typeof useLedgerKeys>;
	onRevoke: (key: LedgerKeySummary) => void;
}) {
	const { hasCompany, isLoading, listError, keys } = controller;

	if (!hasCompany) {
		return (
			<p className="text-sm text-[var(--text-secondary)]">
				Selecciona una empresa activa para gestionar sus llaves.
			</p>
		);
	}
	if (listError) {
		return (
			<p
				role="alert"
				className="rounded-md border border-[var(--color-danger)]/30 bg-[var(--color-danger)]/10 px-3 py-2 text-sm text-[var(--color-danger)]"
			>
				{describeLedgerKeyError(listError, "No se pudieron cargar las llaves.")}
			</p>
		);
	}
	if (isLoading) {
		return (
			<p className="text-sm text-[var(--text-secondary)]">Cargando llaves…</p>
		);
	}
	if (keys.length === 0) {
		return (
			<p className="text-sm text-[var(--text-secondary)]">
				Aún no has emitido llaves para esta empresa. Emite una para conectar la
				app nativa, la CLI o un agente.
			</p>
		);
	}
	return <LedgerKeysTable keys={keys} onRevoke={onRevoke} />;
}
