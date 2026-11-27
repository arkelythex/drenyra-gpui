//! Client for the agent shell's own stdio protocol: `pi --mode rpc`, which `drenyra-shell` extends.
//!
//! This is **not** JSON-RPC 2.0 (that is [`crate::RpcSession`]). The agent reads LF-delimited
//! command objects `{"id"?: string, "type": "<command>", ...}` and writes three kinds of
//! records: `{"type":"response",...}` replies that echo the command's string `id`, session events
//! (`agent_start`, `message_update`, `agent_settled`, ...) that stream for as long as a run lasts,
//! and `extension_ui_request` records. Replies arrive asynchronously, so commands such as
//! `abort` can be sent while a run is streaming.
//!
//! Dialog requests from extensions (`select`, `confirm`, `input`, `editor`) block the agent until
//! answered. Until the window can render them, [`AgentSession`] answers each one with
//! `cancelled: true` and still forwards it, so the operator sees what was asked. The window
//! never answers on the operator's behalf; what an extension does after a cancelled dialog is
//! up to that extension.

use std::fmt;
use std::io::{self, BufReader, BufWriter, Read, Write};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::sync::{Arc, Mutex, TryLockError};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use serde_json::{json, Value};

use crate::{read_bounded_line, RpcError};

/// Largest record accepted from the agent. Long sessions can emit big `agent_end` payloads.
pub const DEFAULT_MAX_RECORD_BYTES: usize = 16 * 1024 * 1024;
/// Time the agent gets to finish pending work after stdin closes, before it is killed.
pub const DEFAULT_AGENT_SHUTDOWN_GRACE: Duration = Duration::from_secs(3);
const STDERR_CAP: usize = 64 * 1024;

/// Extension UI methods that block the agent until the client answers.
const DIALOG_METHODS: [&str; 4] = ["select", "confirm", "input", "editor"];

/// One record from the agent, reduced to what the window shows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AgentEvent {
    /// `agent_start`: a run began.
    RunStarted,
    /// A chunk of assistant text (`message_update` / `text_delta`).
    TextDelta(String),
    /// `message_end` of an assistant message: the authoritative final text and stop reason
    /// (`"stop"`, `"error"`, `"aborted"`, ...), with the provider error when there is one.
    AssistantMessage {
        text: String,
        stop_reason: String,
        error: Option<String>,
    },
    /// `tool_execution_start`.
    ToolStarted { tool: String },
    /// `tool_execution_end`.
    ToolFinished { tool: String, is_error: bool },
    /// `auto_retry_start`: the agent retries after a provider error.
    RetryScheduled {
        attempt: u64,
        max_attempts: u64,
        error: String,
    },
    /// `agent_settled`: nothing more happens until the next command.
    Settled { aborted: bool },
    /// Reply to `get_state`: model as `provider/id` and whether a run is streaming.
    State {
        model: Option<String>,
        streaming: bool,
    },
    /// Any other command reply. `id` is `None` for replies to unparseable commands.
    Response {
        id: Option<String>,
        command: String,
        success: bool,
        error: Option<String>,
    },
    /// `extension_ui_request`. `dialog` is true for methods that block until answered.
    UiRequest {
        id: String,
        method: String,
        title: String,
        dialog: bool,
    },
    /// `extension_error`.
    ExtensionError(String),
    /// A record type the window does not use (`turn_start`, `queue_update`, ...).
    Other(String),
}

/// A line the agent wrote that is not a protocol record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AgentProtocolError {
    NotJson(String),
    MissingType,
}

impl fmt::Display for AgentProtocolError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotJson(err) => write!(f, "agent wrote a line that is not JSON: {err}"),
            Self::MissingType => write!(f, "agent wrote a record without a string \"type\""),
        }
    }
}

impl std::error::Error for AgentProtocolError {}

fn text_field(value: &Value, key: &str) -> Option<String> {
    value.get(key).and_then(Value::as_str).map(str::to_owned)
}

/// Joins the `text` parts of a message `content` array (or returns it when it is a string).
fn message_text(message: &Value) -> String {
    match message.get("content") {
        Some(Value::String(text)) => text.clone(),
        Some(Value::Array(parts)) => parts
            .iter()
            .filter(|part| part.get("type").and_then(Value::as_str) == Some("text"))
            .filter_map(|part| part.get("text").and_then(Value::as_str))
            .collect(),
        _ => String::new(),
    }
}

/// Parses one line written by the agent.
pub fn parse_record(line: &str) -> Result<AgentEvent, AgentProtocolError> {
    let value: Value =
        serde_json::from_str(line).map_err(|err| AgentProtocolError::NotJson(err.to_string()))?;
    let kind = value
        .get("type")
        .and_then(Value::as_str)
        .ok_or(AgentProtocolError::MissingType)?;
    Ok(match kind {
        "agent_start" => AgentEvent::RunStarted,
        "message_update" => {
            let update = value.get("assistantMessageEvent").unwrap_or(&Value::Null);
            match (
                update.get("type").and_then(Value::as_str),
                text_field(update, "delta"),
            ) {
                (Some("text_delta"), Some(delta)) => AgentEvent::TextDelta(delta),
                _ => AgentEvent::Other(kind.to_owned()),
            }
        }
        "message_end" => {
            let message = value.get("message").unwrap_or(&Value::Null);
            if message.get("role").and_then(Value::as_str) == Some("assistant") {
                AgentEvent::AssistantMessage {
                    text: message_text(message),
                    stop_reason: text_field(message, "stopReason").unwrap_or_default(),
                    error: text_field(message, "errorMessage"),
                }
            } else {
                AgentEvent::Other(kind.to_owned())
            }
        }
        "tool_execution_start" => AgentEvent::ToolStarted {
            tool: text_field(&value, "toolName").unwrap_or_default(),
        },
        "tool_execution_end" => AgentEvent::ToolFinished {
            tool: text_field(&value, "toolName").unwrap_or_default(),
            is_error: value
                .get("isError")
                .and_then(Value::as_bool)
                .unwrap_or(false),
        },
        "auto_retry_start" => AgentEvent::RetryScheduled {
            attempt: value.get("attempt").and_then(Value::as_u64).unwrap_or(0),
            max_attempts: value
                .get("maxAttempts")
                .and_then(Value::as_u64)
                .unwrap_or(0),
            error: text_field(&value, "errorMessage").unwrap_or_default(),
        },
        "agent_settled" => AgentEvent::Settled {
            aborted: value
                .get("aborted")
                .and_then(Value::as_bool)
                .unwrap_or(false),
        },
        "response" => {
            let command = text_field(&value, "command").unwrap_or_default();
            let success = value
                .get("success")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            let data = value.get("data");
            match data.filter(|_| success && command == "get_state") {
                Some(data) => {
                    let model = data.get("model").and_then(|model| {
                        let id = model.get("id").and_then(Value::as_str)?;
                        Some(match model.get("provider").and_then(Value::as_str) {
                            Some(provider) => format!("{provider}/{id}"),
                            None => id.to_owned(),
                        })
                    });
                    AgentEvent::State {
                        model,
                        streaming: data
                            .get("isStreaming")
                            .and_then(Value::as_bool)
                            .unwrap_or(false),
                    }
                }
                None => AgentEvent::Response {
                    id: text_field(&value, "id"),
                    command,
                    success,
                    error: text_field(&value, "error"),
                },
            }
        }
        "extension_ui_request" => {
            let method = text_field(&value, "method").unwrap_or_default();
            AgentEvent::UiRequest {
                id: text_field(&value, "id").unwrap_or_default(),
                dialog: DIALOG_METHODS.contains(&method.as_str()),
                title: text_field(&value, "title")
                    .or_else(|| text_field(&value, "message"))
                    .unwrap_or_default(),
                method,
            }
        }
        "extension_error" => {
            AgentEvent::ExtensionError(text_field(&value, "error").unwrap_or_default())
        }
        other => AgentEvent::Other(other.to_owned()),
    })
}

/// `prompt` command. While a run is streaming the agent requires a `streamingBehavior`; this
/// queues the message as a follow-up instead of interrupting the run.
pub fn prompt_command(id: &str, message: &str, streaming: bool) -> String {
    let mut command = json!({ "id": id, "type": "prompt", "message": message });
    if streaming {
        command["streamingBehavior"] = json!("followUp");
    }
    command.to_string()
}

/// `get_state` command (does not call a model).
pub fn get_state_command(id: &str) -> String {
    json!({ "id": id, "type": "get_state" }).to_string()
}

/// `clear_queue` followed by `abort`: the agent's recommended way to stop a run.
pub fn stop_commands() -> [String; 2] {
    [
        json!({ "type": "clear_queue" }).to_string(),
        json!({ "type": "abort" }).to_string(),
    ]
}

/// Answer to an extension dialog that declines it.
pub fn cancel_dialog_response(id: &str) -> String {
    json!({ "type": "extension_ui_response", "id": id, "cancelled": true }).to_string()
}

/// How to start the agent process.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentSessionConfig {
    pub program: String,
    pub args: Vec<String>,
    pub max_record_bytes: usize,
    pub shutdown_grace: Duration,
}

impl AgentSessionConfig {
    pub fn new(program: impl Into<String>, args: Vec<String>) -> Self {
        Self {
            program: program.into(),
            args,
            max_record_bytes: DEFAULT_MAX_RECORD_BYTES,
            shutdown_grace: DEFAULT_AGENT_SHUTDOWN_GRACE,
        }
    }
}

/// What the reader thread reports.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AgentUpdate {
    Event(AgentEvent),
    /// The agent broke the protocol (a non-JSON or oversized line, or unreadable stdout). The
    /// session stops reading; nothing after this is trusted.
    Failed(String),
    /// The agent closed stdout (it exited).
    Exited,
}

type SharedWriter = Arc<Mutex<Option<BufWriter<ChildStdin>>>>;

/// A running agent process speaking the agent protocol over stdio.
pub struct AgentSession {
    child: Child,
    writer: SharedWriter,
    updates: Receiver<AgentUpdate>,
    shutdown_grace: Duration,
    stderr: Arc<Mutex<Vec<u8>>>,
    threads: Vec<JoinHandle<()>>,
}

fn write_line(writer: &SharedWriter, frame: &str) -> io::Result<()> {
    let mut guard = writer
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let stdin = guard
        .as_mut()
        .ok_or_else(|| io::Error::new(io::ErrorKind::BrokenPipe, "agent stdin is closed"))?;
    stdin.write_all(frame.as_bytes())?;
    stdin.write_all(b"\n")?;
    stdin.flush()
}

impl AgentSession {
    /// Starts the agent with piped stdio and begins reading its records.
    pub fn spawn(config: &AgentSessionConfig) -> Result<Self, RpcError> {
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
            .ok_or_else(|| io::Error::other("missing stdin"))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| io::Error::other("missing stdout"))?;
        let mut stderr = child
            .stderr
            .take()
            .ok_or_else(|| io::Error::other("missing stderr"))?;

        let writer: SharedWriter = Arc::new(Mutex::new(Some(BufWriter::new(stdin))));
        let stderr_buffer = Arc::new(Mutex::new(Vec::new()));

        let stderr_sink = Arc::clone(&stderr_buffer);
        let stderr_thread = thread::spawn(move || {
            let mut chunk = [0u8; 4096];
            loop {
                match stderr.read(&mut chunk) {
                    Ok(0) => break,
                    Ok(n) => {
                        let mut buf = stderr_sink.lock().unwrap_or_else(|p| p.into_inner());
                        let room = STDERR_CAP.saturating_sub(buf.len());
                        buf.extend_from_slice(&chunk[..n.min(room)]);
                    }
                    Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                    Err(_) => break,
                }
            }
        });

        let (tx, updates) = mpsc::channel();
        let reply_writer = Arc::clone(&writer);
        let max_bytes = config.max_record_bytes;
        let stdout_thread = thread::spawn(move || {
            let mut reader = BufReader::new(stdout);
            loop {
                let update = match read_bounded_line(&mut reader, max_bytes) {
                    Ok(None) => AgentUpdate::Exited,
                    Err(err) => AgentUpdate::Failed(err.to_string()),
                    Ok(Some(line)) if line.trim().is_empty() => continue,
                    Ok(Some(line)) => match parse_record(&line) {
                        Ok(event) => {
                            if let AgentEvent::UiRequest {
                                id, dialog: true, ..
                            } = &event
                            {
                                // A dialog without an id cannot be answered. Best effort: if
                                // stdin is gone the agent is exiting anyway.
                                if !id.is_empty() {
                                    let _ = write_line(&reply_writer, &cancel_dialog_response(id));
                                }
                            }
                            AgentUpdate::Event(event)
                        }
                        Err(err) => AgentUpdate::Failed(err.to_string()),
                    },
                };
                let terminal = !matches!(update, AgentUpdate::Event(_));
                if tx.send(update).is_err() || terminal {
                    break;
                }
            }
        });

        Ok(Self {
            child,
            writer,
            updates,
            shutdown_grace: config.shutdown_grace,
            stderr: stderr_buffer,
            threads: vec![stdout_thread, stderr_thread],
        })
    }

    /// Sends one command line. Safe to call while a run is streaming.
    pub fn send(&self, frame: &str) -> Result<(), RpcError> {
        write_line(&self.writer, frame).map_err(RpcError::Io)
    }

    /// Returns every update received so far without blocking.
    pub fn drain(&self) -> Vec<AgentUpdate> {
        let mut out = Vec::new();
        loop {
            match self.updates.try_recv() {
                Ok(update) => out.push(update),
                Err(TryRecvError::Empty | TryRecvError::Disconnected) => return out,
            }
        }
    }

    /// Waits up to `timeout` for the next update (used by tests and probes).
    pub fn recv_timeout(&self, timeout: Duration) -> Option<AgentUpdate> {
        self.updates.recv_timeout(timeout).ok()
    }

    /// Lossy snapshot of the agent's stderr (capped at 64 KiB).
    pub fn stderr_snapshot(&self) -> String {
        let buf = self.stderr.lock().unwrap_or_else(|p| p.into_inner());
        String::from_utf8_lossy(&buf).into_owned()
    }
}

impl Drop for AgentSession {
    /// Closes stdin (the agent's shutdown signal), waits up to the grace period, then kills.
    ///
    /// The reader thread may hold the writer while a dialog answer is blocked on a full pipe
    /// (an agent that stopped reading stdin). Drop never waits on that lock past the grace
    /// period: it kills the agent, which fails the blocked write and frees the lock.
    fn drop(&mut self) {
        let deadline = Instant::now() + self.shutdown_grace;
        let mut killed = false;
        loop {
            match self.writer.try_lock() {
                Ok(mut guard) => {
                    guard.take();
                    break;
                }
                Err(TryLockError::Poisoned(poisoned)) => {
                    poisoned.into_inner().take();
                    break;
                }
                Err(TryLockError::WouldBlock) if Instant::now() < deadline => {
                    thread::sleep(Duration::from_millis(5));
                }
                Err(TryLockError::WouldBlock) => {
                    let _ = self.child.kill();
                    killed = true;
                    break;
                }
            }
        }
        let exited = killed
            || loop {
                match self.child.try_wait() {
                    Ok(Some(_)) => break true,
                    Ok(None) if Instant::now() < deadline => {
                        thread::sleep(Duration::from_millis(10))
                    }
                    Ok(None) | Err(_) => break false,
                }
            };
        if !exited {
            let _ = self.child.kill();
        }
        let _ = self.child.wait();
        let join_deadline = Instant::now() + Duration::from_millis(100);
        for handle in self.threads.drain(..) {
            while !handle.is_finished() && Instant::now() < join_deadline {
                thread::sleep(Duration::from_millis(2));
            }
            if handle.is_finished() {
                let _ = handle.join();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(line: &str) -> AgentEvent {
        parse_record(line).expect("valid record")
    }

    #[test]
    fn parses_the_streamed_run_records_from_the_protocol_docs() {
        assert_eq!(parse(r#"{"type":"agent_start"}"#), AgentEvent::RunStarted);
        assert_eq!(
            parse(
                r#"{"type":"message_update","usage":{},"assistantMessageEvent":{"type":"text_delta","contentIndex":0,"delta":"Hello "}}"#
            ),
            AgentEvent::TextDelta("Hello ".into())
        );
        assert_eq!(
            parse(
                r#"{"type":"message_update","assistantMessageEvent":{"type":"thinking_delta","delta":"x"}}"#
            ),
            AgentEvent::Other("message_update".into())
        );
        assert_eq!(
            parse(
                r#"{"type":"message_end","message":{"role":"assistant","content":[{"type":"text","text":"Hello "},{"type":"toolCall","id":"c"},{"type":"text","text":"world"}],"stopReason":"stop"}}"#
            ),
            AgentEvent::AssistantMessage {
                text: "Hello world".into(),
                stop_reason: "stop".into(),
                error: None
            }
        );
        assert_eq!(
            parse(
                r#"{"type":"message_end","message":{"role":"assistant","content":[],"stopReason":"error","errorMessage":"529 overloaded"}}"#
            ),
            AgentEvent::AssistantMessage {
                text: String::new(),
                stop_reason: "error".into(),
                error: Some("529 overloaded".into())
            }
        );
        assert_eq!(
            parse(r#"{"type":"message_end","message":{"role":"user","content":"hi"}}"#),
            AgentEvent::Other("message_end".into())
        );
        assert_eq!(
            parse(
                r#"{"type":"tool_execution_start","toolCallId":"call_abc123","toolName":"bash","args":{"command":"ls -la"}}"#
            ),
            AgentEvent::ToolStarted {
                tool: "bash".into()
            }
        );
        assert_eq!(
            parse(
                r#"{"type":"tool_execution_end","toolCallId":"call_abc123","toolName":"bash","result":{"content":[]},"isError":true}"#
            ),
            AgentEvent::ToolFinished {
                tool: "bash".into(),
                is_error: true
            }
        );
        assert_eq!(
            parse(
                r#"{"type":"auto_retry_start","attempt":1,"maxAttempts":3,"delayMs":2000,"errorMessage":"529 overloaded"}"#
            ),
            AgentEvent::RetryScheduled {
                attempt: 1,
                max_attempts: 3,
                error: "529 overloaded".into()
            }
        );
        assert_eq!(
            parse(r#"{"type":"agent_settled","aborted":true}"#),
            AgentEvent::Settled { aborted: true }
        );
        assert_eq!(
            parse(r#"{"type":"turn_start"}"#),
            AgentEvent::Other("turn_start".into())
        );
    }

    #[test]
    fn parses_replies_including_state_errors_and_id_less_parse_failures() {
        assert_eq!(
            parse(
                r#"{"id":"req-1","type":"response","command":"get_state","success":true,"data":{"model":{"id":"gpt-6.1-sol","provider":"openai"},"isStreaming":false}}"#
            ),
            AgentEvent::State {
                model: Some("openai/gpt-6.1-sol".into()),
                streaming: false
            }
        );
        assert_eq!(
            parse(
                r#"{"id":"req-3","type":"response","command":"set_model","success":false,"error":"no such model"}"#
            ),
            AgentEvent::Response {
                id: Some("req-3".into()),
                command: "set_model".into(),
                success: false,
                error: Some("no such model".into())
            }
        );
        assert_eq!(
            parse(
                r#"{"type":"response","command":"parse","success":false,"error":"Failed to parse command"}"#
            ),
            AgentEvent::Response {
                id: None,
                command: "parse".into(),
                success: false,
                error: Some("Failed to parse command".into())
            }
        );
    }

    #[test]
    fn classifies_extension_dialogs_and_notifications() {
        assert_eq!(
            parse(
                r#"{"type":"extension_ui_request","id":"uuid-2","method":"confirm","title":"Post entry?","message":"R2"}"#
            ),
            AgentEvent::UiRequest {
                id: "uuid-2".into(),
                method: "confirm".into(),
                title: "Post entry?".into(),
                dialog: true
            }
        );
        assert_eq!(
            parse(
                r#"{"type":"extension_ui_request","id":"uuid-9","method":"notify","message":"Synced"}"#
            ),
            AgentEvent::UiRequest {
                id: "uuid-9".into(),
                method: "notify".into(),
                title: "Synced".into(),
                dialog: false
            }
        );
        assert_eq!(
            parse(
                r#"{"type":"extension_error","extensionPath":"x","event":"tool_call","error":"boom"}"#
            ),
            AgentEvent::ExtensionError("boom".into())
        );
    }

    #[test]
    fn rejects_lines_that_are_not_protocol_records() {
        assert!(matches!(
            parse_record("not json"),
            Err(AgentProtocolError::NotJson(_))
        ));
        assert_eq!(
            parse_record(r#"{"id":"1"}"#),
            Err(AgentProtocolError::MissingType)
        );
        assert_eq!(
            parse_record(r#"{"type":7}"#),
            Err(AgentProtocolError::MissingType)
        );
    }

    #[test]
    fn command_builders_escape_operator_text_and_set_streaming_behavior() {
        let prompt = prompt_command("p-1", "say \"hi\"\nnow", false);
        let value: Value = serde_json::from_str(&prompt).expect("json");
        assert_eq!(value["type"], "prompt");
        assert_eq!(value["id"], "p-1");
        assert_eq!(value["message"], "say \"hi\"\nnow");
        assert!(value.get("streamingBehavior").is_none());
        assert!(!prompt.contains('\n'), "one command per line");

        let follow_up: Value =
            serde_json::from_str(&prompt_command("p-2", "and", true)).expect("json");
        assert_eq!(follow_up["streamingBehavior"], "followUp");

        assert_eq!(
            get_state_command("s-1"),
            r#"{"id":"s-1","type":"get_state"}"#
        );
        assert_eq!(
            stop_commands(),
            [
                r#"{"type":"clear_queue"}"#.to_owned(),
                r#"{"type":"abort"}"#.to_owned()
            ]
        );
        let cancel: Value = serde_json::from_str(&cancel_dialog_response("uuid-2")).expect("json");
        assert_eq!(cancel["type"], "extension_ui_response");
        assert_eq!(cancel["id"], "uuid-2");
        assert_eq!(cancel["cancelled"], true);
    }

    fn fake_agent(script: &str) -> AgentSession {
        let mut config = AgentSessionConfig::new("sh", vec!["-c".into(), script.into()]);
        config.shutdown_grace = Duration::from_millis(500);
        AgentSession::spawn(&config).expect("spawn sh")
    }

    fn next(session: &AgentSession) -> AgentUpdate {
        session
            .recv_timeout(Duration::from_secs(5))
            .expect("update within 5s")
    }

    #[test]
    fn session_streams_a_run_and_cancels_extension_dialogs() {
        // A stand-in agent: answers get_state, streams one run for the prompt, asks a confirm
        // dialog and reports whether the client cancelled it.
        let script = r#"
read state
echo '{"id":"s-1","type":"response","command":"get_state","success":true,"data":{"model":{"id":"m","provider":"p"},"isStreaming":false}}'
read prompt
case "$prompt" in *'"type":"prompt"'*) ;; *) echo 'bad prompt'; exit 1;; esac
case "$prompt" in *'"message":"hola"'*) ;; *) echo 'bad prompt'; exit 1;; esac
echo '{"type":"agent_start"}'
echo '{"type":"message_update","assistantMessageEvent":{"type":"text_delta","delta":"Ho"}}'
echo '{"type":"message_update","assistantMessageEvent":{"type":"text_delta","delta":"la"}}'
echo '{"type":"extension_ui_request","id":"u-1","method":"confirm","title":"Post?"}'
read answer
case "$answer" in *'"cancelled":true'*'"id":"u-1"'*|*'"id":"u-1"'*'"cancelled":true'*) echo '{"type":"response","id":"ui","command":"check","success":true}';; *) echo '{"type":"response","id":"ui","command":"check","success":false}';; esac
echo '{"type":"message_end","message":{"role":"assistant","content":[{"type":"text","text":"Hola"}],"stopReason":"stop"}}'
echo '{"type":"agent_settled","aborted":false}'
"#;
        let session = fake_agent(script);
        session.send(&get_state_command("s-1")).expect("send");
        assert_eq!(
            next(&session),
            AgentUpdate::Event(AgentEvent::State {
                model: Some("p/m".into()),
                streaming: false
            })
        );
        session
            .send(&prompt_command("p-1", "hola", false))
            .expect("send");
        let expected = [
            AgentEvent::RunStarted,
            AgentEvent::TextDelta("Ho".into()),
            AgentEvent::TextDelta("la".into()),
            AgentEvent::UiRequest {
                id: "u-1".into(),
                method: "confirm".into(),
                title: "Post?".into(),
                dialog: true,
            },
            AgentEvent::Response {
                id: Some("ui".into()),
                command: "check".into(),
                success: true,
                error: None,
            },
            AgentEvent::AssistantMessage {
                text: "Hola".into(),
                stop_reason: "stop".into(),
                error: None,
            },
            AgentEvent::Settled { aborted: false },
        ];
        for event in expected {
            assert_eq!(next(&session), AgentUpdate::Event(event));
        }
        assert_eq!(next(&session), AgentUpdate::Exited);
    }

    #[test]
    fn session_stops_reading_after_a_protocol_violation() {
        let session = fake_agent("echo '{\"type\":\"agent_start\"}'; echo 'warning: plain text'; echo '{\"type\":\"agent_start\"}'; sleep 5");
        assert_eq!(next(&session), AgentUpdate::Event(AgentEvent::RunStarted));
        assert!(matches!(next(&session), AgentUpdate::Failed(msg) if msg.contains("not JSON")));
        assert_eq!(session.recv_timeout(Duration::from_millis(300)), None);
    }

    #[test]
    fn send_can_interrupt_a_streaming_run_and_drop_reaps_the_agent() {
        // Streams until it reads an abort, like the real agent does.
        let script = r#"
echo '{"type":"agent_start"}'
while read line; do
  case "$line" in *'"type":"abort"'*) echo '{"type":"agent_settled","aborted":true}';; esac
done
"#;
        let session = fake_agent(script);
        assert_eq!(next(&session), AgentUpdate::Event(AgentEvent::RunStarted));
        for frame in stop_commands() {
            session.send(&frame).expect("send while streaming");
        }
        assert_eq!(
            next(&session),
            AgentUpdate::Event(AgentEvent::Settled { aborted: true })
        );
        let started = Instant::now();
        drop(session);
        assert!(
            started.elapsed() < Duration::from_secs(2),
            "stdin close ends the agent"
        );
    }

    /// Contract check against the real agent: `cargo test -p drenyra-rpc -- --ignored`.
    /// Uses only `get_state`, which never calls a model.
    #[test]
    #[ignore = "needs the `pi` agent on PATH"]
    fn real_pi_answers_get_state_in_the_agent_protocol() {
        let session = AgentSession::spawn(&AgentSessionConfig::new(
            "pi",
            vec!["--mode".into(), "rpc".into(), "--no-session".into()],
        ))
        .expect("spawn pi");
        session.send(&get_state_command("s-1")).expect("send");
        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            match session.recv_timeout(left) {
                Some(AgentUpdate::Event(AgentEvent::State { .. })) => break,
                Some(AgentUpdate::Event(_)) => continue,
                other => panic!(
                    "expected a get_state reply, got {other:?}; stderr: {}",
                    session.stderr_snapshot()
                ),
            }
        }
    }

    #[test]
    fn spawn_rejects_an_empty_program_and_reports_missing_binaries() {
        assert!(matches!(
            AgentSession::spawn(&AgentSessionConfig::new("  ", vec![])),
            Err(RpcError::EmptyCommand)
        ));
        assert!(matches!(
            AgentSession::spawn(&AgentSessionConfig::new(
                "drenyra-no-such-agent-binary",
                vec![]
            )),
            Err(RpcError::Io(_))
        ));
    }

    // Regression tests adopted from the independent verifier (T3a).

    #[test]
    fn drop_kills_an_agent_that_ignores_stdin_eof_after_the_grace() {
        let session = fake_agent("trap '' TERM; exec sleep 30");
        let started = Instant::now();
        drop(session);
        let took = started.elapsed();
        assert!(
            took >= Duration::from_millis(450),
            "waited the grace: {took:?}"
        );
        assert!(
            took < Duration::from_secs(2),
            "killed after grace: {took:?}"
        );
    }

    #[test]
    fn drop_finishes_while_a_dialog_answer_write_is_blocked() {
        // The agent never reads stdin: the caller fills the pipe, then a dialog makes the reader
        // thread block in write while holding the writer lock. Drop must still finish.
        let session = fake_agent("sleep 0.3; echo '{\"type\":\"extension_ui_request\",\"id\":\"d\",\"method\":\"confirm\",\"title\":\"t\"}'; trap '' TERM; exec sleep 30");
        session.send(&"x".repeat(65_530)).expect("fits in the pipe");
        thread::sleep(Duration::from_millis(800));
        let (tx, rx) = mpsc::channel();
        thread::spawn(move || {
            let started = Instant::now();
            drop(session);
            let _ = tx.send(started.elapsed());
        });
        assert!(
            rx.recv_timeout(Duration::from_secs(5)).is_ok(),
            "Drop hung while the dialog answer write was blocked"
        );
    }

    #[test]
    fn concurrent_sends_while_the_reader_answers_many_dialogs_do_not_deadlock() {
        let script = r#"
i=0
while [ $i -lt 200 ]; do echo "{\"type\":\"extension_ui_request\",\"id\":\"d-$i\",\"method\":\"select\",\"title\":\"t\"}"; i=$((i+1)); done
n=0
while read line; do n=$((n+1)); if [ $n -ge 400 ]; then echo "{\"type\":\"agent_settled\",\"aborted\":false}"; fi; done
"#;
        let session = fake_agent(script);
        for i in 0..200 {
            session
                .send(&get_state_command(&format!("s-{i}")))
                .expect("send");
        }
        let deadline = Instant::now() + Duration::from_secs(10);
        let mut dialogs = 0;
        loop {
            assert!(
                Instant::now() < deadline,
                "deadlock: only {dialogs} dialogs"
            );
            match session.recv_timeout(Duration::from_secs(5)) {
                Some(AgentUpdate::Event(AgentEvent::UiRequest { .. })) => dialogs += 1,
                Some(AgentUpdate::Event(AgentEvent::Settled { .. })) => break,
                other => panic!("unexpected {other:?}"),
            }
        }
        assert_eq!(dialogs, 200);
    }

    #[test]
    fn crlf_and_blank_lines_are_tolerated_and_id_less_dialogs_get_no_answer() {
        // The first stdin line the agent reads must be the answer to dialog "d", which proves
        // the id-less dialog before it was not answered.
        let script = r#"
printf '{"type":"agent_start"}\r\n'
printf '\n\r\n   \n'
printf '{"type":"extension_ui_request","method":"confirm","title":"no id"}\n'
printf '{"type":"extension_ui_request","id":"d","method":"confirm","title":"with id"}\n'
read answer
case "$answer" in *'"id":"d"'*) echo '{"type":"response","command":"first-answer-is-d","success":true}';; *) echo '{"type":"response","command":"unexpected-answer","success":false}';; esac
"#;
        let session = fake_agent(script);
        assert_eq!(next(&session), AgentUpdate::Event(AgentEvent::RunStarted));
        assert!(matches!(
            next(&session),
            AgentUpdate::Event(AgentEvent::UiRequest { id, .. }) if id.is_empty()
        ));
        assert!(matches!(
            next(&session),
            AgentUpdate::Event(AgentEvent::UiRequest { id, .. }) if id == "d"
        ));
        assert!(matches!(
            next(&session),
            AgentUpdate::Event(AgentEvent::Response { command, success: true, .. })
                if command == "first-answer-is-d"
        ));
        assert_eq!(next(&session), AgentUpdate::Exited);
    }

    #[test]
    fn oversized_and_non_utf8_records_stop_the_session() {
        let mut config = AgentSessionConfig::new(
            "sh",
            vec![
                "-c".into(),
                "printf '{\"type\":\"agent_start\"}\\n'; head -c 2000 /dev/zero | tr '\\0' a; printf '\\n{\"type\":\"agent_start\"}\\n'; sleep 3".into(),
            ],
        );
        config.max_record_bytes = 1000;
        config.shutdown_grace = Duration::from_millis(200);
        let session = AgentSession::spawn(&config).expect("spawn");
        assert_eq!(next(&session), AgentUpdate::Event(AgentEvent::RunStarted));
        let update = next(&session);
        assert!(
            matches!(&update, AgentUpdate::Failed(m) if m.contains("1000")),
            "{update:?}"
        );
        assert_eq!(session.recv_timeout(Duration::from_millis(300)), None);
        assert!(session.drain().is_empty());

        let bad = fake_agent("printf '{\"type\":\"agent_\\377\"}\\n'; sleep 3");
        assert!(matches!(next(&bad), AgentUpdate::Failed(_)));
    }

    #[test]
    fn exit_mid_run_ends_updates_and_later_sends_error() {
        let session = fake_agent("echo '{\"type\":\"agent_start\"}'; echo '{\"type\":\"message_update\",\"assistantMessageEvent\":{\"type\":\"text_delta\",\"delta\":\"x\"}}'; exit 3");
        assert_eq!(next(&session), AgentUpdate::Event(AgentEvent::RunStarted));
        assert_eq!(
            next(&session),
            AgentUpdate::Event(AgentEvent::TextDelta("x".into()))
        );
        assert_eq!(next(&session), AgentUpdate::Exited);
        thread::sleep(Duration::from_millis(100));
        assert!(session.drain().is_empty());
        assert_eq!(session.recv_timeout(Duration::from_millis(50)), None);
        let mut err = None;
        for _ in 0..50 {
            if let Err(e) = session.send(&get_state_command("x")) {
                err = Some(e);
                break;
            }
            thread::sleep(Duration::from_millis(10));
        }
        assert!(err.is_some(), "send to an exited agent eventually errors");
    }

    #[test]
    fn long_records_under_the_cap_parse() {
        let big = "a".repeat(200_000);
        let session = fake_agent("printf '{\"type\":\"message_update\",\"assistantMessageEvent\":{\"type\":\"text_delta\",\"delta\":\"'; head -c 200000 /dev/zero | tr '\\0' a; printf '\"}}\\n'");
        assert_eq!(
            next(&session),
            AgentUpdate::Event(AgentEvent::TextDelta(big))
        );
    }

    #[test]
    fn unicode_line_separators_are_not_record_boundaries() {
        let session = fake_agent("printf '{\"type\":\"message_update\",\"assistantMessageEvent\":{\"type\":\"text_delta\",\"delta\":\"a\\342\\200\\250b\"}}\\n'");
        assert_eq!(
            next(&session),
            AgentUpdate::Event(AgentEvent::TextDelta("a\u{2028}b".into()))
        );
    }
}
