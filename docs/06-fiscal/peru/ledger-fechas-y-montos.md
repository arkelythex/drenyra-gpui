# Ledger API — días fiscales y montos exactos

**Última actualización:** 2026-11-27
**Alcance:** `GET /api/ledger/trial-balance`, `GET /api/ledger/general`, las exportaciones `POST /api/ledger/export/{pdf,xlsx}`, la herramienta de agente `get_ledger_entry` y `PATCH /api/company/:companyId/settings` (`timezone`) (`apps/api/src/features/ledger/`, `apps/api/src/features/ledger-mvp/`, `apps/api/src/features/drenyra/tools/finance.tools.ts`, `apps/api/src/features/company/`)

---

## Respuesta corta

- Una fecha `YYYY-MM-DD` significa **el día calendario completo** en la zona horaria fiscal de la empresa (`companies.settings_timezone`, por defecto `America/Lima`).
- La balanza "al 2026-03-31" incluye todo lo emitido hasta las 23:59:59.999 de Lima del 31 de marzo; nada del 1 de abril.
- `debits`, `credits` y `balance` **de la balanza** son texto decimal exacto con 2 decimales (`"1234.50"`, `"-150.25"`), sumados en Postgres sobre `numeric`. Nunca pasan por números de punto flotante de JavaScript ni usan notación exponencial.
- **Pendiente (no resuelto aún):** el mayor general (`debe` / `haber`) y los totales de sus exportaciones PDF/XLSX **todavía usan números de JavaScript** (ver [Montos](#montos)).

## Por qué importa

Antes, la API comparaba `issue_date <= YYYY-MM-DDT00:00:00Z`. Eso dejaba fuera **todo el último día del período en Perú** (UTC−5), y sumaba montos con floats de JavaScript (`toFixed(2)`), que pierden centavos en totales grandes. Una balanza incompleta o con centavos de más es un error contable, no cosmético.

## Fechas de entrada

| Valor enviado | Significado |
|---|---|
| `2026-03-31` | Día calendario completo en la zona fiscal de la empresa (forma recomendada). |
| `2026-03-31T00:00:00.000Z` (exactamente medianoche UTC) | El día calendario `2026-03-31`, igual que `2026-03-31`. Es el valor que la API devuelve en `asOfDate` y el que produce `new Date("2026-03-31")`, así que reenviarlo significa el mismo día. |
| `2026-04-01T04:59:59.999Z` (cualquier otra fecha-hora ISO) | Instante; se usa el día calendario que le corresponde en la zona fiscal de la empresa (aquí, 31 de marzo en Lima). |
| `2026-02-30`, `invalid` | `400 INVALID_DATE`. Un día que no existe ya no se reinterpreta como otro día. En las exportaciones PDF/XLSX tampoco se ignora: antes exportaban todo el historial con `200`; ahora responden `400 INVALID_DATE` sin consultar el mayor. |

- `asOfDate` (balanza): incluye el día indicado completo (acumulado hasta ese día).
- `startDate` / `endDate` (mayor general y exportaciones): ambos incluidos; `startDate` empieza a las 00:00 del día en la zona fiscal.
- El orden `startDate ≤ endDate` se decide sobre **días fiscales ya resueltos en la zona de la empresa**, no sobre instantes. Con límites mixtos (día + instante) en Lima: `startDate=2026-03-31&endDate=2026-03-31T03:00:00.000Z` (el instante es 30 de marzo en Lima) responde `400 INVALID_DATE_RANGE`, en vez de devolver un mayor vacío con `200`; `startDate=2026-04-01T03:00:00.000Z&endDate=2026-03-31` (22:00 del 31 de marzo en Lima) se acepta y devuelve el 31 de marzo, en vez de rechazarse. Dos días calendario se comparan sin consultar la base; si hay un instante, decide la consulta. Las exportaciones aplican la misma regla también con sus valores por defecto: un `startDate` futuro sin `endDate` (hasta ahora) responde `400 INVALID_DATE_RANGE` en lugar de un archivo vacío.
- La herramienta de agente `get_ledger_entry` comparte la consulta: un rango invertido en días fiscales la hace fallar en vez de devolver una lista vacía.
- La respuesta mantiene `asOfDate` como antes: `"2026-03-31T00:00:00.000Z"` para `2026-03-31`.
- Las exportaciones sin fecha siguen usando desde el inicio (`1970-01-01`) hasta ahora.
- La herramienta de agente `get_ledger_entry` interpreta sus fechas igual que el endpoint (día calendario completo); una fecha inexistente la hace fallar sin consultar el mayor. Sin fechas, usa del 1 de enero del año en curso hasta ahora.

## Zona horaria fiscal de la empresa

- Los reportes usan `companies.settings_timezone` **solo si, quitando espacios al inicio y al final, coincide exactamente con un nombre de `pg_timezone_names`** del servidor Postgres. Si no (vacío, `"Lima"`, `"asia/tokyo"` con otra capitalización, valores antiguos guardados sin validar), usan `America/Lima`. Antes, un valor desconocido hacía fallar la balanza y el mayor con `500` y texto SQL en la respuesta.
- La zona se resuelve una vez por reporte: se lee `settings_timezone` de la empresa pedida y se compara con los nombres de `pg_timezone_names`, que cada proceso guarda en caché durante 1 hora (recorrer esa vista cuesta ~13–50 ms por llamada; llamadas concurrentes comparten un solo recorrido y una respuesta vacía no se guarda).
- Si un valor guardado no vacío cae a `America/Lima`, se registra una advertencia con `companyId`, el valor guardado (sin espacios, máximo 64 caracteres) y la zona usada, para que el valor se pueda corregir.
- Los filtros por día llevan además un prefiltro sobre la columna `issue_date` (hora UTC sin zona), unido con `AND` al filtro exacto por día fiscal: en el mayor, `issue_date >= inicio − 1 día AND issue_date < fin + 2 días`; en la balanza acumulada, solo `issue_date < asOf + 2 días`. Una fila del día fiscal D siempre cae en [D − 1, D + 2) porque ninguna zona se aleja 24 h de UTC, así que el resultado no cambia (pruebas con Lima, Santiago, Tokio, UTC+14 y UTC−12) y un índice sobre `issue_date` podría usarse. Hoy esa columna no tiene índice propio.
- `PATCH /api/company/:companyId/settings` valida `timezone`: se quitan espacios y el nombre debe existir en `pg_timezone_names`; si no, responde `400` con `{ "error": "Unknown time zone. Use an IANA time zone name such as America/Lima.", "code": "INVALID_TIMEZONE" }` y no guarda nada. Se valida contra Postgres y no contra `Intl.supportedValuesOf("timeZone")` porque quien usa la zona es Postgres, y ambas listas difieren (por ejemplo, el servidor tiene `Etc/UTC` y no `US/Eastern`; `Intl` al revés). Así, "aceptado" equivale a "respetado por el ledger".
- Un error de consulta en `/api/ledger/*` responde `500` con el mensaje fijo `"Ledger query failed"` (`INTERNAL_ERROR`); el registro guarda el nombre del error, el código y la severidad del driver (por ejemplo el SQLSTATE de Postgres, `22P02`) y la primera línea del mensaje del driver saneada (sin texto SQL, sin el volcado `params:` de Drizzle y con los valores entre comillas reemplazados por `"?"`), nunca el SQL ni sus parámetros.

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

La misma regla se aplica a las fechas pedidas: una fecha-hora enviada exactamente a las `00:00:00.000` UTC es la etiqueta de ese día (ver [Fechas de entrada](#fechas-de-entrada)).

**Ambigüedad conocida:** un instante guardado o enviado exactamente a las `00:00:00.000000` UTC (19:00 en Lima) se lee como etiqueta del día UTC. La solución definitiva es que los escritores guarden una fecha de documento explícita (columna `date`), lo que requiere una migración aparte.

## Montos

- Se suman en SQL (`SUM` sobre `numeric(12,2)`), se redondean a 2 decimales y se devuelven como `::text`.
- Los totales de la balanza se calculan en la misma consulta (función de ventana) sobre las mismas cuentas listadas.
- Si Postgres devolviera algo que no es texto decimal de 2 decimales, la API falla (500) en lugar de devolver un monto aproximado.
- **Pendiente:** el mayor general (`debe` / `haber`) todavía convierte `numeric` a número de JavaScript y lo devuelve como número JSON; los totales de las exportaciones PDF/XLSX del mayor se acumulan con `+=` sobre esos números y se imprimen con `toFixed(2)`. Esto sigue violando la regla de no usar floats para dinero; cambiarlo afecta un contrato público aparte y queda como seguimiento. Este cambio solo hace exacta la balanza.
