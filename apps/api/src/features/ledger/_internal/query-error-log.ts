/**
 * Log fields for a failed ledger query, safe to write to the logs.
 *
 * Driver errors are useful to diagnose (SQLSTATE, severity, the Postgres
 * message), but Drizzle wraps them in an error whose message is the SQL text
 * plus its parameters, and some Postgres messages quote input values. Only the
 * error name, the driver code and severity, and a sanitized first line of the
 * driver message are kept.
 */

/** Fields written by `ledgerQueryFailed`. */
export interface QueryErrorLogFields {
	errorName: string;
	errorCode?: string;
	errorSeverity?: string;
	errorMessage?: string;
}

const CODE_PATTERN = /^[0-9A-Z_]{1,40}$/;
const SEVERITY_PATTERN = /^[A-Z]{1,10}$/;
const MAX_CAUSE_DEPTH = 5;
const MAX_MESSAGE_LENGTH = 200;
/** Start of SQL text or of Drizzle's `params:` dump inside a message. */
const SQL_TEXT =
	/\b(SELECT|INSERT|UPDATE|DELETE|WITH|FROM|WHERE|VALUES|RETURNING)\b|params:/;
const QUOTED = /"[^"]*"|'[^']*'/g;

function stringField(value: object, key: string): string | undefined {
	const field: unknown = Reflect.get(value, key);
	return typeof field === "string" ? field : undefined;
}

/** First error in the `cause` chain that carries a driver code. */
function driverError(error: unknown): Error | undefined {
	let current: unknown = error;
	for (let depth = 0; depth < MAX_CAUSE_DEPTH; depth++) {
		if (!(current instanceof Error)) return undefined;
		const code = stringField(current, "code");
		if (code !== undefined && CODE_PATTERN.test(code)) return current;
		current = current.cause;
	}
	return undefined;
}

/**
 * First line of an error message without SQL text, parameters, or quoted
 * values (`"abc"` → `"?"`), at most 200 characters.
 */
export function sanitizeQueryErrorMessage(message: string): string {
	const firstLine = message.split("\n", 1)[0] ?? "";
	if (/^Failed query\b/.test(firstLine))
		return "Failed query (SQL text redacted)";
	const sqlStart = firstLine.search(SQL_TEXT);
	const prose = sqlStart === -1 ? firstLine : firstLine.slice(0, sqlStart);
	return prose
		.replace(QUOTED, (quoted) => (quoted.startsWith('"') ? '"?"' : "'?'"))
		.replace(/[\s:]+$/, "")
		.slice(0, MAX_MESSAGE_LENGTH);
}

/**
 * Safe log fields for a query failure: the driver error's name, code,
 * severity and sanitized message when the `cause` chain has one, otherwise
 * the top-level error's name and sanitized message.
 */
export function describeQueryError(error: unknown): QueryErrorLogFields {
	if (!(error instanceof Error)) return { errorName: typeof error };
	const source = driverError(error) ?? error;
	const fields: QueryErrorLogFields = { errorName: source.name };
	const code = stringField(source, "code");
	if (code !== undefined && CODE_PATTERN.test(code)) fields.errorCode = code;
	const severity = stringField(source, "severity");
	if (severity !== undefined && SEVERITY_PATTERN.test(severity)) {
		fields.errorSeverity = severity;
	}
	const message = sanitizeQueryErrorMessage(source.message);
	if (message.length > 0) fields.errorMessage = message;
	return fields;
}
