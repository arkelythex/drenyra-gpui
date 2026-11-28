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
   API_KEY_RATE_LIMITED`.
4. The key must carry `ledger:read` (`403 API_KEY_FORBIDDEN`) and a company
   (`403 API_KEY_NOT_COMPANY_BOUND`).
5. Its owner must still be an active member of that company at request time
   — otherwise `403 API_KEY_MEMBERSHIP_INACTIVE`.
6. `x-company-id`, `x-active-company-id` or a requested company that differs
   from the key's company — `403 API_KEY_TENANT_MISMATCH`.

The key decides identity and tenant on its own: `x-user-id`, `x-user-role` and
the other identity headers are ignored, and a refused key never falls back to
header identity. The caller gets role `viewer` and `readOnly: true` in its
company context. Production and non-production behave the same.

### Key properties

| Property       | Value                                                      |
| -------------- | ---------------------------------------------------------- |
| Prefix         | `drn_ro_`                                                  |
| Storage        | SHA-256 hash only (`auth_api_keys.key`)                    |
| Sessions       | Never: `enableSessionForAPIKeys: false`                    |
| Default expiry | 90 days; at most 365 days                                  |
| Rate limit     | 120 verifications per minute per key                       |
| Issuance       | Server-side only; the plugin's HTTP endpoints are disabled |

Configuration lives in
[`apps/api/src/features/auth/lib/read-only-ledger-key.ts`](../../../apps/api/src/features/auth/lib/read-only-ledger-key.ts);
the caller branch in
[`apps/api/src/features/security/read-only-ledger-key.ts`](../../../apps/api/src/features/security/read-only-ledger-key.ts).
