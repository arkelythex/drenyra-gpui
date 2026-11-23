# Drenyra GPUI — Phase 4: Bulletproof RPC Session Lifecycle & Unified Host Controller

Feature: `drenyra-gpui-phase4` | Repo: `arkelythex/drenyra-gpui` | Stacked on: `feat/drenyra-gpui-phase3` (PR #396) | Status: in_progress

## Specs

- **S1 — Bulletproof `RpcSession` Lifecycle (`crates/drenyra-rpc`, closes Phase 3 advisories `R3-001`..`R3-003`, `R4-001`..`R4-002`).**
  - Add `RpcError::SessionPoisoned`. Mark `RpcSession` poisoned on `RpcError::Timeout` or `RpcError::ResponseTooLarge` so any subsequent `request()` immediately returns `Err(RpcError::SessionPoisoned)` instead of reading stale delayed replies.
  - Bound `Drop` teardown so it never blocks indefinitely if a grandchild process inherits `stdout`/`stderr`: close `stdin`, poll `try_wait()` up to 50ms, `kill()` + `wait()` the direct child, and only join reader threads if they finish within a bounded grace window (otherwise detach cleanly).
  - Fix `wait_stderr_briefly` to use an `Instant` deadline loop (`while !*done`) so spurious `Condvar` wakeups cannot drop trailing stderr diagnostics.
  - Add real subprocess tests proving: (a) reusing a timed-out `RpcSession` returns `Err(RpcError::SessionPoisoned)` even if the child later prints a delayed reply, and (b) `Drop` returns within `< 500ms` even when the child spawned a detached background grandchild holding stdout/stderr open.
- **S2 — Unified `DrenyraHostController` & `--demo-session` (`crates/drenyra-app`).**
  - Add `drenyra-ui = { path = "../drenyra-ui" }` to `drenyra-gpui/crates/drenyra-app/Cargo.toml`.
  - Create `drenyra-gpui/crates/drenyra-app/src/controller.rs` implementing `DrenyraHostController` uniting `WorkspaceManager`, `EntryDiffReview`, `ReconciliationTable`, and `JsonRpcRequest` emission:
    - Manages multi-entity workspaces, dwindle pane splits/closes/undo, staging a `JournalEntryProposal` into an `EntryDiffReview`, accepting/rejecting lines, recording human approval, and building a validated `"tools/call"` `JsonRpcRequest` for `"ledger_post"` ONLY when `review.can_post()` is true (returning an error otherwise).
  - Add `--demo-session` CLI flag to `crates/drenyra-app/src/main.rs` that runs `DrenyraHostController` end-to-end (entity workspace, dwindle split, R2 `EntryDiffReview` approval gate, balanced `MoneyCents` posting frame) and prints a deterministic JSON summary with exit code `0`.
  - Update `--help` and add unit tests covering controller gating (blocked before approval, succeeds after approval) and `--demo-session` CLI output.
- **S3 — Verification, Native Review & Stacked PR.**
  - `cargo test --workspace`, `cargo clippy --workspace --all-targets -- -D warnings`, `--features gpui` checks, independent `gentle-ai-verify`, native review (`gentle_review_run`), and stacked PR against `feat/drenyra-gpui-phase3`.

## Tasks

| ID | Task | Specs | Route | Status | Commit |
| --- | --- | --- | --- | --- | --- |
| T1 | Harden `crates/drenyra-rpc` (poison on timeout, bounded Drop, Condvar deadline loop) | S1 | worker | in_progress | - |
| T2 | Build `DrenyraHostController` + `--demo-session` in `crates/drenyra-app` | S2 | worker | in_progress | - |
| T3 | Run full workspace checks and commit Phase 4 slices | S1, S2 | inline | pending | - |
| T4 | Independent verification (`gentle-ai-verify`), native review, and stacked PR | S3 | verify | pending | - |

## Log

- **L1 — User authorization (verbatim):**
  "avanza"
