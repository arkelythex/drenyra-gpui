# Drenyra GPUI — Phase 5: Native GPUI Window (Fiscal Workspaces, Dwindle Panes, Entry Review)

Feature: `drenyra-gpui-phase5` | Branch: `feat/drenyra-gpui-phase5` stacked on `fix/gpui-rpc-response-id` (PR #398) | Delivery: `ask-on-risk` | Runner: `cargo test --workspace --all-features` | Status: in_progress

## Specs

- **S1 — Headless-first feature boundary.** `gpui = "0.2.2"` (crates.io) is an optional dependency of `drenyra-app` enabled only by `--features gpui`. The default build stays headless with no GPUI dependency. `drenyra-protocol`, `drenyra-workspace`, `drenyra-rpc` and `drenyra-ui` never depend on GPUI (architecture invariant, `docs/architecture/drenyra-gpui-philosophy.md` §3).
- **S2 — Pure shell view model (container-presentational).** A GPUI-free `shell` module in `drenyra-app` derives everything the window renders from `DrenyraHostController`: workspace chips (legal name, RUC, period, activity, active marker), guardrail chips (RDA, Guardian Angel, active risk tier, unposted entries, background alert), integer-pixel pane rectangles from the dwindle tree with the focused pane marked, and Entry Diff Review rows (account, description, debit, credit as `MoneyCents` text, decision) with balance and `can_post` state. Fully unit-tested headless.
- **S3 — Pure shell commands.** A GPUI-free command layer applies operator intents to the controller: split focused pane along its real longest side (dwindle, using the S2 rectangle), close focused pane, undo close, focus next pane, switch to next workspace, move review cursor, accept / reject the line under the cursor. Fully unit-tested headless.
- **S4 — Native window (`--window`).** With `--features gpui`, `drenyra-gpui --window` opens a native GPUI window that renders S2 and maps keys to S3 commands (Ctrl+D split, Ctrl+W close, Ctrl+Shift+T undo close, Ctrl+Tab focus next pane, Ctrl+K next workspace, J/K review cursor, A accept, R reject). It starts from deterministic demo data (two workspaces, one staged R2 proposal) clearly labelled as demo, because no live `drenyra-shell` RPC exists yet. Without the feature, `--window` exits 1 with a message telling how to rebuild. `--help` documents the flag.
- **S5 — Verification.** `cargo test --workspace --all-features`, clippy `-D warnings` with and without features, fmt, the window launched on this machine and visually checked, independent verification only if a high-risk item applies, RDD per commit, stacked PR.

Assumptions (marked, not user-stated): key bindings use Ctrl on Linux in place of the macOS `⌘` shortcuts from the philosophy doc; sidebar (`⌘B`), command palette (`⌘K` search) and live RPC tabs are out of scope for this phase.

## Tasks

| ID | Task | Specs | Route | Status | Commit |
| --- | --- | --- | --- | --- | --- |
| T0 | Spike: `gpui 0.2.2` compiles and opens a window on this machine | S1 | inline | in_progress | - |
| T1 | Pure view model + pane rectangles | S2 | inline | pending | - |
| T2 | Pure commands incl. dwindle split from real rectangle | S3 | inline | pending | - |
| T3 | GPUI window, key bindings, `--window` flag, help, feature wiring | S1, S4 | inline | pending | - |
| T4 | Checks, visual check, RDD, PR | S5 | inline | pending | - |

Route evidence: one behavior chain (T2 depends on T1, T3 on both) → no parallel units, no writer trigger; inline.

## Log

- **L1 — User request (verbatim), 2026-11-27:** "yes continue" — answering: "¿Arranco la fase de la ventana GPUI nativa real (sumar la dependencia GPUI y montar ventana, workspaces y paneles), o preferís cerrar primero los PRs #394–#398?"
