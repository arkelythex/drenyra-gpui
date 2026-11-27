# Drenyra GPUI — Philosophy & Native Desktop Architecture

> **Last updated:** 2026-11-27 (native GPUI window, shell view model and commands; explicit approval replacement and session evidence log; Ledger, Evidence and CLI panes; posted records match the sent frame, overlays block company switches, stale-approval guard; evidence events carry the UTC time they were recorded).

> **Your accounting agents shouldn't live in a scrollback — or trapped in a browser tab.**
>
> **Drenyra GPUI** gives **Drenyra Shell (`drenyra-pi`)** and **Drenyra AI (`drenyra-ai`)** a native desktop home, built in Rust with [GPUI](https://gpui.rs/), inspired by the philosophy of [gentle-shell-gpui](https://github.com/Gentleman-Programming/gentle-shell-gpui) and adapted from first principles to verifiable accounting and financial engineering.

---

## 1. Core Thesis: The App Hosts, Shell & Kernel Do the Work

`gentle-shell-gpui` proved a decisive architectural principle for software engineering: **the native desktop app never reimplements the agent harness or the compiler**. Each tab spawns `gentle-shell --mode rpc` and talks to it over stdio, so the harness, orchestration, permissions, memory, review, and skills run unchanged while GPUI delivers a zero-latency, workspace-native operator surface.

**Drenyra GPUI applies the exact same boundary to accounting:**

1. **One Drenyra Shell per tab (RPC host).** Each agent tab launches `pi --mode rpc` (with the `drenyra-shell` pi package installed, when available) and speaks the agent's own stdio record protocol: LF-delimited `{"type": ...}` commands, `response` records that echo a string `id`, an event stream for each run until `agent_settled`, and `extension_ui_request` dialogs. It is not JSON-RPC 2.0. `drenyra-shell` is a pi package (extensions, skills, prompts), not a standalone binary. Persona, `/drenyra:*` commands, fiscal scope guards, skills (`drenyra-skills`), institutional memory (`drenyra-engram`), and Receipt-Driven Accounting (RDA) run unchanged inside the shell.
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
│   ├── drenyra-rpc/            # Supervised stdio JSON-RPC 2.0 session (timeouts, poisoning, response-id matching) and `agent`: the pi agent protocol client
│   ├── drenyra-ui/             # Accounting Generative UI state: Entry Diff Review, Reconciliation Table
│   └── drenyra-app/            # Desktop host binary: tenant-scoped controller, shell view model & commands,
│                               #   feature-gated GPUI window (--window), --check-env / --demo-session / --probe-rpc
├── vendor/rust/xattr/          # Temporary one-line patch for a GPUI transitive dependency (see PATCHED.md)
├── apps/
│   ├── api/                    # Elysia/Bun backend API (preserved during migration)
│   └── web/                    # Legacy TanStack Start web app + DEPRECATED src-tauri shell
├── packages/                   # Shared TS domain, persistence, fiscal rules & country packs
└── services/go/                # Go reconciliation worker
```

### Running the native window

```bash
cargo run -p drenyra-app --features gpui -- --window
```

The window currently runs on deterministic demo data (two fiscal workspaces and one staged R2 proposal, labelled **DEMO DATA**) until panes are wired to live `drenyra-shell` RPC sessions. Keys: `Ctrl+D` split along the focused pane's longest side, `Ctrl+W` close, `Ctrl+Shift+T` undo close, `Ctrl+Tab` next pane, `Ctrl+K` next workspace, `J`/`K` move through review lines, `A` accept, `R` reject, `Ctrl+B` sidebar, `Ctrl+Shift+P` command palette, `Ctrl+P` approve, `I` ask the agent, `Ctrl+C` stop the agent, `Ctrl+Shift+R` restart a stopped agent, `Ctrl+,` settings, `Ctrl+Q` quit. Linux needs the Wayland or X11 development libraries GPUI links against (`xkbcommon`, `wayland-client` or `xcb`, `fontconfig`, `freetype`, `vulkan`).

**Fiscal Agent pane (live).** The first time a fiscal workspace is shown, its Fiscal Agent pane starts `pi --mode rpc` (found on `PATH`; the `drenyra-shell` pi package plugs in when installed) and shows the model and status (Starting, Ready, Working, Stopped, Unavailable). Press `I` (or click the composer) to type a question; `Enter` sends, `Shift+Enter` adds a line, `Esc` stops typing, and `Ctrl+C` stops a running answer. When the agent stopped or could not start, the pane says so and `Ctrl+Shift+R` (or **Restart the agent** in the palette) starts a fresh process for that workspace only, keeping the transcript and the unsent draft; the previous process is closed off the UI thread. Single-key shortcuts are disabled while typing. Each workspace (one RUC in one fiscal period) has its own agent process and transcript, keyed by that RUC and period rather than by its position in the switcher, so a conversation about one company never appears under another; opening a second workspace for the same RUC and period is refused. Extension dialogs (`select`, `confirm`, `input`, `editor`) are declined and shown as a note until the window can render them. The window never answers for the operator; what an extension does after a declined dialog is up to that extension. Commands to the agent are queued for a dedicated writer thread that owns its stdin, so the window never blocks on an agent that stops reading. Closing the window closes each agent's stdin and waits up to 3 s before killing it. Without `pi`, the pane says so and the rest of the window keeps working.

**Sidebar.** Open by default and toggled with `Ctrl+B`, it lists every fiscal workspace (legal name, RUC, period, activity) with its staged entry and status (In review, Changes requested, Awaiting approval, Ready to post, Not balanced); clicking a company switches to it and leaves the previous company's composer. It reads other workspaces' reviews through the read-only `DrenyraHostController::review(index)`; nothing outside the active workspace can be changed from it. Documents and ledgers show a note until Drenyra Shell serves them.

**Ledger, Evidence and CLI panes.** `Ctrl+D` cycles new panes through Ledger, Evidence Vault and Fiscal CLI; each shows only real session data for the active workspace (one RUC in one period) and states its limits at the bottom. **Ledger** lists the staged entry (ID, risk tier, sidebar status, every line with exact `MoneyCents` debit/credit text, totals) and the entries posted in this session (ID, approver or "no approval required", `ledger_post` request id, totals); posted ledger history arrives when Drenyra Shell serves it. **Evidence Vault** shows the session evidence log newest first, one line per event with its sequence number, the UTC time it was recorded (`HH:MM:SSZ`) and a sentence (`#7  14:03:21Z  Posted je-… (request 1)`, `#2  14:02:58Z  Approved je-… by CPA-PERU-001`, …); it is in memory, unsigned and cleared when the window closes, so it is not a fiscal receipt. Both name the RUC and period when empty, and switching companies never shows another company's entries or events. **Fiscal CLI** shows the `--check-env` fields captured once when the window started (the same field list the flag prints) and one line per flag (`--check-env`, `--demo-session`, `--probe-rpc`, `--window`, the same table `--help` prints); the window runs no shell commands.

**Command palette.** `Ctrl+Shift+P` (or **⌘ Commands** in the top bar) lists every window command with its shortcut, a "Switch to …" entry per fiscal workspace and both themes. Typing filters with a case-insensitive subsequence match (`splt` finds "Split focused pane"); `Up`/`Down` choose, `Enter` runs, `Esc` closes. Commands run through the same `ShellState::apply` path as their shortcuts.

**Human approval (R2/R3).** `Ctrl+P` (or **Approve…** in the Entry Diff Review panel) opens the approval dialog for the active workspace's staged entry: entry ID, totals, risk tier and an Approver ID field. Approval is only accepted when the entry balances and every line is accepted; `Enter` records the typed ID through `EntryDiffReview::record_human_approval`, and `Esc` cancels. The ID is trimmed and must be 1 to 128 visible characters: control, zero-width and bidirectional-formatting characters are refused, so the recorded approver is exactly what a reader sees. It is otherwise recorded as typed (identity verification arrives with signed `drenyra-ai` receipts), and editing any line afterwards clears the approval. A recorded approval is never silently overwritten: `record_human_approval` refuses while one exists. When the entry is already approved, the dialog is titled "Replace approval" (footer "Enter replace approval") and shows "Approved by <id>. Confirming replaces that approval; both are recorded in this session's evidence log (in memory, unsigned)."; `Enter` with a different ID replaces it explicitly through `EntryDiffReview::replace_human_approval`, a compare-and-swap against the approver the dialog showed when it opened, and the same ID is refused with "Already approved by <id>.". If the approval changed while the dialog was open, `Enter` changes nothing and the dialog says "The approval changed to <id> while this dialog was open. Close it and review again.". If it was cleared by a line change meanwhile, the dialog says "The approval was cleared while this dialog was open. Close it and review again." and records nothing. While the approval dialog, the command palette or Settings is open, sidebar clicks do not switch companies, so typed text can never approve another company's entry (palette "Switch to …" entries still work: the palette closes first). Line decisions, approvals, replacements and posts all go through `DrenyraHostController`, but only some of them are logged: the session evidence log, keyed by the entry's RUC and period, records staging, approvals, replacements, an approval cleared by a line change, post requests and confirmed posts, each with a global sequence number and the UTC time it was recorded (Unix milliseconds from the controller's injected clock: the system clock in the app, a fixed clock in tests). Order events by sequence number: the system clock can step backwards, and the time is as trustworthy as the machine's clock (no trusted time source, no signature). A line decision that clears no approval is not logged. The controller also keeps the entries posted in the session; each posted record (entry and approver) is what the confirmed `ledger_post` frame carried, even if the approval was replaced or cleared while the post was in flight. The log is in memory and session-only: it is not persisted, signed or sent anywhere, so it is an operator aid, not a fiscal receipt.

**Settings › Themes.** `Ctrl+,` (or the **Settings** chip in the top bar) opens the Settings overlay. Its Themes section offers **Dreamcoder Dark** (default) and **Dreamcoder Light**; choose with `J`/`K` and `Enter`, or click a theme card. The choice is saved to `$XDG_CONFIG_HOME/drenyra-gpui/settings.toml` (falling back to `~/.config/drenyra-gpui/settings.toml`) and used on the next start; an unreadable or unknown value falls back to the default. Theme colors are copied verbatim from the canonical Dreamcoder palette (`dreamcoder-workbench/DreamcoderThemes/dreamcoder/tokens.json`), and `shell::theme` tests enforce WCAG AA contrast for every text and status role on every surface the window uses. `shell::window` defines no colors of its own.

**End-to-end check.** `scripts/drenyra-gpui-e2e.sh [output-dir]` runs the real window inside its own Xvfb display (never the operator's desktop), drives it with xdotool, asserts the non-visual outcomes (theme saved, `Ctrl+Q` exits 0, every agent process exits) and leaves screenshots of each step: approval blocked and recorded, palette, workspace switch, sidebar toggle, composer typing (`@`, `ñ`, letters that are shortcuts elsewhere), Settings › Themes. It needs Xvfb (`xorg-server-xvfb` on Arch), xdotool and ImageMagick; Vulkan presents through Mesa's software path (`MESA_VK_WSI_DEBUG=sw`) because Xvfb has no DRI3. No prompt is sent to the agent.

The root `Cargo.lock` is committed so the binary and its GPUI dependency tree build reproducibly; pass `--locked` in CI and release builds. Other Rust projects in the monorepo (for example `apps/web/src-tauri`) keep their lockfiles ignored.

### Design Invariants

- **Strict dependency direction:** `drenyra-app` → `drenyra-workspace` → `drenyra-protocol`. Neither `drenyra-protocol` nor `drenyra-workspace` may depend on UI/windowing crates.
- **Headless verifiability:** `drenyra-protocol` and `drenyra-workspace` run deterministic unit tests via `cargo test --workspace`; `drenyra-app --check-env` validates local RPC binaries (`drenyra-shell` / `pi`, `drenyra-ai`) and prints a machine-readable JSON health report without opening a display.
- **GPUI stays at the edge:** only `drenyra-app`'s `shell::window` module (behind the `gpui` Cargo feature) touches GPUI. `shell::view_model` and `shell::commands` derive everything the window renders and every key it handles in pure Rust, so the whole UI behavior is unit-tested without a display; the default build has no GPUI dependency.
- **No silent float coercion:** `MoneyCents` parses exact integer cents and refuses balanced-entry approval whenever `sum(debits) != sum(credits)` or any line is zero/invalid.

---

## 4. Phased Retirement of Legacy `src-tauri` & `apps/web`

| Phase | Milestone | Legacy Surface Status |
| --- | --- | --- |
| **Phase 1 (Complete)** | Repository renamed to `arkelythex/drenyra-gpui`; accounting philosophy contract authored; Rust Cargo workspace (`drenyra-protocol`, `drenyra-workspace`, `drenyra-app`) built and test-verified. | `apps/web/src-tauri` marked **deprecated**; `apps/api`, `packages/`, `services/go` remain untouched and operational. |
| **Phase 2** | Mount GPUI native window, translucent/solid studio themes, live subprocess supervisor for `drenyra-shell --mode rpc` and `drenyra-ai mcp serve`, Fiscal Workspace switcher (`⌃⌘K`), and dwindle pane tiling. | `apps/web/src-tauri` frozen (no new features). |
| **Phase 3** | Native Accounting Sidebar (`⌘B`), `drenyra_ui` Entry Diff Review, Reconciliation Tables, Dual-Approval R2/R3 dialogs, Guardian Angel status chips, and Immutable Ledger & Receipt Graph. | `apps/web` enters maintenance-only mode for browser-only stakeholders. |
| **Phase 4** | Production cutover to native `drenyra-gpui` binaries (macOS, Windows, Linux); remove `apps/web/src-tauri` and extract or archive `apps/web` if no browser portal is retained. | `drenyra-gpui` is the sole desktop surface; `apps/api`, `packages/`, and `services/go` serve as backend/kernel services. |
