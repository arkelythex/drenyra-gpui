//! Live NDJSON stdio JSON-RPC 2.0 subprocess supervisor for Drenyra GPUI.

use std::fmt;
use std::io::{self, BufRead, BufReader, BufWriter, Read, Write};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::{mpsc, Arc, Condvar, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use drenyra_protocol::{JsonRpcRequest, ProtocolError};

/// Default timeout for a single JSON-RPC request waiting for a response line.
pub const DEFAULT_REQUEST_TIMEOUT: Duration = Duration::from_secs(30);

/// Default maximum byte length of a single JSON-RPC response line (4 MiB).
pub const DEFAULT_MAX_RESPONSE_BYTES: usize = 4 * 1024 * 1024;

/// Maximum bytes retained in the bounded background `stderr` diagnostic buffer (64 KiB).
const STDERR_BUFFER_CAP: usize = 64 * 1024;

/// Errors produced while spawning or communicating with a JSON-RPC stdio subprocess.
#[derive(Debug)]
pub enum RpcError {
    Io(io::Error),
    Protocol(ProtocolError),
    UnexpectedEof,
    EmptyCommand,
    Timeout,
    ResponseTooLarge { limit: usize },
    SessionPoisoned,
}

impl fmt::Display for RpcError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(err) => write!(f, "RPC I/O error: {err}"),
            Self::Protocol(err) => write!(f, "RPC protocol error: {err}"),
            Self::UnexpectedEof => f.write_str("RPC subprocess closed stdio before replying (EOF)"),
            Self::EmptyCommand => f.write_str("RPC transport command must not be empty"),
            Self::Timeout => f.write_str("RPC request timed out waiting for response"),
            Self::ResponseTooLarge { limit } => {
                write!(f, "RPC response exceeded max_response_bytes limit ({limit})")
            }
            Self::SessionPoisoned => f.write_str(
                "RPC session is poisoned after a timeout or framing violation; spawn a new session",
            ),
        }
    }
}

impl std::error::Error for RpcError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(err) => Some(err),
            Self::Protocol(err) => Some(err),
            Self::UnexpectedEof
            | Self::EmptyCommand
            | Self::Timeout
            | Self::ResponseTooLarge { .. }
            | Self::SessionPoisoned => None,
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
    pub request_timeout: Duration,
    pub max_response_bytes: usize,
}

impl RpcTransportConfig {
    /// Builds a transport configuration with safe defaults (`30s` timeout, `4 MiB` response cap).
    pub fn new(program: impl Into<String>, args: Vec<String>) -> Self {
        Self {
            program: program.into(),
            args,
            request_timeout: DEFAULT_REQUEST_TIMEOUT,
            max_response_bytes: DEFAULT_MAX_RESPONSE_BYTES,
        }
    }
}

/// Supervised JSON-RPC 2.0 child process session communicating via newline-delimited JSON.
pub struct RpcSession {
    child: Child,
    stdin: Option<BufWriter<ChildStdin>>,
    stdout_rx: mpsc::Receiver<Result<Option<String>, RpcError>>,
    request_timeout: Duration,
    poisoned: bool,
    stderr_buffer: Arc<Mutex<Vec<u8>>>,
    stderr_done: Arc<(Mutex<bool>, Condvar)>,
    stdout_thread: Option<JoinHandle<()>>,
    stderr_thread: Option<JoinHandle<()>>,
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
        let mut stderr = child
            .stderr
            .take()
            .ok_or_else(|| io::Error::other("missing piped child stderr"))?;

        let stderr_buffer = Arc::new(Mutex::new(Vec::new()));
        let stderr_done = Arc::new((Mutex::new(false), Condvar::new()));

        let stderr_buf_clone = Arc::clone(&stderr_buffer);
        let stderr_done_clone = Arc::clone(&stderr_done);
        let stderr_thread = thread::spawn(move || {
            let mut chunk = [0u8; 4096];
            loop {
                match stderr.read(&mut chunk) {
                    Ok(0) => break,
                    Ok(n) => {
                        let mut buf = match stderr_buf_clone.lock() {
                            Ok(guard) => guard,
                            Err(poisoned) => poisoned.into_inner(),
                        };
                        let remaining = STDERR_BUFFER_CAP.saturating_sub(buf.len());
                        if remaining > 0 {
                            let to_copy = n.min(remaining);
                            buf.extend_from_slice(&chunk[..to_copy]);
                        }
                    }
                    Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                    Err(_) => break,
                }
            }
            let (lock, cvar) = &*stderr_done_clone;
            if let Ok(mut done) = lock.lock() {
                *done = true;
                cvar.notify_all();
            }
        });

        let (stdout_tx, stdout_rx) = mpsc::channel();
        let max_bytes = config.max_response_bytes;
        let stdout_thread = thread::spawn(move || {
            let mut reader = BufReader::new(stdout);
            loop {
                let res = read_bounded_line(&mut reader, max_bytes);
                let is_terminal = !matches!(res, Ok(Some(_)));
                if stdout_tx.send(res).is_err() || is_terminal {
                    break;
                }
            }
        });

        Ok(Self {
            child,
            stdin: Some(BufWriter::new(stdin)),
            stdout_rx,
            request_timeout: config.request_timeout,
            poisoned: false,
            stderr_buffer,
            stderr_done,
            stdout_thread: Some(stdout_thread),
            stderr_thread: Some(stderr_thread),
        })
    }

    /// Returns the OS process ID of the supervised child process.
    pub fn child_id(&self) -> u32 {
        self.child.id()
    }

    /// Returns a lossy UTF-8 snapshot of the bounded `stderr` buffer (capped at 64 KiB).
    pub fn stderr_snapshot(&self) -> String {
        match self.stderr_buffer.lock() {
            Ok(buf) => String::from_utf8_lossy(&buf).into_owned(),
            Err(poisoned) => String::from_utf8_lossy(&poisoned.into_inner()).into_owned(),
        }
    }

    /// Sends one validated NDJSON `JsonRpcRequest` line and reads one NDJSON response line.
    pub fn request(&mut self, req: &JsonRpcRequest) -> Result<String, RpcError> {
        if self.poisoned {
            return Err(RpcError::SessionPoisoned);
        }

        let frame = req.to_ndjson()?;

        let Some(stdin) = self.stdin.as_mut() else {
            self.poisoned = true;
            return Err(RpcError::UnexpectedEof);
        };

        if let Err(err) = write_frame(stdin, &frame) {
            self.wait_stderr_briefly();
            self.poisoned = true;
            if err.kind() == io::ErrorKind::BrokenPipe {
                return Err(RpcError::UnexpectedEof);
            }
            return Err(RpcError::Io(err));
        }

        match self.stdout_rx.recv_timeout(self.request_timeout) {
            Ok(Ok(Some(line))) => Ok(line),
            Ok(Ok(None)) | Err(mpsc::RecvTimeoutError::Disconnected) => {
                self.wait_stderr_briefly();
                self.poisoned = true;
                Err(RpcError::UnexpectedEof)
            }
            Ok(Err(err)) => {
                self.wait_stderr_briefly();
                self.poisoned = true;
                Err(err)
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {
                self.poisoned = true;
                Err(RpcError::Timeout)
            }
        }
    }

    fn wait_stderr_briefly(&self) {
        let (lock, cvar) = &*self.stderr_done;
        let deadline = Instant::now() + Duration::from_millis(20);
        if let Ok(mut done) = lock.lock() {
            while !*done {
                let remaining = deadline.saturating_duration_since(Instant::now());
                if remaining.is_zero() {
                    break;
                }
                let (guard, timeout_res) = match cvar.wait_timeout(done, remaining) {
                    Ok(res) => res,
                    Err(poisoned) => poisoned.into_inner(),
                };
                done = guard;
                if timeout_res.timed_out() {
                    break;
                }
            }
        }
    }
}

fn read_bounded_line(
    reader: &mut BufReader<ChildStdout>,
    max_bytes: usize,
) -> Result<Option<String>, RpcError> {
    let mut line = Vec::new();
    loop {
        let available = match reader.fill_buf() {
            Ok(buf) => buf,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(RpcError::Io(e)),
        };

        if available.is_empty() {
            if line.is_empty() {
                return Ok(None);
            }
            while line.last() == Some(&b'\r') {
                line.pop();
            }
            let text = String::from_utf8(line)
                .map_err(|e| RpcError::Io(io::Error::new(io::ErrorKind::InvalidData, e)))?;
            return Ok(Some(text));
        }

        if let Some(newline_pos) = available.iter().position(|&b| b == b'\n') {
            let slice = &available[..newline_pos];
            if line.len().saturating_add(slice.len()) > max_bytes {
                reader.consume(newline_pos + 1);
                return Err(RpcError::ResponseTooLarge { limit: max_bytes });
            }
            line.extend_from_slice(slice);
            reader.consume(newline_pos + 1);
            while line.last() == Some(&b'\r') {
                line.pop();
            }
            let text = String::from_utf8(line)
                .map_err(|e| RpcError::Io(io::Error::new(io::ErrorKind::InvalidData, e)))?;
            return Ok(Some(text));
        }

        let len = available.len();
        if line.len().saturating_add(len) > max_bytes {
            reader.consume(len);
            return Err(RpcError::ResponseTooLarge { limit: max_bytes });
        }
        line.extend_from_slice(available);
        reader.consume(len);
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

        let mut exited = false;
        for _ in 0..10 {
            match self.child.try_wait() {
                Ok(Some(_)) => {
                    exited = true;
                    break;
                }
                Ok(None) => thread::sleep(Duration::from_millis(5)),
                Err(_) => break,
            }
        }
        if !exited {
            if let Ok(Some(_)) = self.child.try_wait() {
                exited = true;
            }
        }
        if !exited {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }

        let join_deadline = Instant::now() + Duration::from_millis(50);
        let mut stdout_handle = self.stdout_thread.take();
        let mut stderr_handle = self.stderr_thread.take();
        loop {
            if let Some(handle) = stdout_handle.take() {
                if handle.is_finished() {
                    let _ = handle.join();
                } else {
                    stdout_handle = Some(handle);
                }
            }
            if let Some(handle) = stderr_handle.take() {
                if handle.is_finished() {
                    let _ = handle.join();
                } else {
                    stderr_handle = Some(handle);
                }
            }
            if (stdout_handle.is_none() && stderr_handle.is_none()) || Instant::now() >= join_deadline
            {
                break;
            }
            thread::sleep(Duration::from_millis(2));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_command_is_rejected_without_spawning() {
        let cfg = RpcTransportConfig::new("   ", vec![]);
        let err = match RpcSession::spawn(&cfg) {
            Ok(_) => panic!("expected EmptyCommand error"),
            Err(err) => err,
        };
        assert!(matches!(err, RpcError::EmptyCommand));
    }

    #[cfg(unix)]
    #[test]
    fn bidirectional_request_response_roundtrip_over_stdio() {
        let cfg = RpcTransportConfig::new(
            "sh",
            vec![
                "-c".into(),
                r#"IFS= read -r line && printf '{"jsonrpc":"2.0","id":1,"result":{"echo":%s}}\n' "$line""#.into(),
            ],
        );
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
        let cfg = RpcTransportConfig::new("sh", vec!["-c".into(), "exit 0".into()]);
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
        let cfg = RpcTransportConfig::new("sh", vec!["-c".into(), "exec sleep 60".into()]);
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

    #[test]
    fn transport_config_new_sets_safe_defaults() {
        let cfg = RpcTransportConfig::new("drenyra-shell", vec!["--mode".into(), "rpc".into()]);
        assert_eq!(cfg.program, "drenyra-shell");
        assert_eq!(cfg.args, vec!["--mode", "rpc"]);
        assert_eq!(cfg.request_timeout, Duration::from_secs(30));
        assert_eq!(cfg.max_response_bytes, 4 * 1024 * 1024);
    }

    #[cfg(unix)]
    #[test]
    fn noisy_stderr_exceeding_pipe_buffer_does_not_deadlock_and_is_bounded() {
        // Write > 128 KiB to stderr before writing one reply line to stdout.
        let script = concat!(
            "i=0; chunk='0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef'; ",
            "line=\"$chunk$chunk$chunk$chunk$chunk$chunk$chunk$chunk\"; ",
            "while [ $i -lt 260 ]; do printf '%s\\n' \"$line\" >&2; i=$((i+1)); done; ",
            "IFS= read -r req && printf '{\"jsonrpc\":\"2.0\",\"id\":1,\"result\":\"ok\"}\\n'"
        );
        let mut cfg = RpcTransportConfig::new("sh", vec!["-c".into(), script.into()]);
        cfg.request_timeout = Duration::from_secs(5);
        let mut session = RpcSession::spawn(&cfg).expect("spawn noisy child");
        let req = JsonRpcRequest {
            id: 1,
            method: "ping".into(),
            params_json: "{}".into(),
        };
        let resp = session.request(&req).expect("noisy child should not deadlock");
        assert!(resp.contains(r#""result":"ok""#));

        let snap = session.stderr_snapshot();
        assert!(!snap.is_empty());
        assert!(
            snap.len() <= 64 * 1024,
            "stderr_snapshot length {} exceeded 64 KiB cap",
            snap.len()
        );
    }

    #[cfg(unix)]
    #[test]
    fn request_times_out_when_child_hangs() {
        let mut cfg = RpcTransportConfig::new("sh", vec!["-c".into(), "exec sleep 10".into()]);
        cfg.request_timeout = Duration::from_millis(80);
        let mut session = RpcSession::spawn(&cfg).expect("spawn hanging child");
        let req = JsonRpcRequest {
            id: 1,
            method: "ping".into(),
            params_json: "{}".into(),
        };
        let err = session.request(&req).expect_err("expected Timeout");
        assert!(matches!(err, RpcError::Timeout), "got {err:?}");
    }

    #[cfg(unix)]
    #[test]
    fn response_exceeding_max_response_bytes_returns_response_too_large() {
        let script = r#"IFS= read -r _; printf '%0256d\n' 0"#;
        let mut cfg = RpcTransportConfig::new("sh", vec!["-c".into(), script.into()]);
        cfg.max_response_bytes = 64;
        let mut session = RpcSession::spawn(&cfg).expect("spawn oversized responder");
        let req = JsonRpcRequest {
            id: 1,
            method: "ping".into(),
            params_json: "{}".into(),
        };
        let err = session
            .request(&req)
            .expect_err("expected ResponseTooLarge");
        assert!(
            matches!(err, RpcError::ResponseTooLarge { limit: 64 }),
            "got {err:?}"
        );
    }

    #[cfg(unix)]
    #[test]
    fn drop_allows_cooperative_child_to_exit_cleanly_without_sigkill() {
        let marker = std::env::temp_dir().join(format!(
            "drenyra-rpc-coop-{}-{}.marker",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        ));
        let _ = std::fs::remove_file(&marker);

        let script = r#"cat >/dev/null; sleep 0.01; printf 'clean' > "$1""#;
        let cfg = RpcTransportConfig::new(
            "sh",
            vec![
                "-c".into(),
                script.into(),
                "_".into(),
                marker.to_string_lossy().into_owned(),
            ],
        );
        let session = RpcSession::spawn(&cfg).expect("spawn cooperative child");
        drop(session);

        let contents = std::fs::read_to_string(&marker).unwrap_or_default();
        let _ = std::fs::remove_file(&marker);
        assert_eq!(
            contents, "clean",
            "cooperative child should exit cleanly on stdin EOF before SIGKILL"
        );
    }

    #[cfg(unix)]
    #[test]
    fn reusing_timed_out_session_returns_session_poisoned_instead_of_stale_reply() {
        let script = r#"IFS= read -r _; sleep 0.1; printf '{"jsonrpc":"2.0","id":1,"result":"late"}\n'; IFS= read -r _"#;
        let mut cfg = RpcTransportConfig::new("sh", vec!["-c".into(), script.into()]);
        cfg.request_timeout = Duration::from_millis(20);
        let mut session = RpcSession::spawn(&cfg).expect("spawn delayed responder");

        let req1 = JsonRpcRequest {
            id: 1,
            method: "ping".into(),
            params_json: "{}".into(),
        };
        let err1 = session.request(&req1).expect_err("expected Timeout");
        assert!(matches!(err1, RpcError::Timeout), "got {err1:?}");

        // Wait until the delayed reply has definitely arrived in the stdout channel.
        thread::sleep(Duration::from_millis(120));

        let req2 = JsonRpcRequest {
            id: 2,
            method: "ping".into(),
            params_json: "{}".into(),
        };
        let err2 = session
            .request(&req2)
            .expect_err("expected SessionPoisoned on second request");
        assert!(
            matches!(err2, RpcError::SessionPoisoned),
            "expected SessionPoisoned, got {err2:?}"
        );
        assert_eq!(
            err2.to_string(),
            "RPC session is poisoned after a timeout or framing violation; spawn a new session"
        );
    }

    #[cfg(unix)]
    #[test]
    fn wait_stderr_briefly_continues_waiting_after_spurious_wakeup_until_done() {
        let cfg = RpcTransportConfig::new("sh", vec!["-c".into(), "exec sleep 1".into()]);
        let session = RpcSession::spawn(&cfg).expect("spawn child");

        let done_pair = Arc::clone(&session.stderr_done);
        let notifier = thread::spawn(move || {
            thread::sleep(Duration::from_millis(3));
            // Spurious wakeup while `*done == false`
            done_pair.1.notify_all();
            thread::sleep(Duration::from_millis(5));
            // Real completion
            if let Ok(mut done) = done_pair.0.lock() {
                *done = true;
                done_pair.1.notify_all();
            }
        });

        let start = Instant::now();
        session.wait_stderr_briefly();
        let elapsed = start.elapsed();
        let _ = notifier.join();

        assert!(
            elapsed >= Duration::from_millis(7),
            "wait_stderr_briefly returned too early on spurious wakeup ({elapsed:?})"
        );
    }

    #[cfg(unix)]
    #[test]
    fn reusing_oversized_response_session_returns_session_poisoned() {
        let script = r#"IFS= read -r _; printf '%0256d\n' 0; IFS= read -r _; printf '{"jsonrpc":"2.0","id":2,"result":"ok"}\n'"#;
        let mut cfg = RpcTransportConfig::new("sh", vec!["-c".into(), script.into()]);
        cfg.max_response_bytes = 64;
        let mut session = RpcSession::spawn(&cfg).expect("spawn oversized responder");

        let req1 = JsonRpcRequest {
            id: 1,
            method: "ping".into(),
            params_json: "{}".into(),
        };
        let err1 = session
            .request(&req1)
            .expect_err("expected ResponseTooLarge");
        assert!(
            matches!(err1, RpcError::ResponseTooLarge { limit: 64 }),
            "got {err1:?}"
        );

        let req2 = JsonRpcRequest {
            id: 2,
            method: "ping".into(),
            params_json: "{}".into(),
        };
        let err2 = session
            .request(&req2)
            .expect_err("expected SessionPoisoned after framing violation");
        assert!(
            matches!(err2, RpcError::SessionPoisoned),
            "expected SessionPoisoned, got {err2:?}"
        );
    }

    #[cfg(unix)]
    #[test]
    fn drop_returns_promptly_even_when_grandchild_inherits_pipes() {
        let cfg = RpcTransportConfig::new("sh", vec!["-c".into(), "(sleep 2) & exit 0".into()]);
        let session = RpcSession::spawn(&cfg).expect("spawn child with detached grandchild");
        let start = std::time::Instant::now();
        drop(session);
        let elapsed = start.elapsed();
        assert!(
            elapsed < Duration::from_millis(500),
            "Drop took {elapsed:?}, expected < 500ms even with detached grandchild holding pipes"
        );
    }
}
