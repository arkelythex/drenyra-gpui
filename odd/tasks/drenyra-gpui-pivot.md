# Drenyra GPUI — Rename & Native Rust/GPUI Accounting Host Pivot

Feature: `drenyra-gpui-pivot` | Repo: `arkelythex/drenyra-gpui` (renamed from `arkelythex/drenyra-console`) | Status: done

## Specs

- **S1 — Full repository & ecosystem rename (`drenyra-console` → `drenyra-gpui`).**
  Rename GitHub repository `arkelythex/drenyra-console` to `arkelythex/drenyra-gpui`, move local directory `drenyra-console/` to `drenyra-gpui/`, update `origin` remote URL, repair linked git worktrees, and update repository URLs, metadata, Go module path, CI scripts, README, and sibling repo references (`Arkelythex/README.md`, `Arkelythex/docs/ARCHITECTURE.md`, `Arkelythex/.agents/skills/architecture-design/SKILL.md`). Preserve immutable database migration files and journal tags (`packages/infrastructure/drizzle/**`, `0006_drenyra_command_center`).
- **S2 — Accounting-adapted `gentle-shell-gpui` philosophy & architecture.**
  Author `docs/architecture/drenyra-gpui-philosophy.md` (and link it from `README.md` and `docs/products/drenyra-product-philosophy.md`) adapting `https://github.com/Gentleman-Programming/gentle-shell-gpui` ("The app hosts, Shell does the work") to verifiable accounting: 1 Drenyra Shell per tab over JSON-RPC stdio + `drenyra-ai mcp serve`, Fiscal Workspaces (Entity/RUC/Period) & dwindle tiling panes, Accounting Sidebar (Documents/Ledgers, Unposted Entries, Fiscal Periods/Scenarios), hash-chained Ledger & Receipt Graph, Accounting Generative UI (`drenyra_ui` with BigInt-cent Entry Diff review), Session Chips & Receipt-Driven Accounting (RDA) / Guardian Angel guardrails, and a 4-phase retirement roadmap for `apps/web/src-tauri` and `apps/web`.
- **S3 — Native Rust + GPUI workspace foundation (`crates/`).**
  Create a Cargo workspace at the repository root with `crates/drenyra-protocol` (JSON-RPC 2.0 stdio transport + `MoneyCents(i128)` invariant + accounting review/receipt protocol types), `crates/drenyra-workspace` (Fiscal Workspace, dwindle pane tiling tree, Accounting Sidebar, Session Chips, RDA approval gate state), and `crates/drenyra-app` (native host binary entrypoint with `--check-env` headless verification mode). Every crate must pass `cargo test --workspace` and `cargo clippy --workspace --all-targets`.
- **S4 — Phased deprecation of legacy Tauri/Web shell.**
  Mark `apps/web/src-tauri` as deprecated in favor of `crates/drenyra-app` and update its package identity to `drenyra-gpui` without breaking existing `apps/api` or `packages/` workspaces.

## Tasks

| ID | Task | Specs | Route | Status | Commit |
| --- | --- | --- | --- | --- | --- |
| T1 | Rename GitHub repo, local dir, origin, and repair linked worktrees | S1 | inline | done | `bc70a968` |
| T2 | Update repo metadata, Go module, CI scripts, Tauri deprecation, and sibling refs | S1, S4 | inline | done | `bc70a968` |
| T3 | Author `drenyra-gpui` accounting philosophy & architecture doc + README/philosophy updates | S2 | inline | done | `bc70a968` |
| T4 | Build test-first Rust Cargo workspace (`drenyra-protocol`, `drenyra-workspace`, `drenyra-app`) | S3 | inline | done | `4e71a216` |
| T5 | Independent verification (`gentle-ai-verify`) and native review assessment | S1-S4 | verify | done | `review-21e554dbb30ad94d` (approved/burned) |

## Log

- **L1 — Original user request (verbatim):**
  "hey bro drenyra-consolo renombralo a drenyra gpui , debe inspirarse en la filosofia de gentle-shell que es para la ingeneria de softwart pero adaptarse para la contabilidad , https://github.com/Gentleman-Programming/gentle-shell-gpui"
- **L2 — User scope answers (`ask_user_question`):**
  1. Renombre: `GitHub + local + ecosistema (completo)`
  2. Alcance GPUI: `Pivote completo a app nativa Rust/GPUI`
- **L3 — Execution & T5 verification/review state:**
  - Slice 1 (`bc70a968`): GitHub repo renamed to `arkelythex/drenyra-gpui`, local dir moved to `drenyra-gpui/`, 4 linked worktrees repaired, Go module + CI scripts + Tauri deprecation + `docs/architecture/drenyra-gpui-philosophy.md` + `README.md` updated. Sibling refs updated in `Arkelythex/` (uncommitted in `Arkelythex/`).
  - Slice 2 (`4e71a216`): Rust Cargo workspace (`drenyra-protocol`, `drenyra-workspace`, `drenyra-app`) built test-first; 9/9 tests pass, `cargo clippy --workspace --all-targets -- -D warnings` passes, `go test ./...` passes, `./scripts/ci/odd-guard.sh` passes, DB migrations untouched.
  - T5 closed: `review-21e554dbb30ad94d` resumed and closed with `acknowledge-approved` (`authority: burned`, `burn_evidence: gentle-ai.review-acknowledged/v1`) across `review-risk`, `review-readability`, and `review-reliability` (9 non-blocking advisories logged for Phase 2 hardening). Independent `gentle-ai-verify` (`mwsqds7s-1-dc0g`) ran and verified S1, S3, and S4 as MET, and flagged one S2 gap (missing cross-link in `docs/products/drenyra-product-philosophy.md`), which was fixed immediately.
