import {
	AlertDialog,
	AlertDialogCancel,
	AlertDialogContent,
	AlertDialogDescription,
	AlertDialogFooter,
	AlertDialogHeader,
	AlertDialogTitle,
} from "@/components/ui/alert-dialog";
import { Button } from "@/components/ui/button";
import type { LedgerKeysController } from "../hooks/use-ledger-keys";
import {
	describeLedgerKeyError,
	type LedgerKeySummary,
} from "../services/ledger-keys";

interface RevokeLedgerKeyDialogProps {
	target: LedgerKeySummary | null;
	onClose: () => void;
	revoke: LedgerKeysController["revoke"];
}

/** Explicit confirmation before revoking; stays open to show a failure. */
export function RevokeLedgerKeyDialog({
	target,
	onClose,
	revoke,
}: RevokeLedgerKeyDialogProps) {
	const label = target?.name ?? target?.start ?? "sin nombre";

	const close = () => {
		if (revoke.isPending) return;
		revoke.reset();
		onClose();
	};

	const confirm = () => {
		if (!target) return;
		revoke.mutate(target.id, {
			onSuccess: () => {
				revoke.reset();
				onClose();
			},
		});
	};

	return (
		<AlertDialog
			open={target !== null}
			onOpenChange={(open) => {
				if (!open) close();
			}}
		>
			<AlertDialogContent>
				<AlertDialogHeader>
					<AlertDialogTitle>¿Revocar la llave «{label}»?</AlertDialogTitle>
					<AlertDialogDescription>
						Los clientes que la usen (app nativa, CLI o agentes) dejarán de leer
						el ledger en su siguiente solicitud. No se puede deshacer: para
						volver a conectar tendrás que emitir una llave nueva.
					</AlertDialogDescription>
				</AlertDialogHeader>

				{revoke.error ? (
					<p
						role="alert"
						className="rounded-md border border-[var(--color-danger)]/30 bg-[var(--color-danger)]/10 px-3 py-2 text-sm text-[var(--color-danger)]"
					>
						{describeLedgerKeyError(
							revoke.error,
							"No se pudo revocar la llave.",
						)}
					</p>
				) : null}

				<AlertDialogFooter>
					<AlertDialogCancel asChild>
						<Button variant="secondary" disabled={revoke.isPending}>
							Cancelar
						</Button>
					</AlertDialogCancel>
					<Button
						variant="danger"
						onClick={confirm}
						disabled={revoke.isPending}
					>
						{revoke.isPending ? "Revocando…" : "Revocar llave"}
					</Button>
				</AlertDialogFooter>
			</AlertDialogContent>
		</AlertDialog>
	);
}
