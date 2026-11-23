//! Drenyra GPUI — native desktop host binary entrypoint.

use std::env;
use std::path::PathBuf;
use std::process::ExitCode;

use drenyra_protocol::MoneyCents;
use drenyra_workspace::FiscalWorkspace;

fn main() -> ExitCode {
    let args: Vec<String> = env::args().skip(1).collect();
    run_cli(&args)
}

pub fn run_cli(args: &[String]) -> ExitCode {
    match args {
        [] | [_] if matches!(args.first().map(String::as_str), None | Some("--help" | "-h")) => {
            println!("drenyra-gpui {}", env!("CARGO_PKG_VERSION"));
            println!("Native Rust + GPUI accounting desktop host for Drenyra Shell & Drenyra AI.");
            println!();
            println!("Usage:");
            println!("  drenyra-gpui --help        Show this help message");
            println!("  drenyra-gpui --check-env   Print headless RPC host environment diagnostics (JSON)");
            ExitCode::SUCCESS
        }
        [flag] if flag == "--check-env" => {
            let shell_found = find_in_path("drenyra-shell").or_else(|| find_in_path("pi"));
            let kernel_found = find_in_path("drenyra-ai");
            let ws = FiscalWorkspace::new("00000000000", "Diagnostic Entity", "2026-01");
            println!(
                r#"{{"app":"drenyra-gpui","version":"{}","zero_float_money":"{}","rda_default":{},"shell_bin_found":{},"kernel_bin_found":{}}}"#,
                env!("CARGO_PKG_VERSION"),
                MoneyCents::ZERO,
                ws.guardrails.rda_enabled,
                shell_found.is_some(),
                kernel_found.is_some()
            );
            ExitCode::SUCCESS
        }
        _ => {
            eprintln!(
                "drenyra-gpui: unrecognized arguments: {} (try --help or --check-env)",
                args.join(" ")
            );
            ExitCode::FAILURE
        }
    }
}

fn find_in_path(binary: &str) -> Option<PathBuf> {
    let path_var = env::var_os("PATH")?;
    for dir in env::split_paths(&path_var) {
        let candidate = dir.join(binary);
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn help_flag_exits_successfully() {
        assert_eq!(run_cli(&["--help".into()]), ExitCode::SUCCESS);
    }

    #[test]
    fn check_env_flag_exits_successfully() {
        assert_eq!(run_cli(&["--check-env".into()]), ExitCode::SUCCESS);
    }

    #[test]
    fn unknown_flag_returns_failure() {
        assert_eq!(run_cli(&["--unknown-flag".into()]), ExitCode::FAILURE);
    }
}
