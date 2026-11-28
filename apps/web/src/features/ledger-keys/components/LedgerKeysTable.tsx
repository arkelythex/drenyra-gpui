import { Trash2 } from "lucide-react";
import {
	StatusBadge,
	type StatusBadgeProps,
} from "@/components/ui/StatusBadge";
import { formatDate } from "@/lib/utils";
import {
	type LedgerKeyStatus,
	type LedgerKeySummary,
	ledgerKeyStatus,
} from "../services/ledger-keys";

const STATUS_BADGE: Record<
	LedgerKeyStatus,
	{ status: StatusBadgeProps["status"]; label: string }
> = {
	active: { status: "success", label: "Activa" },
	expired: { status: "warning", label: "Vencida" },
	disabled: { status: "neutral", label: "Desactivada" },
};

const COLUMNS = [
	"Nombre",
	"Inicio",
	"Creada",
	"Vence",
	"Último uso",
	"Estado",
] as const;

function dateOrDash(value: string | null, empty = "—"): string {
	return value ? formatDate(value) : empty;
}

interface LedgerKeysTableProps {
	keys: readonly LedgerKeySummary[];
	onRevoke: (key: LedgerKeySummary) => void;
}

/** Keys as listed by the API: name, visible start and dates, never the secret. */
export function LedgerKeysTable({ keys, onRevoke }: LedgerKeysTableProps) {
	const now = new Date();

	return (
		<div className="overflow-x-auto">
			<table className="w-full text-left text-sm">
				<caption className="sr-only">
					Tus llaves de lectura del ledger para esta empresa
				</caption>
				<thead>
					<tr className="border-b border-[var(--border-subtle)] text-xs uppercase tracking-wider text-[var(--text-muted)]">
						{COLUMNS.map((column) => (
							<th key={column} scope="col" className="px-3 py-2 font-semibold">
								{column}
							</th>
						))}
						<th scope="col" className="px-3 py-2">
							<span className="sr-only">Acciones</span>
						</th>
					</tr>
				</thead>
				<tbody className="divide-y divide-[var(--border-subtle)]">
					{keys.map((key) => {
						const badge = STATUS_BADGE[ledgerKeyStatus(key, now)];
						const name = key.name ?? "Sin nombre";
						return (
							<tr key={key.id}>
								<td className="px-3 py-2.5 font-medium text-[var(--text-primary)]">
									{name}
								</td>
								<td className="px-3 py-2.5 font-mono text-xs text-[var(--text-secondary)]">
									{key.start ? `${key.start}…` : "—"}
								</td>
								<td className="px-3 py-2.5 text-[var(--text-secondary)]">
									{dateOrDash(key.createdAt)}
								</td>
								<td className="px-3 py-2.5 text-[var(--text-secondary)]">
									{dateOrDash(key.expiresAt, "Sin vencimiento")}
								</td>
								<td className="px-3 py-2.5 text-[var(--text-secondary)]">
									{dateOrDash(key.lastRequest, "Nunca")}
								</td>
								<td className="px-3 py-2.5">
									<StatusBadge status={badge.status} label={badge.label} />
								</td>
								<td className="px-3 py-2.5 text-right">
									<button
										type="button"
										onClick={() => onRevoke(key)}
										aria-label={`Revocar ${name}`}
										title="Revocar llave"
										className="inline-flex h-8 w-8 items-center justify-center rounded-lg text-[var(--text-muted)] transition-colors hover:bg-[var(--color-danger)]/10 hover:text-[var(--color-danger)]"
									>
										<Trash2 size={14} strokeWidth={2} aria-hidden="true" />
									</button>
								</td>
							</tr>
						);
					})}
				</tbody>
			</table>
		</div>
	);
}
