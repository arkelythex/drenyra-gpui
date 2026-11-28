# Route protection matrix

**Last updated**: 2026-11-27

How each mounted API surface authenticates callers and picks the tenant. The
source of truth is
[`apps/api/src/features/security/route-protection-matrix.ts`](../../../apps/api/src/features/security/route-protection-matrix.ts);
`route-protection-matrix.test.ts` fails when this page, the matrix and
`app-core.ts` / `api-module-surface.ts` disagree.

> **Risk first.** A row's auth mode is a fact about today's code, not a target.
> `legacy-header-fallback` means that outside production a caller may be
> identified by `x-user-id` / `x-user-role` / `x-company-id` headers alone;
> production accepts only a Better Auth session or a signed machine caller
> there. Treat those surfaces as hardening debt.

## Auth modes

| Mode                     | Meaning                                                                                          |
| ------------------------ | ------------------------------------------------------------------------------------------------ |
| `public`                 | No authentication (documented exceptions only).                                                  |
| `session`                | Better Auth session cookie.                                                                      |
| `bearer-tenant`          | Session plus a tenant header that must match the session tenant.                                 |
| `signed-machine`         | HMAC-signed machine caller from an allowlist.                                                    |
| `ai-surface`             | AI surface policy (`ai-surface-access.ts`), organization-scoped.                                 |
| `legacy-header-fallback` | Session first; outside production, header identity is also accepted.                             |
| `read-only-ledger-key`   | Read-only ledger key (`drn_ro_…`) on the listed GET routes only, in addition to the row's mode.  |

## Surfaces

| Surface               | Prefix                     | Export                     | Auth mode                | Tenant source       | Read-only ledger key | Rate limit  |
| --------------------- | -------------------------- | -------------------------- | ------------------------ | ------------------- | -------------------- | ----------- |
| Agentic Ledger        | `/api/agentic-ledger`      | `agenticLedgerModule`      | `ai-surface`             | `x-organization-id` | no                   | missing     |
| AI Control Plane      | `/api/v1/ai/control-plane` | `aiControlPlaneModule`     | `ai-surface`             | `x-organization-id` | no                   | missing     |
| AI RAG                | `/api/v1/ai/rag`           | `aiRagModule`              | `ai-surface`             | `x-organization-id` | no                   | documented  |
| AI Swarm              | `/api/ai-swarm`            | `aiSwarmRoutes`            | `ai-surface`             | `x-organization-id` | no                   | missing     |
| AI Workers            | `/api/ai-workers`          | `aiWorkersRoutes`          | `ai-surface`             | `x-organization-id` | no                   | documented  |
| Auth                  | `/api/auth`                | `authRoutes`               | `session`                | `session`           | no                   | missing     |
| Banking               | `/api/banking`             | `bankingRoutes`            | `legacy-header-fallback` | `x-company-id`      | no                   | documented  |
| Banking Providers     | `/api/banking-providers`   | `bankingProvidersRoutes`   | `legacy-header-fallback` | `x-company-id`      | no                   | missing     |
| Bills                 | `/api/bills`               | `billRoutes`               | `legacy-header-fallback` | `x-company-id`      | no                   | documented  |
| Cashflow              | `/api/cashflow`            | `cashflowRoutes`           | `legacy-header-fallback` | `x-company-id`      | no                   | documented  |
| Cognitive Stream      | `/api/v1/ai/stream`        | `cognitiveStreamRoute`     | `ai-surface`             | `x-organization-id` | no                   | missing     |
| Company Settings      | `/api/company-settings`    | `companySettingsRoute`     | `legacy-header-fallback` | `x-company-id`      | no                   | missing     |
| Context Control Plane | `/api/ai-swarm`            | `contextControlPlaneRoute` | `ai-surface`             | `x-organization-id` | no                   | missing     |
| Customers             | `/api/customers`           | `customerRoutes`           | `legacy-header-fallback` | `x-company-id`      | no                   | documented  |
| Dashboard             | `/api/dashboard`           | `dashboardModule`          | `legacy-header-fallback` | `x-company-id`      | no                   | documented  |
| Detractions           | `/api/detractions`         | `detractionsModule`        | `bearer-tenant`          | `x-company-id`      | no                   | missing     |
| Drenyra               | `/api/drenyra`             | `drenyraModule`            | `ai-surface`             | `x-organization-id` | no                   | missing     |
| Drenyra Console       | `/api/fiscal/console`      | `fiscalConsoleModule`      | `bearer-tenant`          | `x-company-id`      | no                   | missing     |
| Fiscal Truth          | `/api/fiscal/truth`        | `fiscalTruthModule`        | `bearer-tenant`          | `x-company-id`      | no                   | missing     |
| Frontend Telemetry    | `/api/telemetry`           | `frontendTelemetryModule`  | `signed-machine`         | `machine-scope`     | no                   | documented  |
| Health                | `/health`                  | `healthModule`             | `public`                 | `none`              | no                   | missing     |
| Invoices              | `/api/invoices`            | `invoiceRoutes`            | `legacy-header-fallback` | `x-company-id`      | no                   | documented  |
| Journal Entries       | `/api/journal-entries`     | `journalEntryRoutes`       | `bearer-tenant`          | `x-company-id`      | GET only (see below) | missing     |
| Ledger                | `/api/ledger`              | `ledgerModule`             | `legacy-header-fallback` | `mixed`             | GET only (see below) | missing     |
| Ledger Keys           | `/api/ledger-keys`         | `ledgerKeysRoutes`         | `session`                | `session`           | no (session only)    | missing     |
| Ledger MVP            | `/ledger-mvp`              | `ledgerMvpModule`          | `legacy-header-fallback` | `mixed`             | no                   | implemented |
| LLM Gateway           | `/api/v1`                  | `llmGatewayModule`         | `ai-surface`             | `x-organization-id` | no                   | documented  |
| Platform MCP          | `/api/platform/mcp`        | `platformMcpModule`        | `ai-surface`             | `x-organization-id` | no                   | missing     |
| PSE Compliance        | `/api/pse-compliance`      | `pseComplianceRoutes`      | `legacy-header-fallback` | `x-company-id`      | no                   | documented  |
| SIRE Audit            | `/api/ai-swarm`            | `sireAuditRoute`           | `ai-surface`             | `x-organization-id` | no                   | missing     |
| SUNAT API             | `/api/sunat/api`           | `sunatApiModule`           | `bearer-tenant`          | `x-company-id`      | no                   | missing     |
| Vendors               | `/api/vendors`             | `vendorRoutes`             | `legacy-header-fallback` | `x-company-id`      | no                   | documented  |

`Ledger` is mounted through `apiModules` (`api-module-surface.ts`), not
directly in `app-core.ts`; it is listed in `API_MODULES_ROUTE_PROTECTION_MATRIX`.

## Read-only ledger keys (`read-only-ledger-key`)

A read-only ledger key is a personal token for the native app, the CLI and
agents. It is issued by the Better Auth `@better-auth/api-key` plugin, stored
hashed, carries `ledger:read`, and is bound to exactly one company
(`metadata.companyId`).

Keys are issued, listed and revoked through `Ledger Keys`
(`/api/ledger-keys`), which accepts **only** a Better Auth session: a key can
never manage keys. How to issue, use and revoke a key:
[Read-only ledger keys — how-to](../ledger-read-keys.md).

### Routes that accept a key

| Route                              | Surface         |
| ---------------------------------- | --------------- |
| `GET /api/ledger/accounts`         | Ledger          |
| `GET /api/ledger/general`          | Ledger          |
| `GET /api/ledger/trial-balance`    | Ledger          |
| `GET /api/journal-entries/`        | Journal Entries |
| `GET /api/journal-entries/:id`     | Journal Entries |

HEAD is treated like GET. Every other route ignores keys, and every write on
an opted-in surface refuses them. `POST /api/ledger/export/pdf` and
`POST /api/ledger/export/xlsx` (Ledger MVP) do not accept keys.

### How a request with a key is decided

Send the key as `Authorization: Bearer drn_ro_…` or `x-api-key: drn_ro_…`.

1. A Better Auth session, when present, wins; the key is not read.
2. The method must be GET or HEAD — otherwise `403 API_KEY_READ_ONLY`, before
   the key is even looked up.
3. The key must verify with `ledger:read` (exists, enabled, not expired, under
   its rate limit) — otherwise `401 API_KEY_INVALID`, or `429
   API_KEY_RATE_LIMITED` with `Retry-After` (seconds until the key's window
   allows another request).
4. The key must carry `ledger:read` (`403 API_KEY_FORBIDDEN`) and a company
   (`403 API_KEY_NOT_COMPANY_BOUND`).
5. Its owner must still be an active member of that company at request time
   — otherwise `403 API_KEY_MEMBERSHIP_INACTIVE`.
6. `x-company-id`, `x-active-company-id` or a requested company that differs
   from the key's company — `403 API_KEY_TENANT_MISMATCH`. When
   `x-company-id` and `x-active-company-id` disagree with each other and a
   key is sent, the answer is the same `403 API_KEY_TENANT_MISMATCH`, decided
   before the key is looked up (never `401`/`500`).

The key decides identity and tenant on its own: `x-user-id`, `x-user-role` and
the other identity headers are ignored, and a refused key never falls back to
header identity. The caller gets role `viewer` and `readOnly: true` in its
company context. Production and non-production behave the same.

### Key properties

| Property       | Value                                                                                        |
| -------------- | -------------------------------------------------------------------------------------------- |
| Prefix         | `drn_ro_`                                                                                    |
| Storage        | SHA-256 hash only (`auth_api_keys.key`)                                                      |
| Sessions       | Never: `enableSessionForAPIKeys: false`                                                      |
| Default expiry | 90 days; at most 365 days                                                                    |
| Rate limit     | 60 verifications per minute per key (`429`, `Retry-After`)                                   |
| Issuance       | `POST /api/ledger-keys`, session only; plugin HTTP endpoints disabled                        |
| Shown once     | The plaintext key is in the issue response only                                              |
| Visible part   | `start`: `drn_ro_` plus 4 characters                                                         |
| Per company    | At most 10 active keys per user, enforced under a per (user, company) Postgres advisory lock |

### Rate limits

Two limits apply to a request with a key; both answer `429` with the standard
envelope (`{ "success": false, "error", "code" }`) and a `Retry-After` header
in seconds:

| Limit            | Scope                                              | Code                   |
| ---------------- | -------------------------------------------------- | ---------------------- |
| Per key          | 60 verifications per minute per key                | `API_KEY_RATE_LIMITED` |
| Global (all API) | 100 requests per minute per client (`app-core.ts`) | `RATE_LIMITED`         |

The per-key limit is lower, so it is the one a key holder meets. The global
limiter counts per client (the `Authorization` value, else
`x-forwarded-for`) on every route that reaches a handler.

### Key management rules (`/api/ledger-keys`)

Session only, as above. In addition, for the cookie-authenticated writes:

- `POST /api/ledger-keys` needs `Content-Type: application/json`, otherwise
  `415 UNSUPPORTED_MEDIA_TYPE`.
- `POST /api/ledger-keys` and `DELETE /api/ledger-keys/:id` with an `Origin`
  header outside the API's CORS allowlist (`apps/api/src/config/cors.ts`) get
  `403 ORIGIN_NOT_ALLOWED`. Requests without `Origin` are not affected.
- Issue and list responses carry `Cache-Control: no-store`.
- Concurrent issues for one user and company run one at a time; a wait over
  5 seconds answers `409 LEDGER_KEY_ISSUE_BUSY`.

Configuration lives in
[`apps/api/src/features/auth/lib/read-only-ledger-key.ts`](../../../apps/api/src/features/auth/lib/read-only-ledger-key.ts);
the caller branch in
[`apps/api/src/features/security/read-only-ledger-key.ts`](../../../apps/api/src/features/security/read-only-ledger-key.ts).
