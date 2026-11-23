# Drenyra GPUI — Rename & Native Rust/GPUI Accounting Host Pivot

Feature: `drenyra-gpui-pivot` | Repo: `arkelythex/drenyra-gpui` (renamed from `arkelythex/drenyra-console`) | Status: in_progress

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
| T1 | Rename GitHub repo, local dir, origin, and repair linked worktrees | S1 | inline | in_progress | - |
| T2 | Update repo metadata, Go module, CI scripts, Tauri deprecation, and sibling refs | S1, S4 | inline | pending | - |
| T3 | Author `drenyra-gpui` accounting philosophy & architecture doc + README/philosophy updates | S2 | inline | pending | - |
| T4 | Build test-first Rust Cargo workspace (`drenyra-protocol`, `drenyra-workspace`, `drenyra-app`) | S3 | inline | pending | - |
| T5 | Independent verification (`gentle-ai-verify`) and native review assessment | S1-S4 | verify | pending | - |

## Log

- **L1 — Original user request (verbatim):**
  "hey bro drenyra-consolo renombralo a drenyra gpui , debe inspirarse en la filosofia de gentle-shell que es para la ingeneria de softwart pero adaptarse para la contabilidad , https://github.com/Gentleman-Programming/gentle-shell-gpui"
- **L2 — User scope answers (`ask_user_question`):**
  1. Renombre: `GitHub + local + ecosistema (completo)`
  2. Alcance GPUI: `Pivote completo a app nativa Rust/GPUI`
