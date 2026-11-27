# Drenyra GPUI — Phase 6: Live Agent Pane, Settings & DreamCoder Themes, Operator Surfaces

Feature: `drenyra-gpui-phase6` | Branch: `feat/drenyra-gpui-phase6` stacked on `fix/gpui-approval-invalidated-on-edit` (PR #401) | Delivery: `ask-on-risk` (chain strategy already established: stacked PRs) | Runner: `cargo test --workspace --locked` (+ `--all-features`) | Status: in progress

## Specs

- **S1 — Live Fiscal Agent pane.** User: "yes avanza con todo", answering: "Mi recomendación es **conectar el panel del agente a `drenyra-shell` real** antes que nada [...] ¿Arranco por ahí?". The Fiscal Agent pane launches the real agent shell over stdio (`drenyra-shell`, falling back to `pi --mode rpc`, the binary `--check-env` already accepts), lets the operator type a prompt, and renders the streamed reply. When no shell binary is found or the session fails, the pane states it and the app keeps working on demo data. Protocol details: see L2 (pending map).
- **S2 — Settings section with Themes.** User (verbatim): "ademas el mejora el system desing la secciion settings con su seccion themes con theme dreamcoder light y dreamcoder dark". The app gets a Settings section containing a Themes section that offers "dreamcoder light" and "dreamcoder dark". Palettes come from the user's theme repository `/home/dreamcoder08/Documents/PROYECTOS/dreamcoder08/dreamcoder-workbench` (user-provided `pwd`), never invented.
- **S3 — Design system.** User: "mejora el system desing". All window colors come from one theme token set (no hard-coded colors in `shell::window`), so the whole window switches with the selected theme.
- **S4 — Operator surfaces ("con todo").** The items listed as missing in the previous answer, in priority order: R2/R3 approval command/dialog (uses the #401 guard), Command Palette, Sidebar (documents/ledgers, unposted entries, periods), additional pane kinds. Assumption (not user-stated): delivered after S1–S3, each as its own work unit, as far as the session allows; remaining items are recorded as pending, never claimed.
- **S5 — Verification.** Test-first for pure logic (view model, commands, theme tokens, protocol parsing); headless tests; clippy `-D warnings` with and without `gpui`; fmt; the window launched and visually checked in both themes; independent verifier for high-risk items; RDD per commit; stacked PRs.

Assumptions (marked): keys keep Ctrl on Linux; the selected theme persists across runs only if a settings file location is agreed — until then it is per session (assumption, revisit in Log).

## Tasks

| ID | Task | Specs | Route | Status | Commit |
| --- | --- | --- | --- | --- | --- |
| T0 | Map theme palettes and agent RPC protocol | S1, S2 | delegated (2 read-only explorers, mapping trigger: two unfamiliar external sources) | done | L3 |
| T1 | Theme tokens (light/dark) + window uses tokens only | S2, S3 | inline | done | see L4 |
| T2 | Settings section with Themes selector | S2 | inline | done | see L4 |
| T3 | Live agent session protocol + pane (prompt input, streamed output, failure states) | S1 | inline | pending | — |
| T4 | Approval command/dialog | S4 | inline | pending | — |
| T5 | Command palette | S4 | inline | pending | — |
| T6 | Sidebar | S4 | inline | pending | — |
| T7 | Checks, visual check both themes, verifier, RDD, PRs | S5 | verify | pending | — |

## Log

- **L1 — User request (verbatim), 2026-11-27:** "yes avanza con todo ademas el mejora el system desing la secciion settings con su seccion themes con theme dreamcoder light y dreamcoder dark , ❯ pwd
/home/dreamcoder08/Documents/PROYECTOS/dreamcoder08/dreamcoder-workbench"
- **L2 — 2026-11-27 discovery.** `drenyra-shell` is not on PATH; `--check-env` reports `shell_bin_found: true` because it falls back to `pi`. Theme sources: `dreamcoder-workbench/DreamcoderThemes/` and `themes/dreamcoder`. Two read-only explorers launched (palettes; `pi --mode rpc` protocol).
- **L3 — 2026-11-27 maps.**
  - Palette: canonical source `dreamcoder-workbench/DreamcoderThemes/dreamcoder/tokens.json` (`modes.dark` "Dreamcoder Dark" lines 8-66, `modes.light` "Dreamcoder Light" lines 91-143; contrast guardrails lines 201-218; rules in `docs/DREAMCODER_DESIGN_SYSTEM.md`). Fonts installed: JetBrainsMono / Iosevka Nerd Font.
  - Agent protocol: `pi --mode rpc` (pi 1.6.0) is **not JSON-RPC 2.0**: LF-delimited `{"type":...}` records, optional string `id` echoed in `{"type":"response",...}`, async replies, an unsolicited event stream after `prompt` until `agent_settled`, and blocking `extension_ui_request` dialogs that must be answered. Probe confirmed pi rejects a JSON-RPC frame (`Unknown command: undefined`). The current `RpcSession::request` would poison the session on the first event with an id or a parse error without id. Reusable: spawn, bounded reader threads, `read_bounded_line`, `write_frame`, `Drop`. `drenyra-shell` is a pi extension package at `arkelythex/drenyra-pi` (no `bin`, not installed into pi here); the architecture doc's "JSON-RPC 2.0" claims are wrong and must be corrected.
  - Decision: T3 adds a pi-protocol agent client next to `RpcSession` (dispatcher by `type`, string ids, event channel, `abort` while streaming) instead of bending `RpcSession`.
- **L4 — 2026-11-27 T1–T2 evidence.**
  - RED → GREEN: settings overlay (21 compile errors → 3 tests pass); theme contrast guard proven by mutation (Light `warning` set to the Dark value → "warning on chrome is 1.45:1, below 4.5:1"), then restored. The palette passes WCAG AA for every text/status role on chrome, surface, surface_raised and surface_strong, 3:1 for `border_ui`/`focus`.
  - `save_if_changed` was extracted after the live check could not drive persistence (the operator moved to another workspace; keys are only sent to the active `drenyra-gpui` window); its test covers no-op, write and failure. RED not observed for it (written with the implementation).
  - Live: Dreamcoder Dark default, Settings overlay, Dreamcoder Light loaded from the settings file (screenshots cropped to the window). Found and fixed by looking: the overlay overflowed to the right (missing `min_w_0` on content and cards) and the active card was taller (fixed footer height).
  - Not verified live: applying a theme from the overlay and its save from the window (covered by `ShellState` and `save_if_changed` tests).
  - Checks: 110 tests headless / 109 with features; clippy `-D warnings` with and without `gpui`; fmt.
  - Size: ~1.1k changed lines in one unit (tokens, settings, window restyle share `shell::window` functions; splitting would be artificial).
  - Risk: none of items 1-6 (new optional settings file, new keys; existing keys and `--demo-session` unchanged).
