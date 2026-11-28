import { AlertTriangle, Copy } from "lucide-react";
import { type FormEvent, useId, useState } from "react";
import { Button } from "@/components/ui/button";
import {
	Dialog,
	DialogContent,
	DialogDescription,
	DialogFooter,
	DialogHeader,
	DialogTitle,
} from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { formatDate } from "@/lib/utils";
import type { LedgerKeysController } from "../hooks/use-ledger-keys";
import {
	describeLedgerKeyError,
	type IssuedLedgerKey,
	type IssueLedgerKeyFormValues,
	LEDGER_KEY_DEFAULT_EXPIRES_IN_DAYS,
	LEDGER_KEY_MAX_EXPIRES_IN_DAYS,
	LEDGER_KEY_NAME_MAX_LENGTH,
	validateIssueLedgerKeyForm,
} from "../services/ledger-keys";

type FieldErrors = Partial<Record<keyof IssueLedgerKeyFormValues, string>>;
type CopyState = "idle" | "copied" | "failed";

const EMPTY_FORM: IssueLedgerKeyFormValues = {
	name: "",
	expiresInDays: String(LEDGER_KEY_DEFAULT_EXPIRES_IN_DAYS),
};

interface IssueLedgerKeyDialogProps {
	open: boolean;
	onOpenChange: (open: boolean) => void;
	companyName: string;
	issue: LedgerKeysController["issue"];
}

/**
 * Issue a key and show its plaintext once. Closing the dialog clears the form
 * and resets the mutation, so the plaintext leaves memory with the dialog.
 */
export function IssueLedgerKeyDialog({
	open,
	onOpenChange,
	companyName,
	issue,
}: IssueLedgerKeyDialogProps) {
	const [form, setForm] = useState(EMPTY_FORM);
	const [fieldErrors, setFieldErrors] = useState<FieldErrors>({});

	const handleOpenChange = (next: boolean) => {
		if (!next && issue.isPending) return;
		if (!next) {
			setForm(EMPTY_FORM);
			setFieldErrors({});
			issue.reset();
		}
		onOpenChange(next);
	};

	const submit = (event: FormEvent<HTMLFormElement>) => {
		event.preventDefault();
		const result = validateIssueLedgerKeyForm(form);
		if (!result.ok) {
			setFieldErrors(result.errors);
			return;
		}
		setFieldErrors({});
		issue.mutate(result.value);
	};

	return (
		<Dialog open={open} onOpenChange={handleOpenChange}>
			<DialogContent>
				{issue.data ? (
					<IssuedKeyView
						issued={issue.data}
						companyName={companyName}
						onDone={() => handleOpenChange(false)}
					/>
				) : (
					<IssueKeyForm
						form={form}
						fieldErrors={fieldErrors}
						apiError={
							issue.error
								? describeLedgerKeyError(
										issue.error,
										"No se pudo emitir la llave.",
									)
								: null
						}
						isPending={issue.isPending}
						companyName={companyName}
						onChange={setForm}
						onSubmit={submit}
						onCancel={() => handleOpenChange(false)}
					/>
				)}
			</DialogContent>
		</Dialog>
	);
}

interface IssueKeyFormProps {
	form: IssueLedgerKeyFormValues;
	fieldErrors: FieldErrors;
	apiError: string | null;
	isPending: boolean;
	companyName: string;
	onChange: (form: IssueLedgerKeyFormValues) => void;
	onSubmit: (event: FormEvent<HTMLFormElement>) => void;
	onCancel: () => void;
}

function IssueKeyForm({
	form,
	fieldErrors,
	apiError,
	isPending,
	companyName,
	onChange,
	onSubmit,
	onCancel,
}: IssueKeyFormProps) {
	const nameId = useId();
	const daysId = useId();

	return (
		<form noValidate onSubmit={onSubmit} className="space-y-4">
			<DialogHeader>
				<DialogTitle>Emitir llave de lectura</DialogTitle>
				<DialogDescription>
					Solo lectura del ledger de {companyName}. No permite registrar,
					modificar ni declarar nada.
				</DialogDescription>
			</DialogHeader>

			<div className="space-y-1.5">
				<Label htmlFor={nameId}>Nombre</Label>
				<Input
					id={nameId}
					value={form.name}
					maxLength={LEDGER_KEY_NAME_MAX_LENGTH}
					placeholder="Ej. Laptop de contabilidad"
					autoComplete="off"
					aria-invalid={Boolean(fieldErrors.name)}
					onChange={(event) => onChange({ ...form, name: event.target.value })}
				/>
				{fieldErrors.name ? (
					<p className="text-xs text-[var(--color-danger)]">
						{fieldErrors.name}
					</p>
				) : null}
			</div>

			<div className="space-y-1.5">
				<Label htmlFor={daysId}>Vigencia (días)</Label>
				<Input
					id={daysId}
					type="number"
					inputMode="numeric"
					min={1}
					max={LEDGER_KEY_MAX_EXPIRES_IN_DAYS}
					step={1}
					value={form.expiresInDays}
					aria-invalid={Boolean(fieldErrors.expiresInDays)}
					onChange={(event) =>
						onChange({ ...form, expiresInDays: event.target.value })
					}
				/>
				<p className="text-xs text-[var(--text-secondary)]">
					Entre 1 y {LEDGER_KEY_MAX_EXPIRES_IN_DAYS} días. Por defecto{" "}
					{LEDGER_KEY_DEFAULT_EXPIRES_IN_DAYS}.
				</p>
				{fieldErrors.expiresInDays ? (
					<p className="text-xs text-[var(--color-danger)]">
						{fieldErrors.expiresInDays}
					</p>
				) : null}
			</div>

			{apiError ? (
				<p
					role="alert"
					className="rounded-md border border-[var(--color-danger)]/30 bg-[var(--color-danger)]/10 px-3 py-2 text-sm text-[var(--color-danger)]"
				>
					{apiError}
				</p>
			) : null}

			<DialogFooter>
				<Button
					type="button"
					variant="secondary"
					onClick={onCancel}
					disabled={isPending}
				>
					Cancelar
				</Button>
				<Button type="submit" variant="primary" disabled={isPending}>
					{isPending ? "Emitiendo…" : "Emitir"}
				</Button>
			</DialogFooter>
		</form>
	);
}

interface IssuedKeyViewProps {
	issued: IssuedLedgerKey;
	companyName: string;
	onDone: () => void;
}

function IssuedKeyView({ issued, companyName, onDone }: IssuedKeyViewProps) {
	const keyId = useId();
	const [copyState, setCopyState] = useState<CopyState>("idle");

	const copy = async () => {
		try {
			await navigator.clipboard.writeText(issued.key);
			setCopyState("copied");
		} catch {
			setCopyState("failed");
		}
	};

	return (
		<div className="space-y-4">
			<DialogHeader>
				<DialogTitle>Llave emitida</DialogTitle>
				<DialogDescription>
					{issued.name ?? "Sin nombre"} · solo lectura del ledger de{" "}
					{companyName}
					{issued.expiresAt ? ` · vence ${formatDate(issued.expiresAt)}` : ""}
				</DialogDescription>
			</DialogHeader>

			<div className="flex gap-2 rounded-md border border-[var(--color-warning)]/30 bg-[var(--color-warning)]/10 px-3 py-2 text-sm text-[var(--text-primary)]">
				<AlertTriangle
					size={16}
					aria-hidden="true"
					className="mt-0.5 shrink-0 text-[var(--color-warning)]"
				/>
				<div className="space-y-1">
					<p className="font-semibold">
						Copia esta llave ahora. No se volverá a mostrar.
					</p>
					<p className="text-xs text-[var(--text-secondary)]">
						Guárdala en el llavero del sistema o en un gestor de secretos; nunca
						en un repositorio, chat, log o URL. Si se filtra, revócala.
					</p>
				</div>
			</div>

			<div className="space-y-1.5">
				<Label htmlFor={keyId}>Llave emitida</Label>
				<div className="flex gap-2">
					<Input
						id={keyId}
						readOnly
						value={issued.key}
						autoComplete="off"
						spellCheck={false}
						className="font-mono text-xs"
						onFocus={(event) => event.currentTarget.select()}
					/>
					<Button type="button" variant="secondary" onClick={copy}>
						<Copy size={14} aria-hidden="true" />
						{copyState === "copied" ? "Copiada" : "Copiar"}
					</Button>
				</div>
				{copyState === "failed" ? (
					<p role="alert" className="text-xs text-[var(--color-danger)]">
						No se pudo copiar. Selecciona el texto y cópialo manualmente.
					</p>
				) : null}
			</div>

			<DialogFooter>
				<Button type="button" variant="primary" onClick={onDone}>
					Listo
				</Button>
			</DialogFooter>
		</div>
	);
}
