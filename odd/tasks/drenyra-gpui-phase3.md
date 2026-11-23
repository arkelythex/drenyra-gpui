# Drenyra GPUI — Phase 3: Resilience Hardening & Accounting Generative UI (`drenyra-ui`)

Feature: `drenyra-gpui-phase3` | Repo: `arkelythex/drenyra-gpui` | Stacked on: `feat/drenyra-gpui-phase2` (PR #395) | Status: done

## Specs

- **S1 — Resilience & Risk Hardening (closes Phase 2 advisories `R1-001`, `R1-002`, `R3-001`–`R3-004`, `R4-001`–`R4-003`).**
  - `crates/drenyra-protocol`: Replace shallow delimiter check in `JsonRpcRequest::to_ndjson` with a zero-dependency RFC 8259 JSON structural validator rejecting unbalanced braces/brackets, trailing commas, unclosed strings, and multiple top-level objects.
  - `crates/drenyra-workspace`: Remove `#[derive(Default)]` from `WorkspaceManager` so empty-vec panics in `active()`/`active_mut()` are impossible by construction.
  - `crates/drenyra-rpc`:
    - Drain child `stderr` in a background thread into a bounded buffer (64 KiB cap) and expose `stderr_snapshot(&self) -> String` so noisy children can never deadlock on OS pipe buffers.
    - Enforce `max_response_bytes` (default 4 MiB) returning `RpcError::ResponseTooLarge` on unbounded stdout streams.
    - Enforce `request_timeout` (default 30s, configurable on `RpcTransportConfig`) returning `RpcError::Timeout` if a child hangs without sending `\n`.
    - Wait briefly (up to 50ms polling `try_wait`) after closing `stdin` in `Drop` before invoking `child.kill()` + `child.wait()`.
  - `crates/drenyra-app`: Surface non-empty `session.stderr_snapshot()` on `--probe-rpc` failure and add `run_cli` unit tests covering `--probe-rpc` failure branches (`ExitCode::FAILURE`).
- **S2 — Accounting Generative UI Domain Surfaces (`crates/drenyra-ui`).**
  - Add pure-Rust crate `crates/drenyra-ui` (registered in root `Cargo.toml`) implementing:
    - `EntryDiffReview`: per-line decision model (`Pending`, `Accepted`, `Rejected { comment }`) over `JournalEntryProposal` with `accept_line`, `reject_line`, `accept_all`, `decided_count`, `all_decided`, `record_human_approval(approver)`, and `can_post()` (requires all lines `Accepted`, `proposal.is_balanced()`, and explicit human approval for `R2Material`/`R3Irreversible`).
    - `ReconciliationTable`: bank-vs-ledger matching rows using `MoneyCents(i128)` with status (`Matched`, `Pending`, `Discrepancy`), filtering, and `total_unmatched_delta() -> Option<MoneyCents>`.
- **S3 — Verification, Native Review & Stacked PR.**
  - `cargo test --workspace`, `cargo clippy --workspace --all-targets -- -D warnings`, `--features gpui` checks, independent `gentle-ai-verify`, native review (`gentle_review_run`), and stacked PR against `feat/drenyra-gpui-phase2`.

## Tasks

| ID | Task | Specs | Route | Status | Commit |
| --- | --- | --- | --- | --- | --- |
| T1 | Harden `drenyra-protocol`, `drenyra-workspace`, `drenyra-rpc`, and `drenyra-app` | S1 | worker | done | `869402b9` |
| T2 | Build `crates/drenyra-ui` (`EntryDiffReview` + `ReconciliationTable`) | S2 | worker | done | `4ad7ab4a` |
| T3 | Wire `crates/drenyra-ui` into root `Cargo.toml`, run full checks, and commit | S1, S2 | inline | done | `4ad7ab4a` |
| T4 | Independent verification (`gentle-ai-verify`), native review, and stacked PR | S3 | verify | done | `review-3a8f9af5b06ae5f0` / PR #396 |

## Log

- **L1 — User authorization (verbatim):**
  "avanza"
- **L2 — Phase 3 verified, reviewed, and delivered:**
  - Commits `869402b9` (S1 hardening) and `4ad7ab4a` (S2 `crates/drenyra-ui`) on `feat/drenyra-gpui-phase3`.
  - Independent `gentle-ai-verify` (`mwsr9ley-6-wk7n`): S1–S2 MET (`cargo test --workspace` 33/33, `--features gpui` 10/10, `cargo clippy -- -D warnings` 0 warnings).
  - Native review `review-3a8f9af5b06ae5f0` (`review-risk`, `review-resilience`, `review-reliability`): `acknowledge-approved` (`authority: burned`, `burn_evidence: gentle-ai.review-acknowledged/v1`); 0 risk findings, 5 non-blocking advisories logged for Phase 4.
  - Stacked PR opened: `https://github.com/arkelythex/drenyra-gpui/pull/396`.
