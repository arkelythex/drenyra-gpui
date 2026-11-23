//! Live NDJSON stdio JSON-RPC 2.0 subprocess supervisor for Drenyra GPUI.

use std::fmt;
use std::io::{self, BufRead, BufReader, BufWriter, Write};
use std::process::{Child, ChildStderr, ChildStdin, ChildStdout, Command, Stdio};

use drenyra_protocol::{JsonRpcRequest, ProtocolError};

/// Errors produced while spawning or communicating with a JSON-RPC stdio subprocess.
#[derive(Debug)]
pub enum RpcError {
    Io(io::Error),
    Protocol(ProtocolError),
    UnexpectedEof,
    EmptyCommand,
}

impl fmt::Display for RpcError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(err) => write!(f, "RPC I/O error: {err}"),
            Self::Protocol(err) => write!(f, "RPC protocol error: {err}"),
            Self::UnexpectedEof => f.write_str("RPC subprocess closed stdio before replying (EOF)"),
            Self::EmptyCommand => f.write_str("RPC transport command must not be empty"),
        }
    }
}

impl std::error::Error for RpcError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(err) => Some(err),
            Self::Protocol(err) => Some(err),
            Self::UnexpectedEof | Self::EmptyCommand => None,
        }
    }
}

impl From<io::Error> for RpcError {
    fn from(err: io::Error) -> Self {
        Self::Io(err)
    }
}

impl From<ProtocolError> for RpcError {
    fn from(err: ProtocolError) -> Self {
        Self::Protocol(err)
    }
}

/// Configuration for spawning an NDJSON stdio RPC child process.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RpcTransportConfig {
    pub program: String,
    pub args: Vec<String>,
}

/// Supervised JSON-RPC 2.0 child process session communicating via newline-delimited JSON.
pub struct RpcSession {
    child: Child,
    stdin: Option<BufWriter<ChildStdin>>,
    stdout: BufReader<ChildStdout>,
    stderr: Option<ChildStderr>,
}

impl RpcSession {
    /// Spawns a child process with piped `stdin`, `stdout`, and `stderr`.
    pub fn spawn(config: &RpcTransportConfig) -> Result<Self, RpcError> {
        if config.program.trim().is_empty() {
            return Err(RpcError::EmptyCommand);
        }

        let mut child = Command::new(&config.program)
            .args(&config.args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?;

        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| io::Error::other("missing piped child stdin"))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| io::Error::other("missing piped child stdout"))?;
        let stderr = child.stderr.take();

        Ok(Self {
            child,
            stdin: Some(BufWriter::new(stdin)),
            stdout: BufReader::new(stdout),
            stderr,
        })
    }

    /// Returns the OS process ID of the supervised child process.
    pub fn child_id(&self) -> u32 {
        self.child.id()
    }

    /// Takes ownership of the child's piped `stderr` stream for diagnostics.
    pub fn take_stderr(&mut self) -> Option<ChildStderr> {
        self.stderr.take()
    }

    /// Sends one validated NDJSON `JsonRpcRequest` line and reads one NDJSON response line.
    pub fn request(&mut self, req: &JsonRpcRequest) -> Result<String, RpcError> {
        let frame = req.to_ndjson()?;

        let stdin = self
            .stdin
            .as_mut()
            .ok_or(RpcError::UnexpectedEof)?;

        if let Err(err) = write_frame(stdin, &frame) {
            if err.kind() == io::ErrorKind::BrokenPipe {
                return Err(RpcError::UnexpectedEof);
            }
            return Err(RpcError::Io(err));
        }

        let mut line = String::new();
        let bytes_read = self.stdout.read_line(&mut line)?;
        if bytes_read == 0 {
            return Err(RpcError::UnexpectedEof);
        }

        Ok(line.trim_end_matches(&['\r', '\n'][..]).to_string())
    }
}

fn write_frame(stdin: &mut BufWriter<ChildStdin>, frame: &str) -> io::Result<()> {
    stdin.write_all(frame.as_bytes())?;
    stdin.write_all(b"\n")?;
    stdin.flush()
}

impl Drop for RpcSession {
    fn drop(&mut self) {
        // Close stdin first so cooperative subprocesses can exit cleanly on EOF.
        drop(self.stdin.take());

        match self.child.try_wait() {
            Ok(Some(_status)) => {}
            Ok(None) | Err(_) => {
                let _ = self.child.kill();
                let _ = self.child.wait();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_command_is_rejected_without_spawning() {
        let cfg = RpcTransportConfig {
            program: "   ".into(),
            args: vec![],
        };
        let err = match RpcSession::spawn(&cfg) {
            Ok(_) => panic!("expected EmptyCommand error"),
            Err(err) => err,
        };
        assert!(matches!(err, RpcError::EmptyCommand));
    }

    #[cfg(unix)]
    #[test]
    fn bidirectional_request_response_roundtrip_over_stdio() {
        let cfg = RpcTransportConfig {
            program: "sh".into(),
            args: vec![
                "-c".into(),
                r#"IFS= read -r line && printf '{"jsonrpc":"2.0","id":1,"result":{"echo":%s}}\n' "$line""#.into(),
            ],
        };
        let mut session = RpcSession::spawn(&cfg).expect("spawn mock rpc responder");
        let req = JsonRpcRequest {
            id: 1,
            method: "initialize".into(),
            params_json: r#"{"client":"drenyra-gpui"}"#.into(),
        };
        let resp = session.request(&req).expect("receive ndjson response");
        assert!(resp.contains(r#""jsonrpc":"2.0""#));
        assert!(resp.contains(r#""method":"initialize""#));
    }

    #[cfg(unix)]
    #[test]
    fn unexpected_eof_when_child_exits_without_replying() {
        let cfg = RpcTransportConfig {
            program: "sh".into(),
            args: vec!["-c".into(), "exit 0".into()],
        };
        let mut session = RpcSession::spawn(&cfg).expect("spawn short-lived child");
        let req = JsonRpcRequest {
            id: 1,
            method: "ping".into(),
            params_json: "{}".into(),
        };
        let err = session.request(&req).expect_err("expected UnexpectedEof");
        assert!(matches!(err, RpcError::UnexpectedEof));
    }

    #[cfg(unix)]
    #[test]
    fn drop_terminates_and_reaps_long_running_child_without_zombie() {
        let cfg = RpcTransportConfig {
            program: "sh".into(),
            args: vec!["-c".into(), "exec sleep 60".into()],
        };
        let session = RpcSession::spawn(&cfg).expect("spawn long-running child");
        let pid = session.child_id();
        drop(session);

        // If the child were still alive or left as an unreaped zombie, `kill -0` would succeed.
        let probe = std::process::Command::new("kill")
            .args(["-0", &pid.to_string()])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .expect("run kill -0");
        assert!(
            !probe.success(),
            "child pid {pid} should be terminated and reaped on Drop"
        );
    }
}
