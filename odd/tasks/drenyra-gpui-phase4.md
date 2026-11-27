# Drenyra GPUI — Phase 4: Bulletproof RPC Session Lifecycle & Unified Host Controller

Feature: `drenyra-gpui-phase4` | Repo: `arkelythex/drenyra-gpui` | Stacked on: `feat/drenyra-gpui-phase3` (PR #396) | Status: checking

## Specs

- **S1 — Bulletproof `RpcSession` Lifecycle (`crates/drenyra-rpc`, closes Phase 3 advisories `R3-001`..`R3-003`, `R4-001`..`R4-002`).**
  - Add `RpcError::SessionPoisoned`. Mark `RpcSession` poisoned on `RpcError::Timeout` or `RpcError::ResponseTooLarge` so any subsequent `request()` immediately returns `Err(RpcError::SessionPoisoned)` instead of reading stale delayed replies.
  - Bound `Drop` teardown so it never blocks indefinitely if a grandchild process inherits `stdout`/`stderr`: close `stdin`, poll `try_wait()` up to `RpcTransportConfig::shutdown_grace` (default 250ms, see L4; originally a fixed 50ms), `kill()` + `wait()` the direct child, and only join reader threads if they finish within a bounded grace window (otherwise detach cleanly).
  - Fix `wait_stderr_briefly` to use an `Instant` deadline loop (`while !*done`) so spurious `Condvar` wakeups cannot drop trailing stderr diagnostics.
  - Add real subprocess tests proving: (a) reusing a timed-out `RpcSession` returns `Err(RpcError::SessionPoisoned)` even if the child later prints a delayed reply, and (b) `Drop` returns within `< 500ms` even when the child spawned a detached background grandchild holding stdout/stderr open.
- **S2 — Unified `DrenyraHostController` & `--demo-session` (`crates/drenyra-app`).**
  - Add `drenyra-ui = { path = "../drenyra-ui" }` to `drenyra-gpui/crates/drenyra-app/Cargo.toml`.
  - Create `drenyra-gpui/crates/drenyra-app/src/controller.rs` implementing `DrenyraHostController` uniting `WorkspaceManager`, `EntryDiffReview`, `ReconciliationTable`, and `JsonRpcRequest` emission:
    - Manages multi-entity workspaces, dwindle pane splits/closes/undo, staging a `JournalEntryProposal` into an `EntryDiffReview`, accepting/rejecting lines, recording human approval, and building a validated `"tools/call"` `JsonRpcRequest` for `"ledger_post"` ONLY when `review.can_post()` is true (returning an error otherwise).
  - Add `--demo-session` CLI flag to `crates/drenyra-app/src/main.rs` that runs `DrenyraHostController` end-to-end (entity workspace, dwindle split, R2 `EntryDiffReview` approval gate, balanced `MoneyCents` posting frame) and prints a deterministic JSON summary with exit code `0`.
  - Update `--help` and add unit tests covering controller gating (blocked before approval, succeeds after approval) and `--demo-session` CLI output.
  - Tenant scoping (added 2026-11-27, see L3): staged reviews are keyed per workspace. `stage_proposal` returns `ControllerError::ProposalScopeMismatch` when the proposal RUC or fiscal period differs from the active workspace and `ControllerError::ReviewAlreadyStaged` when the active workspace already holds an unposted review. A review staged in workspace A is invisible and unpostable from workspace B; posting resets only the owning workspace.
- **S3 — Verification, Native Review & Stacked PR.**
  - `cargo test --workspace`, `cargo clippy --workspace --all-targets -- -D warnings`, `--features gpui` checks, independent `gentle-ai-verify`, native review (`gentle_review_run`), and stacked PR against `feat/drenyra-gpui-phase3`.

## Tasks

| ID | Task | Specs | Route | Status | Commit |
| --- | --- | --- | --- | --- | --- |
| T1 | Harden `crates/drenyra-rpc` (poison on timeout, bounded Drop, Condvar deadline loop) | S1 | worker | done | `04d56c04` |
| T2 | Build `DrenyraHostController` + `--demo-session` in `crates/drenyra-app` | S2 | worker | done | `b927cda2` |
| T3 | Run full workspace checks, rustfmt, tenant-scope fix (RED→GREEN), commit slices | S1, S2 | inline | done | `11789ba9`, `6de1f2c9` |
| T3b | Verifier correction batch: B1 post-time scope re-check; configurable Drop shutdown grace (flake fix) | S1, S2 | inline | done | `1a008378`, `284008c8` |
| T4 | Independent verification, native review, and stacked PR | S3 | verify | in_progress | - |

## Log

- **L1 — User authorization (verbatim):**
  "avanza"

- **L2 — 2026-11-27 resume; user authorization (verbatim):**
  "yes te autorizo todo and continue debe estar todo 10/10 al 100% funcional siguiendo las buenas practicas de ingeneria de softwart + filosofia de gentlman + los mejores del mundo"
- **L3 — 2026-11-27 T3 evidence.** `cargo fmt --check` failed on 3 crates → applied rustfmt (`11789ba9`, style only). Inline review found a tenant-isolation gap in `DrenyraHostController`: reviews were controller-global, so a foreign-RUC proposal could be staged, a post after a workspace switch reset the wrong guardrails, and staging discarded an unposted review. RED: 3 new tests failed to compile (7 errors) against the old API. GREEN: per-workspace `BTreeMap<usize, EntryDiffReview>` + `WorkspaceManager::active_index()` (`6de1f2c9`). Risk: item 2 (tenant guard on `ledger_post`).
  - `cargo test --workspace --all-features`: 51 passed, 0 failed. `cargo clippy --workspace --all-targets [--all-features] -- -D warnings`: clean. `cargo fmt --all -- --check`: clean. `cargo build -p drenyra-app --features gpui`: ok.
  - `drenyra-gpui --demo-session`: exit 0, valid JSON, `blocked_before_approval:true`, `posted_frame_valid:true`, `unposted_entries:0`. Extra arg: exit 1. 15 repeated `drenyra-rpc` test runs: 0 failures.
  - RDD assess (base `feat/drenyra-gpui-phase3`, committed-only, escalated item 2): `risk: high`, `review_due: true`.
- **L4 — 2026-11-27 independent verifier.** S1 PASS; S2 PARTIAL with blocker B1: `build_post_request` did not re-check RUC/period, so a period change after staging still posted `2026-10` from a `2026-11` workspace. S3 PARTIAL (review + PR pending).
  - B1 fix (`1a008378`): RED with the verifier's probe (`Ok(...fiscal_period 2026-10...)` vs `Err(ProposalScopeMismatch)`), GREEN with a shared `ensure_proposal_matches_active_workspace` used by staging and posting.
  - Advisory 2 (pre-existing flake, `drop_allows_cooperative_child_to_exit_cleanly_without_sigkill`, 6/48 parallel failures): root cause was the fixed 50ms Drop grace racing `sh` startup; a fixed 50ms also SIGKILLs real servers flushing on EOF. Fix (`284008c8`): `RpcTransportConfig::shutdown_grace` (default 250ms) + handshake in the test. RED: missing field (E0609); GREEN; 48 parallel runs, 0 failures. S1 text updated accordingly.
  - Checks after correction: 55 tests pass; clippy `-D warnings` clean (default and all features); fmt clean; `--features gpui` builds; `--demo-session` exit 0.
  - Follow-ups (not in scope, pre-existing): RpcSession does not match response `id` to request `id` (unsolicited line is returned to the next request); `ledger_post` frame carries ids only, no `MoneyCents` lines (balance enforced via `can_post`); spurious-wakeup test may not catch a regression.
- **L5 — 2026-11-27 verifier recheck (B1 only + S1 regression).** B1 PASS (period and RUC change both rejected, no side effects, request id not consumed). S1 still PASS (unresponsive child Drop ≈303ms, child reaped; 16 parallel runs all green). Advisory applied: worst-case Drop latency documented on `shutdown_grace`. Scratch removed.
