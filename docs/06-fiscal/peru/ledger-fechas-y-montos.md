# Ledger API — días fiscales y montos exactos

**Última actualización:** 2026-11-27
**Alcance:** `GET /api/ledger/trial-balance`, `GET /api/ledger/general` y las exportaciones `POST /api/ledger/export/{pdf,xlsx}` (`apps/api/src/features/ledger/`, `apps/api/src/features/ledger-mvp/`)

---

## Respuesta corta

- Una fecha `YYYY-MM-DD` significa **el día calendario completo** en la zona horaria fiscal de la empresa (`companies.settings_timezone`, por defecto `America/Lima`).
- La balanza "al 2026-03-31" incluye todo lo emitido hasta las 23:59:59.999 de Lima del 31 de marzo; nada del 1 de abril.
- `debits`, `credits` y `balance` son texto decimal exacto con 2 decimales (`"1234.50"`, `"-150.25"`), sumados en Postgres sobre `numeric`. Nunca pasan por números de punto flotante de JavaScript ni usan notación exponencial.

## Por qué importa

Antes, la API comparaba `issue_date <= YYYY-MM-DDT00:00:00Z`. Eso dejaba fuera **todo el último día del período en Perú** (UTC−5), y sumaba montos con floats de JavaScript (`toFixed(2)`), que pierden centavos en totales grandes. Una balanza incompleta o con centavos de más es un error contable, no cosmético.

## Fechas de entrada

| Valor enviado | Significado |
|---|---|
| `2026-03-31` | Día calendario completo en la zona fiscal de la empresa (forma recomendada). |
| `2026-04-01T04:59:59.999Z` (fecha-hora ISO) | Instante; se usa el día calendario que le corresponde en la zona fiscal de la empresa (aquí, 31 de marzo en Lima). |
| `2026-02-30`, `invalid` | `400 INVALID_DATE`. Un día que no existe ya no se reinterpreta como otro día. |

- `asOfDate` (balanza): incluye el día indicado completo (acumulado hasta ese día).
- `startDate` / `endDate` (mayor general y exportaciones): ambos incluidos; `startDate` empieza a las 00:00 del día en la zona fiscal.
- La respuesta mantiene `asOfDate` como antes: `"2026-03-31T00:00:00.000Z"` para `2026-03-31`.

## Cómo se decide el día de cada transacción

`transactions.issue_date` es `timestamp without time zone` y Drizzle guarda la hora UTC de un `Date`. Hoy conviven dos tipos de escritores:

| Escritor | Qué guarda | Ejemplo |
|---|---|---|
| Importación UBL (inbox), fechas de documento | Etiqueta de día: exactamente `00:00:00` (`new Date("YYYY-MM-DD")`) | `2026-03-31 00:00:00` = documento del 31 de marzo |
| Transacciones manuales, inter-company | Instante UTC (`new Date()`) | `2026-04-01 04:30:00` = 23:30 del 31 de marzo en Lima |

Por eso el día fiscal de una fila es:

1. si `issue_date` es exactamente medianoche → la fecha guardada (etiqueta de documento);
2. si no → el día calendario del instante UTC en la zona fiscal de la empresa.

"Día ≤ D" equivale al límite exclusivo "antes del inicio del día D + 1" en la zona fiscal.

**Ambigüedad conocida:** un instante guardado exactamente a las `00:00:00.000000` UTC (19:00 en Lima) se lee como etiqueta del día UTC. La solución definitiva es que los escritores guarden una fecha de documento explícita (columna `date`), lo que requiere una migración aparte.

## Montos

- Se suman en SQL (`SUM` sobre `numeric(12,2)`), se redondean a 2 decimales y se devuelven como `::text`.
- Los totales de la balanza se calculan en la misma consulta (función de ventana) sobre las mismas cuentas listadas.
- Si Postgres devolviera algo que no es texto decimal de 2 decimales, la API falla (500) en lugar de devolver un monto aproximado.
- El mayor general (`debe` / `haber`) todavía devuelve números JSON: es un contrato público aparte y queda pendiente.
