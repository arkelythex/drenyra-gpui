# Read-only ledger keys — how-to

**Last updated**: 2026-11-27

A read-only ledger key (`drn_ro_…`) lets the native app, the CLI or an agent
read **one company's** ledger without your session. You issue it with your
signed-in session, use it as a bearer token, and revoke it when you no longer
need it.

> **Risk first.** A key reads ledger data (chart of accounts, general ledger,
> trial balance, journal entries) of its company until it expires or you
> revoke it. The plaintext key is shown **once**, in the issue response:
> store it in the OS keyring or a secret manager, never in a repository,
> chat, log or URL. If a key leaks, revoke it right away.

Reference (auth decision order, error codes, accepted routes):
[Route protection matrix — read-only ledger keys](hardening/route-protection-matrix.md#read-only-ledger-keys-read-only-ledger-key).

## 1. Issue a key

You need a Better Auth session (sign in on the web app) and active membership
of the company. A key cannot issue keys.

```bash
curl -sS -X POST "$DRENYRA_API/api/ledger-keys" \
  -H "content-type: application/json" \
  -b "better-auth.session_token=$SESSION_COOKIE" \
  -d '{ "companyId": "<company uuid>", "name": "GPUI laptop", "expiresInDays": 30 }'
```

Body:

| Field           | Required | Rule                                                 |
| --------------- | -------- | ---------------------------------------------------- |
| `companyId`     | yes      | UUID of a company where you are an active member     |
| `name`          | yes      | 1–64 visible characters (trimmed)                    |
| `expiresInDays` | no       | Whole number 1–365; default 90                       |

Any other field (`permissions`, `metadata`, `prefix`, `userId`, limits) is
refused with `400 VALIDATION_ERROR`: the server alone decides that a key is
yours, read-only (`ledger:read`) and bound to that company.

Response (`201`), the only time `key` is returned:

```json
{
  "success": true,
  "data": {
    "id": "<key id>",
    "key": "drn_ro_…",
    "start": "drn_ro_AbCd",
    "name": "GPUI laptop",
    "companyId": "<company uuid>",
    "expiresAt": "2026-12-27T10:00:00.000Z"
  }
}
```

In production the session cookie is named
`__Secure-better-auth.session_token`.

## 2. Use the key

Send it as a bearer token (or as `x-api-key`) on the read routes:

```bash
curl -sS "$DRENYRA_API/api/ledger/trial-balance?asOfDate=2026-03-31" \
  -H "authorization: Bearer $DRENYRA_LEDGER_KEY"
```

The key alone decides the company. Do not send identity headers; if you send
`x-company-id`, it must be the key's company or the request is refused with
`403 API_KEY_TENANT_MISMATCH`. Writes are refused with `403
API_KEY_READ_ONLY`.

Routes that accept a key: `GET /api/ledger/accounts`,
`GET /api/ledger/general`, `GET /api/ledger/trial-balance`,
`GET /api/journal-entries/`, `GET /api/journal-entries/:id`.

## 3. List your keys

```bash
curl -sS "$DRENYRA_API/api/ledger-keys?companyId=<company uuid>" \
  -b "better-auth.session_token=$SESSION_COOKIE"
```

Returns only **your** keys for that company, without secrets: `id`, `name`,
`start`, `companyId`, `createdAt`, `expiresAt`, `lastRequest`, `enabled`.
Use `start` and `name` to recognise a key.

## 4. Revoke a key

```bash
curl -sS -X DELETE "$DRENYRA_API/api/ledger-keys/<key id>" \
  -b "better-auth.session_token=$SESSION_COOKIE"
```

`200 { "success": true, "data": { "id": "<key id>", "revoked": true } }`.
The key stops working on the next request (`401 API_KEY_INVALID`). Someone
else's key and an unknown id both answer `404 LEDGER_KEY_NOT_FOUND`. You can
revoke your keys even after you leave the company.

To rotate: issue a new key, switch the client to it, revoke the old one.

## Security properties

- **Stored hashed.** The database keeps a SHA-256 hash; a key cannot be read
  back from it.
- **Read-only and company-bound.** `ledger:read` only, one company, enforced
  on every request; a key never becomes a session.
- **Membership checked live.** If you lose membership of the company, its keys
  stop working (`403 API_KEY_MEMBERSHIP_INACTIVE`).
- **Session-only management.** `/api/ledger-keys` refuses any API key
  (`403 API_KEY_NOT_ALLOWED`) and never accepts header identity, in any
  environment.
- **No secrets in logs.** Issuance and revocation are logged with user,
  company and key ids only.

## Limits

| Limit                    | Value                                     |
| ------------------------ | ----------------------------------------- |
| Active keys              | 10 per user per company (`409` beyond)    |
| Lifetime                 | 1–365 days, default 90                    |
| Rate limit               | 120 requests per minute per key (`429`)   |
| Name                     | 1–64 visible characters                   |

The 10-key cap is checked before issuing, so two simultaneous issue requests
can end one key above it.

## Errors

| Status | Code                          | Meaning                                         |
| ------ | ----------------------------- | ----------------------------------------------- |
| 400    | `VALIDATION_ERROR`            | Body or query does not match the rules above    |
| 401    | `SESSION_REQUIRED`            | No signed-in session                            |
| 403    | `API_KEY_NOT_ALLOWED`         | An API key was sent to a management route       |
| 403    | `COMPANY_MEMBERSHIP_REQUIRED` | You are not an active member of that company    |
| 404    | `LEDGER_KEY_NOT_FOUND`        | Not your key, or no such key                    |
| 409    | `LEDGER_KEY_LIMIT_REACHED`    | Already 10 active keys for that company         |
| 500    | `LEDGER_KEY_ERROR`            | Unexpected failure; nothing secret is returned  |

Code:
[`apps/api/src/features/ledger-keys/`](../../apps/api/src/features/ledger-keys/)
(routes, schemas, service) and
[`apps/api/src/features/auth/lib/read-only-ledger-key.ts`](../../apps/api/src/features/auth/lib/read-only-ledger-key.ts)
(plugin options).
