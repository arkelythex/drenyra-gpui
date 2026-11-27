//! Drenyra GPUI — native desktop host binary entrypoint.

#[cfg_attr(not(test), allow(dead_code))]
pub mod controller;
#[cfg_attr(not(test), allow(dead_code))]
mod shell;
#[cfg_attr(not(test), allow(dead_code))]
mod ui;

use std::env;
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use controller::{ControllerError, DrenyraHostController};
use drenyra_protocol::{escape_json_str, validate_json_container, JsonRpcRequest, MoneyCents};
use drenyra_rpc::{RpcSession, RpcTransportConfig};
use drenyra_workspace::{PaneKind, SessionGuardrails};

fn main() -> ExitCode {
    let args: Vec<String> = env::args().skip(1).collect();
    run_cli(&args)
}

/// Flags other than `--help`, with the one-line description `--help` prints for each. The
/// window's Fiscal CLI pane lists the same table, so the two never drift apart.
pub const CLI_FLAGS: &[(&str, &str)] = &[
    (
        "--check-env",
        "Print headless RPC host environment diagnostics (JSON)",
    ),
    (
        "--demo-session",
        "Run deterministic end-to-end controller & approval gate demo (JSON)",
    ),
    (
        "--probe-rpc <cmd> [args...]",
        "Probe an NDJSON stdio JSON-RPC 2.0 server",
    ),
    (
        "--window",
        "Open the native GPUI window on demo data (requires --features gpui)",
    ),
];

/// A value of the environment check: JSON text or a JSON boolean.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CheckValue {
    Text(String),
    Flag(bool),
}

impl fmt::Display for CheckValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Text(text) => f.write_str(text),
            Self::Flag(flag) => write!(f, "{flag}"),
        }
    }
}

/// What `--check-env` reports. The window captures it once at start-up for its Fiscal CLI pane.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EnvironmentCheck {
    pub version: String,
    pub rda_default: bool,
    pub shell_bin_found: bool,
    pub kernel_bin_found: bool,
}

impl EnvironmentCheck {
    /// Looks up the agent shell (`drenyra-shell`, else `pi`) and kernel binaries on `PATH`.
    pub fn detect() -> Self {
        Self {
            version: env!("CARGO_PKG_VERSION").to_owned(),
            rda_default: SessionGuardrails::default().rda_enabled,
            shell_bin_found: find_in_path("drenyra-shell")
                .or_else(|| find_in_path("pi"))
                .is_some(),
            kernel_bin_found: find_in_path("drenyra-ai").is_some(),
        }
    }

    /// Every field in the order `--check-env` prints it: the single source of the field list.
    pub fn fields(&self) -> [(&'static str, CheckValue); 6] {
        [
            ("app", CheckValue::Text("drenyra-gpui".to_owned())),
            ("version", CheckValue::Text(self.version.clone())),
            (
                "zero_float_money",
                CheckValue::Text(MoneyCents::ZERO.to_string()),
            ),
            ("rda_default", CheckValue::Flag(self.rda_default)),
            ("shell_bin_found", CheckValue::Flag(self.shell_bin_found)),
            ("kernel_bin_found", CheckValue::Flag(self.kernel_bin_found)),
        ]
    }

    /// The one-line JSON object `--check-env` prints.
    pub fn to_json(&self) -> String {
        let body: Vec<String> = self
            .fields()
            .iter()
            .map(|(name, value)| match value {
                CheckValue::Text(text) => format!(r#""{name}":"{}""#, escape_json_str(text)),
                CheckValue::Flag(flag) => format!(r#""{name}":{flag}"#),
            })
            .collect();
        format!("{{{}}}", body.join(","))
    }
}

pub fn format_check_env_json(
    version: &str,
    rda_default: bool,
    shell_found: bool,
    kernel_found: bool,
) -> String {
    EnvironmentCheck {
        version: version.to_owned(),
        rda_default,
        shell_bin_found: shell_found,
        kernel_bin_found: kernel_found,
    }
    .to_json()
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
    let mut controller = DrenyraHostController::new(shell::demo::demo_workspace());
    controller
        .workspaces_mut()
        .active_mut()
        .split_focused(PaneKind::LedgerViewer, 1440, 900);
    let pane_count = controller.workspaces().active().pane_count();

    controller.stage_proposal(shell::demo::demo_proposal())?;

    let blocked_before_approval = matches!(
        controller.build_post_request(),
        Err(ControllerError::ReviewNotReadyToPost)
    );

    let line_count = controller
        .active_review()
        .ok_or(ControllerError::NoStagedReview)?
        .proposal()
        .lines
        .len();
    // Line decisions and the approval go through the controller so they reach its evidence log.
    for line in 0..line_count {
        controller.accept_active_line(line);
    }
    if !controller.approve_active("CPA-PERU-001") {
        return Err(ControllerError::ReviewNotReadyToPost);
    }

    let frame = controller.build_post_request()?;
    let unposted_before_confirmation = controller.workspaces().active().guardrails.unposted_entries;
    // The demo has no live transport, so it confirms the in-flight post deterministically.
    let request_id = controller
        .pending_post_request()
        .ok_or(ControllerError::UnknownPostRequest)?;
    controller.confirm_posted(request_id)?;
    let posted_frame_valid = !frame.contains('\n')
        && !frame.contains('\r')
        && validate_json_container(&frame).is_ok()
        && frame.contains(r#""method":"tools/call""#)
        && frame.contains(r#""name":"ledger_post""#);

    let active = controller.workspaces().active();
    Ok(format!(
        r#"{{"entity_ruc":"{}","fiscal_period":"{}","pane_count":{},"blocked_before_approval":{},"posted_frame_valid":{},"unposted_before_confirmation":{},"unposted_entries":{},"frame":{}}}"#,
        escape_json_str(&active.entity_ruc),
        escape_json_str(&active.fiscal_period),
        pane_count,
        blocked_before_approval,
        posted_frame_valid,
        unposted_before_confirmation,
        active.guardrails.unposted_entries,
        frame,
    ))
}

/// Error printed for arguments no flag accepts (text kept stable for existing scripts).
pub fn unrecognized_arguments_message(args: &[String]) -> String {
    format!(
        "drenyra-gpui: unrecognized arguments: {} (try --help, --check-env, --demo-session, or --probe-rpc)",
        args.join(" ")
    )
}

/// Message printed by `--window` in a headless build.
pub const fn window_unavailable_message() -> &'static str {
    "drenyra-gpui: --window requires a native build; rebuild with `cargo run -p drenyra-app --features gpui -- --window`"
}

#[cfg(feature = "gpui")]
fn run_window() -> ExitCode {
    let settings_path = shell::settings::default_settings_path();
    let settings = settings_path
        .as_deref()
        .map(shell::settings::Settings::load)
        .unwrap_or_default();
    match ui::mount_native_window(
        ui::DesktopShellConfig::default(),
        shell::commands::ShellState::demo()
            .with_settings(settings)
            .with_environment(EnvironmentCheck::detect()),
        settings_path,
    ) {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("drenyra-gpui: --window failed: {err}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(not(feature = "gpui"))]
fn run_window() -> ExitCode {
    eprintln!("{}", window_unavailable_message());
    ExitCode::FAILURE
}

pub fn run_cli(args: &[String]) -> ExitCode {
    match args {
        [] | [_]
            if matches!(
                args.first().map(String::as_str),
                None | Some("--help" | "-h")
            ) =>
        {
            println!("drenyra-gpui {}", env!("CARGO_PKG_VERSION"));
            println!("Native Rust + GPUI accounting desktop host for Drenyra Shell & Drenyra AI.");
            println!();
            println!("Usage:");
            println!("  drenyra-gpui {:<29}Show this help message", "--help");
            for (usage, description) in CLI_FLAGS {
                println!("  drenyra-gpui {usage:<29}{description}");
            }
            ExitCode::SUCCESS
        }
        [flag] if flag == "--check-env" => {
            println!("{}", EnvironmentCheck::detect().to_json());
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
        [flag] if flag == "--window" => run_window(),
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
            eprintln!("{}", unrecognized_arguments_message(args));
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

pub(crate) fn find_in_path(binary: &str) -> Option<PathBuf> {
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
            run_cli(&[
                "--probe-rpc".into(),
                "sh".into(),
                "-c".into(),
                "exit 0".into()
            ]),
            ExitCode::FAILURE
        );
    }

    #[cfg(unix)]
    #[test]
    fn probe_rpc_failure_includes_non_empty_stderr_snapshot() {
        let err = probe_rpc(
            "sh",
            &["-c".into(), "echo 'boom on stderr' >&2; exit 1".into()],
        )
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
        assert!(json.contains(r#""unposted_before_confirmation":1"#));
        assert!(json.contains(r#""unposted_entries":0"#));
        assert!(json.contains(r#""name":"ledger_post""#));
        assert!(json.contains(r#""approved_by":"CPA-PERU-001""#));

        assert_eq!(run_cli(&["--demo-session".into()]), ExitCode::SUCCESS);
        assert_eq!(
            run_cli(&["--demo-session".into(), "--extra".into()]),
            ExitCode::FAILURE
        );
    }

    #[cfg(not(feature = "gpui"))]
    #[test]
    fn window_flag_fails_with_rebuild_hint_without_gpui_feature() {
        assert_eq!(run_cli(&["--window".into()]), ExitCode::FAILURE);
        assert_eq!(
            window_unavailable_message(),
            "drenyra-gpui: --window requires a native build; rebuild with `cargo run -p drenyra-app --features gpui -- --window`"
        );
    }

    #[test]
    fn window_flag_rejects_extra_arguments() {
        assert_eq!(
            run_cli(&["--window".into(), "--extra".into()]),
            ExitCode::FAILURE
        );
    }

    #[test]
    fn unrecognized_arguments_message_keeps_its_existing_text() {
        assert_eq!(run_cli(&["--bogus".into()]), ExitCode::FAILURE);
        assert_eq!(
            unrecognized_arguments_message(&["--bogus".into(), "x".into()]),
            "drenyra-gpui: unrecognized arguments: --bogus x (try --help, --check-env, --demo-session, or --probe-rpc)"
        );
    }
}
