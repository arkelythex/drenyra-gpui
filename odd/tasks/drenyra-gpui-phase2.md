# Drenyra GPUI — Phase 2: Protocol Hardening, Live RPC Supervisor & Host Diagnostics

Feature: `drenyra-gpui-phase2` | Repo: `arkelythex/drenyra-gpui` | Stacked on: `feat/drenyra-gpui-pivot` (PR #394) | Status: done

## Specs

- **S1 — Protocol & Workspace Hardening (Phase 1 advisories + multi-entity manager).**
  - `crates/drenyra-protocol`: Full RFC 8259 control-character escaping (`\u00XX` for `< 0x20`) in `escape_json_str`; `JsonRpcRequest::to_ndjson() -> Result<String, ProtocolError>` validates non-empty single-line JSON object/array `params_json` (rejects empty strings, embedded `\n`/`\r`, and non-object/array frames); add `SignedReceiptSummary` contract and full boundary tests (`MoneyCents::checked_add` overflow, single-line / zero-line / both-sides-nonzero / sum-overflow `JournalEntryProposal`, control-char escaping, malformed `params_json`).
  - `crates/drenyra-workspace`: Rename `viewport_width`/`viewport_height` → `pane_width`/`pane_height`; encapsulate `layout` and `focused_pane_id` behind read-only accessors (`layout()`, `focused_pane_id()`, `focus_pane(id) -> bool`) so focus can never silently corrupt in release builds; add `WorkspaceManager` for multi-entity workspace switching (by RUC/period), background activity aggregation (`Reconciling` / `AwaitingApproval`), and command palette filtering.
  - `scripts/sync-drenyra-standalone.sh`: Remove dead `DRENYRA_ROOT` assignment on line 10.
- **S2 — Live NDJSON Stdio RPC Subprocess Supervisor (`crates/drenyra-rpc`).**
  - Add `crates/drenyra-rpc` to the root Cargo workspace.
  - Provide `RpcTransportConfig` and `RpcSession` that spawns a child process (`drenyra-shell --mode rpc`, `pi --mode rpc`, or `drenyra-ai mcp serve`) with piped `stdin`/`stdout`/`stderr`, sends validated NDJSON `JsonRpcRequest` frames, reads newline-delimited responses, and guarantees clean child shutdown/reap on `Drop` (zero leaked zombies).
  - Prove bidirectional request/response, EOF error handling, and clean process teardown via real subprocess integration tests.
- **S3 — `drenyra-app` Diagnostics Hardening, Live `--probe-rpc`, & GPUI Host Module.**
  - Replace dummy `FiscalWorkspace` in `--check-env` with `SessionGuardrails::default().rda_enabled`; extract pure `format_check_env_json(...)` and assert exact JSON stdout payload in unit tests.
  - Tighten `find_in_path` to verify executable permission on Unix (`mode & 0o111 != 0`).
  - Add `--probe-rpc <command> [args...]` CLI subcommand that boots `RpcSession`, sends a JSON-RPC `initialize` request, and prints the raw NDJSON response (or exits non-zero with clear stderr on failure).
  - Add `src/ui.rs` native GPUI shell host module (cleanly decoupled behind the optional `gpui` Cargo feature so headless CI builds stay zero-dependency and fast while desktop builds mount the native GPUI window).
- **S4 — Verification & Review.**
  - `cargo test --workspace`, `cargo clippy --workspace --all-targets -- -D warnings`, independent `gentle-ai-verify`, and native review assessment before pushing `feat/drenyra-gpui-phase2` and opening the stacked PR.

## Tasks

| ID | Task | Specs | Route | Status | Commit |
| --- | --- | --- | --- | --- | --- |
| T1 | Deliver Phase 1 (`Arkelythex` PR #404 + `drenyra-gpui` PR #394) | - | inline | done | `b978058a` |
| T2 | Harden `drenyra-protocol`, `drenyra-workspace`, and `sync-drenyra-standalone.sh` | S1 | inline | done | `cceb9853` |
| T3 | Build `crates/drenyra-rpc` live NDJSON stdio subprocess supervisor + tests | S2 | worker | done | `f8f4ce1f` |
| T4 | Upgrade `drenyra-app` (`--check-env`, `--probe-rpc`, `src/ui.rs` GPUI feature module) | S3 | worker | done | `f8f4ce1f` |
| T5 | Independent verification (`gentle-ai-verify`), native review, and stacked PR | S4 | verify | done | `review-fa59f2da0a53f77b` / PR #395 |

## Log

- **L1 — User authorization (verbatim):**
  "te autorizo todo y avanza de forma autonoma + filosofia de gentleman"
- **L2 — Phase 1 PRs opened:**
  - `https://github.com/arkelythex/drenyra-gpui/pull/394` (`feat/drenyra-gpui-pivot`)
  - `https://github.com/arkelythex/Arkelythex/pull/404` (`docs/rename-drenyra-gpui`)
- **L3 — Phase 2 verified, reviewed, and delivered:**
  - Commits `cceb9853` (T2) and `f8f4ce1f` (T3+T4) on `feat/drenyra-gpui-phase2`.
  - Independent `gentle-ai-verify` (`mwsqv1bz-3-kf9x`): S1–S3 MET (`cargo test --workspace` 18/18, `cargo clippy -- -D warnings` 0 warnings, `--features gpui` 8/8).
  - Native review `review-fa59f2da0a53f77b` (`review-risk`, `review-resilience`, `review-reliability`): `acknowledge-approved` (`authority: burned`, `burn_evidence: gentle-ai.review-acknowledged/v1`), 9 non-blocking advisories logged for Phase 3 hardening.
  - Stacked PR opened: `https://github.com/arkelythex/drenyra-gpui/pull/395`.
