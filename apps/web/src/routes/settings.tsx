import { createFileRoute, Link } from "@tanstack/react-router";

export const Route = createFileRoute("/settings")({
	component: () => (
		<div className="p-6">
			<h1 className="text-xl font-semibold">Settings</h1>
			<p className="text-muted-foreground mt-2">Coming soon</p>
			<nav aria-label="Secciones de configuración" className="mt-6">
				<Link
					to="/settings/ledger-keys"
					className="text-sm font-medium text-[var(--text-primary)] underline-offset-4 hover:underline"
				>
					Llaves de lectura del ledger
				</Link>
				<p className="text-muted-foreground text-xs">
					Llaves de solo lectura para la app nativa, la CLI o agentes.
				</p>
			</nav>
		</div>
	),
});
