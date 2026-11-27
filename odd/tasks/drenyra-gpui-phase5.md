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
| T0 | Spike: `gpui 0.2.2` compiles and opens a window on this machine | S1 | inline | done | scratch spike (see L2) |
| T1 | Pure view model + pane rectangles | S2 | inline | done | `29c47f29` |
| T2 | Pure commands incl. dwindle split from real rectangle | S3 | inline | done | `29c47f29` |
| T3 | GPUI window, key bindings, `--window` flag, help, feature wiring | S1, S4 | inline | done | `ba272154`, `d9cbe554`, `76223609` |
| T4 | Checks, visual check, verifier (Risk item 5), RDD, PR | S5 | verify | in_progress | - |

Route evidence: one behavior chain (T2 depends on T1, T3 on both) → no parallel units, no writer trigger; inline.

## Log

- **L1 — User request (verbatim), 2026-11-27:** "yes continue" — answering: "¿Arranco la fase de la ventana GPUI nativa real (sumar la dependencia GPUI y montar ventana, workspaces y paneles), o preferís cerrar primero los PRs #394–#398?"
- **L2 — 2026-11-27 T0 spike.** `gpui 0.2.2` failed to build: `xattr 0.2.3` (via `gpui_http_client` → `zed-async-tar 0.5.0-zed`) uses `libc::ENOATTR`, removed on Linux in `libc 0.2.191`. Pinning `libc` is impossible (`nix 0.31.4` needs ≥0.2.186, `mio 1.2.5` needs ≥0.2.190, and 0.2.190 still fails). `zed-async-tar`'s `xattr` default feature cannot be disabled transitively. Decision: vendor `xattr 0.2.3` with a one-constant patch (Linux/Android `ENOATTR = ENODATA`) via `[patch.crates-io]`; rejected alternative: GPUI from Zed git `main` (unstable API, huge checkout) for one constant. Spike then built and opened a Wayland window (Hyprland).
- **L3 — 2026-11-27 T1–T3 evidence.**
  - RED → GREEN: view model (22 compile errors → 7 tests), commands (33 → 8), bindings/pane area (11 → 2), `--window` (1 → 2), risk tier (assert `R0Read` vs `R2Material` → pass).
  - Defects found by looking at the real window and fixed: guardrail chips clipped on a 936px tiled window (overflow and shrink rules); session risk chip showed `R0 Read` next to a staged `R2 Material` proposal (`d9cbe554`, an existing view-model test had pinned the defect and was corrected); pending marker barely visible.
  - Live checks on Hyprland/Wayland with guarded `ydotool` keys (only sent while `drenyra-gpui` was the active window): A accept → ✓, J cursor, R reject → ✕, Ctrl+D split side by side on a wide pane, Ctrl+D vertical split on a tall pane (real dwindle), Ctrl+K → Sierra SRL with review hidden, `R0 Read`, `Unposted 0`, amber background alert. Screenshots were cropped to the app window only.
  - Not verified visually: the narrow-window layout after the overflow fix (Hyprland Lua dispatch syntax blocked resizing; a second instance stacked instead of tiling). Stopped rather than keep fighting the compositor.
  - Checks: 81 tests headless / 80 with `--all-features`; clippy `-D warnings` with and without features; fmt; `cargo doc`; `--demo-session` byte-identical after the fixture refactor; each new commit builds and passes in isolation. `docs:check-freshness` 4 → 3 pre-existing failures (dated the philosophy doc); `docs:check-links` pass.
  - Risk: item 5 (new dependency + `[patch.crates-io]` vendored code) → independent verifier.
  - `Cargo.lock` is gitignored, so the resolved GPUI dependency tree is not pinned in the repo (pending user decision, see PR #398 notes).
