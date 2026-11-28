export function formatDatePE(date: Date): string {
	return date.toLocaleDateString("es-PE", {
		day: "2-digit",
		month: "2-digit",
		year: "numeric",
	});
}
