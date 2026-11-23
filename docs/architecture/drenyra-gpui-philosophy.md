# Drenyra GPUI — Philosophy & Native Desktop Architecture

> **Your accounting agents shouldn't live in a scrollback — or trapped in a browser tab.**
>
> **Drenyra GPUI** gives **Drenyra Shell (`drenyra-pi`)** and **Drenyra AI (`drenyra-ai`)** a native desktop home, built in Rust with [GPUI](https://gpui.rs/), inspired by the philosophy of [gentle-shell-gpui](https://github.com/Gentleman-Programming/gentle-shell-gpui) and adapted from first principles to verifiable accounting and financial engineering.

---

## 1. Core Thesis: The App Hosts, Shell & Kernel Do the Work

`gentle-shell-gpui` proved a decisive architectural principle for software engineering: **the native desktop app never reimplements the agent harness or the compiler**. Each tab spawns `gentle-shell --mode rpc` over JSON-RPC stdio, so the harness, orchestration, permissions, memory, review, and skills run unchanged while GPUI delivers a zero-latency, workspace-native operator surface.

**Drenyra GPUI applies the exact same boundary to accounting:**

1. **One Drenyra Shell per tab (RPC host).** Each agent tab launches `drenyra-shell` / `pi --mode rpc` over JSON-RPC 2.0 stdio. Persona, `/drenyra:*` commands, fiscal scope guards, skills (`drenyra-skills`), institutional memory (`drenyra-engram`), and Receipt-Driven Accounting (RDA) run unchanged inside the shell.
2. **Deterministic fiscal kernel over stdio MCP.** Whenever the UI inspects ledger state, validates double-entry invariants, verifies receipts, or advances missions, it speaks JSON-RPC 2.0 to `drenyra-ai mcp serve` — never reimplementing tax or accounting logic in the desktop layer.
3. **Human authority is non-delegable.** Agents propose; deterministic validators check; certified professionals approve material decisions (R2/R3); `drenyra-ai` emits a signed, offline-verifiable receipt. Memory (`drenyra-engram`) informs; it never authorizes.
4. **Zero-float monetary invariant.** Every monetary amount crossing `drenyra-protocol` is represented as signed integer cents (`MoneyCents(i128)`), matching Drenyra's ecosystem-wide BigInt-cent rule. Floating-point numbers are forbidden for money.

---

## 2. 1:1 Adaptation: `gentle-shell-gpui` → `drenyra-gpui`

| `gentle-shell-gpui` (Software Engineering) | `drenyra-gpui` (Accounting & Financial Engineering) | Operational Purpose |
| --- | --- | --- |
| **Named Project Workspaces** (code repo + folder + tabs) | **Fiscal Workspaces** (Legal Entity / RUC / Tax Regime / Fiscal Period) | Switch between client companies or periods without stopping background reconciliations. Activity dot shows reconciling (accent) or awaiting approval (amber). |
| **Hyprland Dwindle Tiling Panes** (Agent, Terminal, File) | **Accounting Dwindle Panes** (Fiscal Agent, Ledger/Trial Balance, Evidence Viewer, Fiscal CLI) | Split along the longest side (`⌘D`), drag to swap or dock, undo close (`⌥⌘T`). No noisy pane chrome. |
| **Files / Changes / Branches Sidebar (`⌘B`)** | **Documents & Ledgers / Unposted Entries / Fiscal Periods & Scenarios (`⌘B`)** | Browse source vouchers (XML, PDF, CDR, bank statements), stage/unstage proposed journal entries, and isolate period adjustments or what-if scenarios like Git worktrees. |
| **Git Commit Graph** | **Immutable Ledger & Receipt Graph** | Atomic journal entries as commits, review bundles as PRs, period closings as releases, and signed execution receipts as verified tags. |
| **Change Review (`gentle_ui` diff hunks)** | **Accounting Entry Diff Review (`drenyra_ui`)** | Accept, reject, or comment on each proposed journal line (`j/k`, `a/r`, `c`) with live `debit == credit` (`MoneyCents`) verification before posting. |
| **Generative UI Panels** (Diagrams, Tables, Forms, Checklists, Gantt) | **Accounting Generative UI** (Evidence Flow Diagrams, Reconciliation Tables, Dual-Approval Forms, Period-Close Run Plans, Tax Deadline Timelines) | Native GPUI surfaces when text is not enough: SUNAT/SIRE/PLE filing calendars, bank-to-ledger matching tables, and R2/R3 sign-off forms. |
| **Session Chips & Todos Island** | **Fiscal Guardrail Chips & Close Checklist Island** | Active Entity/RUC/Period chip, **Approval Gate (R0–R3)** chip, **RDA (Receipt-Driven Accounting)** chip, **Guardian Angel** dual-review chip, **Unposted Entries** chip, **Helpers (Subagents)** chip, and floating period-close checklist. |
| **Command Palette (`⌘K`)** | **Fiscal Command Palette (`⌘K`)** | Instant access to `/drenyra:*` commands, country-pack skills, entities, ledgers, accounts (PCGE), and periods. |
| **In-App Instances / Previews** | **Evidence & Filing Previews** | Non-intrusive floating cards for UBL XML, PDF invoices, CDR receipts, and tax form previews without stealing focus from the ledger. |

---

## 3. Rust Workspace Architecture (`crates/`)

Drenyra GPUI mirrors `gentle-shell-gpui`'s clean crate separation so domain invariants and RPC framing stay 100% testable without requiring a live GPU/windowing session:

```text
drenyra-gpui/
├── Cargo.toml                  # Root Rust workspace (native GPUI host)
├── crates/
│   ├── drenyra-protocol/       # JSON-RPC 2.0 stdio transport, MoneyCents(i128), Entry Diff & Receipt contracts
│   ├── drenyra-workspace/      # Fiscal Workspaces, Hyprland dwindle tiling tree, Sidebar, Session Chips & RDA gate
│   └── drenyra-app/            # Native desktop host binary (GPUI shell mount + --check-env headless diagnostics)
├── apps/
│   ├── api/                    # Elysia/Bun backend API (preserved during migration)
│   └── web/                    # Legacy TanStack Start web app + DEPRECATED src-tauri shell
├── packages/                   # Shared TS domain, persistence, fiscal rules & country packs
└── services/go/                # Go reconciliation worker
```

### Design Invariants

- **Strict dependency direction:** `drenyra-app` → `drenyra-workspace` → `drenyra-protocol`. Neither `drenyra-protocol` nor `drenyra-workspace` may depend on UI/windowing crates.
- **Headless verifiability:** `drenyra-protocol` and `drenyra-workspace` run deterministic unit tests via `cargo test --workspace`; `drenyra-app --check-env` validates local RPC binaries (`drenyra-shell` / `pi`, `drenyra-ai`) and prints a machine-readable JSON health report without opening a display.
- **No silent float coercion:** `MoneyCents` parses exact integer cents and refuses balanced-entry approval whenever `sum(debits) != sum(credits)` or any line is zero/invalid.

---

## 4. Phased Retirement of Legacy `src-tauri` & `apps/web`

| Phase | Milestone | Legacy Surface Status |
| --- | --- | --- |
| **Phase 1 (Complete)** | Repository renamed to `arkelythex/drenyra-gpui`; accounting philosophy contract authored; Rust Cargo workspace (`drenyra-protocol`, `drenyra-workspace`, `drenyra-app`) built and test-verified. | `apps/web/src-tauri` marked **deprecated**; `apps/api`, `packages/`, `services/go` remain untouched and operational. |
| **Phase 2** | Mount GPUI native window, translucent/solid studio themes, live subprocess supervisor for `drenyra-shell --mode rpc` and `drenyra-ai mcp serve`, Fiscal Workspace switcher (`⌃⌘K`), and dwindle pane tiling. | `apps/web/src-tauri` frozen (no new features). |
| **Phase 3** | Native Accounting Sidebar (`⌘B`), `drenyra_ui` Entry Diff Review, Reconciliation Tables, Dual-Approval R2/R3 dialogs, Guardian Angel status chips, and Immutable Ledger & Receipt Graph. | `apps/web` enters maintenance-only mode for browser-only stakeholders. |
| **Phase 4** | Production cutover to native `drenyra-gpui` binaries (macOS, Windows, Linux); remove `apps/web/src-tauri` and extract or archive `apps/web` if no browser portal is retained. | `drenyra-gpui` is the sole desktop surface; `apps/api`, `packages/`, and `services/go` serve as backend/kernel services. |
