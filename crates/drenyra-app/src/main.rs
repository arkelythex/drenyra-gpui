//! Drenyra GPUI — native desktop host binary entrypoint.

#[cfg_attr(not(test), allow(dead_code))]
mod ui;

use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use drenyra_protocol::{escape_json_str, JsonRpcRequest, MoneyCents};
use drenyra_rpc::{RpcError, RpcSession, RpcTransportConfig};
use drenyra_workspace::SessionGuardrails;

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

pub fn probe_rpc(command: &str, args: &[String]) -> Result<String, RpcError> {
    let config = RpcTransportConfig {
        program: command.to_string(),
        args: args.to_vec(),
    };
    let mut session = RpcSession::spawn(&config)?;
    let req = JsonRpcRequest {
        id: 1,
        method: "initialize".into(),
        params_json: r#"{"client":"drenyra-gpui"}"#.into(),
    };
    session.request(&req)
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
                "drenyra-gpui: unrecognized arguments: {} (try --help, --check-env, or --probe-rpc)",
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
}
