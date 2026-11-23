//! Drenyra GPUI — native desktop host binary entrypoint.

#[cfg_attr(not(test), allow(dead_code))]
pub mod controller;
#[cfg_attr(not(test), allow(dead_code))]
mod ui;

use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use controller::{ControllerError, DrenyraHostController};
use drenyra_protocol::{
    escape_json_str, validate_json_container, JournalEntryProposal, JournalLine, JsonRpcRequest,
    MoneyCents, RiskTier,
};
use drenyra_rpc::{RpcSession, RpcTransportConfig};
use drenyra_workspace::{FiscalWorkspace, PaneKind, SessionGuardrails};

fn main() -> ExitCode {
    let args: Vec<String> = env::args().skip(1).collect();
    run_cli(&args)
}

pub fn format_check_env_json(
    version: &str,
    rda_default: bool,
    shell_found: bool,
    kernel_found: bool,
) -> String {
    format!(
        r#"{{"app":"drenyra-gpui","version":"{}","zero_float_money":"{}","rda_default":{rda_default},"shell_bin_found":{shell_found},"kernel_bin_found":{kernel_found}}}"#,
        escape_json_str(version),
        MoneyCents::ZERO,
    )
}

pub fn probe_rpc(command: &str, args: &[String]) -> Result<String, String> {
    let config = RpcTransportConfig::new(command, args.to_vec());
    let mut session = RpcSession::spawn(&config).map_err(|err| err.to_string())?;
    let req = JsonRpcRequest {
        id: 1,
        method: "initialize".into(),
        params_json: r#"{"client":"drenyra-gpui"}"#.into(),
    };
    session.request(&req).map_err(|err| {
        let stderr = session.stderr_snapshot();
        let stderr = stderr.trim();
        if stderr.is_empty() {
            err.to_string()
        } else {
            format!("{err} (stderr: {stderr})")
        }
    })
}

pub fn run_demo_session_json() -> Result<String, ControllerError> {
    let workspace = FiscalWorkspace::new("20123456789", "Andino SAC", "2026-10");
    let mut controller = DrenyraHostController::new(workspace);
    controller
        .workspaces_mut()
        .active_mut()
        .split_focused(PaneKind::LedgerViewer, 1440, 900);
    let pane_count = controller.workspaces().active().pane_count();

    let proposal = JournalEntryProposal {
        id: "je-demo-2026-10-001".into(),
        entity_ruc: "20123456789".into(),
        fiscal_period: "2026-10".into(),
        risk_tier: RiskTier::R2Material,
        lines: vec![
            JournalLine {
                account_code: "1212".into(),
                debit: MoneyCents::from_cents(11_800),
                credit: MoneyCents::ZERO,
                description: "Accounts receivable".into(),
            },
            JournalLine {
                account_code: "7011".into(),
                debit: MoneyCents::ZERO,
                credit: MoneyCents::from_cents(10_000),
                description: "Sales revenue".into(),
            },
            JournalLine {
                account_code: "40111".into(),
                debit: MoneyCents::ZERO,
                credit: MoneyCents::from_cents(1_800),
                description: "IGV 18%".into(),
            },
        ],
    };
    controller.stage_proposal(proposal);

    let blocked_before_approval = matches!(
        controller.build_post_request(),
        Err(ControllerError::ReviewNotReadyToPost)
    );

    if let Some(review) = controller.active_review_mut() {
        review.accept_all();
        review.record_human_approval("CPA-PERU-001");
    }

    let frame = controller.build_post_request()?;
    let posted_frame_valid = !frame.contains('\n')
        && !frame.contains('\r')
        && validate_json_container(&frame).is_ok()
        && frame.contains(r#""method":"tools/call""#)
        && frame.contains(r#""name":"ledger_post""#);

    let active = controller.workspaces().active();
    Ok(format!(
        r#"{{"entity_ruc":"{}","fiscal_period":"{}","pane_count":{},"blocked_before_approval":{},"posted_frame_valid":{},"unposted_entries":{},"frame":{}}}"#,
        escape_json_str(&active.entity_ruc),
        escape_json_str(&active.fiscal_period),
        pane_count,
        blocked_before_approval,
        posted_frame_valid,
        active.guardrails.unposted_entries,
        frame,
    ))
}

pub fn run_cli(args: &[String]) -> ExitCode {
    match args {
        [] | [_] if matches!(args.first().map(String::as_str), None | Some("--help" | "-h")) => {
            println!("drenyra-gpui {}", env!("CARGO_PKG_VERSION"));
            println!("Native Rust + GPUI accounting desktop host for Drenyra Shell & Drenyra AI.");
            println!();
            println!("Usage:");
            println!("  drenyra-gpui --help                       Show this help message");
            println!("  drenyra-gpui --check-env                  Print headless RPC host environment diagnostics (JSON)");
            println!("  drenyra-gpui --demo-session               Run deterministic end-to-end controller & approval gate demo (JSON)");
            println!("  drenyra-gpui --probe-rpc <cmd> [args...]  Probe an NDJSON stdio JSON-RPC 2.0 server");
            ExitCode::SUCCESS
        }
        [flag] if flag == "--check-env" => {
            let shell_found = find_in_path("drenyra-shell").or_else(|| find_in_path("pi"));
            let kernel_found = find_in_path("drenyra-ai");
            let rda_default = SessionGuardrails::default().rda_enabled;
            println!(
                "{}",
                format_check_env_json(
                    env!("CARGO_PKG_VERSION"),
                    rda_default,
                    shell_found.is_some(),
                    kernel_found.is_some(),
                )
            );
            ExitCode::SUCCESS
        }
        [flag] if flag == "--demo-session" => match run_demo_session_json() {
            Ok(json) => {
                println!("{json}");
                ExitCode::SUCCESS
            }
            Err(err) => {
                eprintln!("drenyra-gpui: --demo-session failed: {err}");
                ExitCode::FAILURE
            }
        },
        [flag] if flag == "--probe-rpc" => {
            eprintln!("drenyra-gpui: --probe-rpc requires a <command> argument");
            ExitCode::FAILURE
        }
        [flag, command, rest @ ..] if flag == "--probe-rpc" => match probe_rpc(command, rest) {
            Ok(response) => {
                println!("{response}");
                ExitCode::SUCCESS
            }
            Err(err) => {
                eprintln!("drenyra-gpui: --probe-rpc failed: {err}");
                ExitCode::FAILURE
            }
        },
        _ => {
            eprintln!(
                "drenyra-gpui: unrecognized arguments: {} (try --help, --check-env, --demo-session, or --probe-rpc)",
                args.join(" ")
            );
            ExitCode::FAILURE
        }
    }
}

fn is_executable_file(path: &Path) -> bool {
    let Ok(metadata) = fs::metadata(path) else {
        return false;
    };
    if !metadata.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        metadata.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    {
        true
    }
}

fn find_in_path(binary: &str) -> Option<PathBuf> {
    let path_var = env::var_os("PATH")?;
    for dir in env::split_paths(&path_var) {
        let candidate = dir.join(binary);
        if is_executable_file(&candidate) {
            return Some(candidate);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_args_and_help_flag_exit_successfully() {
        assert_eq!(run_cli(&[]), ExitCode::SUCCESS);
        assert_eq!(run_cli(&["--help".into()]), ExitCode::SUCCESS);
    }

    #[test]
    fn check_env_flag_and_json_format_are_exact() {
        assert_eq!(run_cli(&["--check-env".into()]), ExitCode::SUCCESS);
        let json = format_check_env_json("0.1.0", true, true, false);
        assert_eq!(
            json,
            r#"{"app":"drenyra-gpui","version":"0.1.0","zero_float_money":"0.00","rda_default":true,"shell_bin_found":true,"kernel_bin_found":false}"#
        );
    }

    #[test]
    fn check_env_with_extra_args_and_unknown_flags_fail() {
        assert_eq!(
            run_cli(&["--check-env".into(), "--extra".into()]),
            ExitCode::FAILURE
        );
        assert_eq!(run_cli(&["--unknown-flag".into()]), ExitCode::FAILURE);
    }

    #[test]
    fn probe_rpc_without_command_fails() {
        assert_eq!(run_cli(&["--probe-rpc".into()]), ExitCode::FAILURE);
    }

    #[test]
    fn probe_rpc_nonexistent_binary_and_eof_fail() {
        assert_eq!(
            run_cli(&[
                "--probe-rpc".into(),
                "definitely-not-a-real-drenyra-binary-12345".into(),
            ]),
            ExitCode::FAILURE
        );
        #[cfg(unix)]
        assert_eq!(
            run_cli(&["--probe-rpc".into(), "sh".into(), "-c".into(), "exit 0".into()]),
            ExitCode::FAILURE
        );
    }

    #[cfg(unix)]
    #[test]
    fn probe_rpc_failure_includes_non_empty_stderr_snapshot() {
        let err = probe_rpc("sh", &["-c".into(), "echo 'boom on stderr' >&2; exit 1".into()])
            .expect_err("expected probe_rpc failure");
        let msg = err.to_string();
        assert!(
            msg.contains("boom on stderr"),
            "expected stderr in error message, got: {msg}"
        );
    }

    #[cfg(unix)]
    #[test]
    fn probe_rpc_live_handshake_succeeds_against_mock_responder() {
        let script = r#"IFS= read -r req && printf '{"jsonrpc":"2.0","id":1,"result":{"seen":%s}}\n' "$req""#;
        let args = vec![
            "--probe-rpc".into(),
            "sh".into(),
            "-c".into(),
            script.into(),
        ];
        assert_eq!(run_cli(&args), ExitCode::SUCCESS);

        let reply = probe_rpc("sh", &["-c".into(), script.into()])
            .expect("probe_rpc returns response frame");
        assert!(reply.contains(r#""method":"initialize""#));
        assert!(reply.contains(r#""client":"drenyra-gpui""#));
    }

    #[cfg(unix)]
    #[test]
    fn is_executable_file_requires_unix_exec_bit() {
        use std::os::unix::fs::PermissionsExt;

        let dir = env::temp_dir().join(format!("drenyra-gpui-test-{}", std::process::id()));
        fs::create_dir_all(&dir).expect("create temp test dir");
        let non_exec = dir.join("not-exec");
        let exec = dir.join("is-exec");
        fs::write(&non_exec, b"#!/bin/sh\n").expect("write non-exec");
        fs::write(&exec, b"#!/bin/sh\n").expect("write exec");

        fs::set_permissions(&non_exec, fs::Permissions::from_mode(0o644)).expect("chmod 644");
        fs::set_permissions(&exec, fs::Permissions::from_mode(0o755)).expect("chmod 755");

        let non_exec_ok = is_executable_file(&non_exec);
        let exec_ok = is_executable_file(&exec);

        let _ = fs::remove_dir_all(&dir);

        assert!(!non_exec_ok);
        assert!(exec_ok);
    }

    #[test]
    fn demo_session_json_and_cli_succeed_with_expected_fields() {
        let json = run_demo_session_json().expect("demo session should succeed");
        assert_eq!(drenyra_protocol::validate_json_container(&json), Ok(()));
        assert!(json.contains(r#""entity_ruc":"20123456789""#));
        assert!(json.contains(r#""pane_count":2"#));
        assert!(json.contains(r#""blocked_before_approval":true"#));
        assert!(json.contains(r#""posted_frame_valid":true"#));
        assert!(json.contains(r#""name":"ledger_post""#));
        assert!(json.contains(r#""approved_by":"CPA-PERU-001""#));

        assert_eq!(run_cli(&["--demo-session".into()]), ExitCode::SUCCESS);
        assert_eq!(
            run_cli(&["--demo-session".into(), "--extra".into()]),
            ExitCode::FAILURE
        );
    }
}
