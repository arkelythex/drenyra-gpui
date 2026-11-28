# Read-only ledger keys for the native app

Feature: `gpui-ledger-readonly-keys` | API branch: `feat/api-readonly-ledger-keys` (worktree `~/Documents/PROYECTOS/_worktrees/drenyra-gpui/api-readonly-ledger-keys`, from `origin/main`) | GPUI branch: stacked on #411 | Delivery: ask-on-risk, chained PRs (API first, GPUI client second) | Critical: auth + DB migration + new dependencies → test-first, independent verifier, high-risk RDD review.

## Specs

- **S1 — Native app reads the real Ledger.** User: "continua", answering the suggested next step (verbatim): "que los paneles Ledger y Evidence lean datos reales de Drenyra Shell en lugar de solo los de la sesión". Research (L1) found real read endpoints in `apps/api` (`/api/ledger/*`, `/api/journal-entries`) and no evidence endpoint.
- **S2 — Authentication by a read-only token.** User choice (verbatim option): "Token de solo lectura (Recomendado) — Nuevo token personal, con scope por empresa y solo lectura, emitido desde la web y guardado en el keyring del SO. Sirve también para CLI y agentes. Toca la auth del API (riesgo alto): test-first, verificador y revisión."
- **S3 — Evidence stays session-only.** User choice (verbatim option): "Solo sesión por ahora (Recomendado) — Ledger pasa a datos reales y Evidence sigue mostrando el log de la sesión hasta que exista un endpoint de recibos (drenyra-ai)."

Assumptions (parent, revisit in Log): implement S2 with the official `@better-auth/api-key` plugin (hashed keys, permissions, expiry, revocation, rate limit) instead of custom tokens; a key carries `permissions: { ledger: ["read"] }` and is bound to exactly one company (metadata), and the API accepts it only on allowlisted GET ledger/journal-entry routes for that company; every other route ignores or rejects it. "Emitido desde la web": issuance/revocation endpoints first (session-authenticated); a web settings UI is a separate later task unless trivial. Keyring storage on the native side is a new Rust dependency (decided in T5 with the lock in view).

## Tasks

| ID | Task | Specs | Route | Status | Commit |
| --- | --- | --- | --- | --- | --- |
| T1 | Map auth integration points (plugin wiring, caller resolver, company scope, migrations, tests) | S2 | delegated explore | in progress | — |
| T2 | API: api-key plugin + migration; read-only, company-bound key accepted only on allowlisted ledger GET routes | S1, S2 | worker + verify (items 1, 2, 5) | pending | — |
| T3 | API: issue / list / revoke own keys for a company (session-authenticated) + docs | S2 | worker + verify (item 2) | pending | — |
| T4 | API checks, verifier, RDD, PR | S2 | verify | pending | — |
| T5 | GPUI: Rust read-only client + key storage + Ledger pane shows real trial balance / journal entries for the active RUC + period | S1, S2, S3 | worker + verify (item 5) | pending | — |
| T6 | GPUI checks, E2E, RDD, PR | S1 | verify | pending | — |

## Log

- **L1 — 2026-11-27 research and decisions.** Explorer map recorded in `odd/tasks/drenyra-gpui-phase6.md` L28 (GPUI branch): read endpoints `GET /api/ledger/{accounts,general,trial-balance}`, `GET /api/journal-entries`; `companyScopeGuard({ allowHeaderFallback: true })`; Better Auth cookie sessions only; signed machine-caller path not enabled on these routes; no API key/PAT; no evidence endpoint; `db:seed` script missing. User chose S2 and S3 via the question tool. Context7 (Better Auth v1.6.23 docs): the API key plugin moved to `@better-auth/api-key` in 1.5 (table `apikey`, `referenceId`, `configId`), `auth.api.verifyApiKey({ body: { key, permissions } })`. Installed: better-auth 1.6.23 (root), `@better-auth/api-key` not installed. Rust lock has `hyper` 1.13, `http` 1.5, `rustls` 0.23; no `reqwest`/`ureq`/`keyring`.
