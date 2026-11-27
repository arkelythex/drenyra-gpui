//! GPUI-free state of the Fiscal Agent pane: transcript, composer and agent status.
//!
//! The window owns the agent process ([`drenyra_rpc::agent::AgentSession`]); this module only
//! turns the agent's [`AgentUpdate`]s into what the pane shows and queues the command lines the
//! window must send. One `AgentPane` exists per fiscal workspace, so a conversation about one
//! company never shows up under another.

use std::path::PathBuf;

use drenyra_rpc::agent::{
    get_state_command, prompt_command, stop_commands, AgentEvent, AgentSessionConfig, AgentUpdate,
};

/// Shown in the pane when no agent binary can be found.
pub const NO_AGENT_MESSAGE: &str =
    "No agent found on PATH. Install pi to talk to the Fiscal Agent (the pane runs `pi --mode rpc`).";

/// How to start the agent: `pi --mode rpc`, where the `drenyra-shell` package plugs in.
/// `find` resolves a binary name on `PATH`.
pub fn agent_launch(
    find: impl Fn(&str) -> Option<PathBuf>,
) -> Result<AgentSessionConfig, &'static str> {
    let program = find("pi").ok_or(NO_AGENT_MESSAGE)?;
    Ok(AgentSessionConfig::new(
        program.to_string_lossy().into_owned(),
        vec!["--mode".to_owned(), "rpc".to_owned()],
    ))
}

/// Transcript notice written when the operator restarts a stopped agent.
pub const RESTART_NOTICE: &str = "Restarting the agent…";

/// Transcript entries kept per pane; older entries are dropped first.
pub const TRANSCRIPT_LIMIT: usize = 500;

/// Lifecycle of the pane's agent process.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum AgentStatus {
    /// No process yet; the window starts one when the pane is shown.
    #[default]
    NotStarted,
    /// Process started, waiting for its first `get_state` reply.
    Starting,
    /// Idle and ready for a prompt.
    Ready,
    /// A run is streaming.
    Running,
    /// The process stopped or broke the protocol; the reason is shown.
    Stopped(String),
    /// No agent could be started (for example, `pi` is not installed).
    Unavailable(String),
}

/// Who an entry in the transcript comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EntryRole {
    Operator,
    Agent,
    Tool,
    Notice,
    Error,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TranscriptEntry {
    pub role: EntryRole,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct AgentPane {
    status: AgentStatus,
    model: Option<String>,
    transcript: Vec<TranscriptEntry>,
    /// Assistant text of the run in progress, replaced by the final message at `message_end`.
    streaming_text: String,
    composer: String,
    outbox: Vec<String>,
    next_id: u64,
}

impl AgentPane {
    pub fn status(&self) -> &AgentStatus {
        &self.status
    }

    /// Model reported by the agent, as `provider/id`.
    pub fn model(&self) -> Option<&str> {
        self.model.as_deref()
    }

    pub fn transcript(&self) -> &[TranscriptEntry] {
        &self.transcript
    }

    pub fn streaming_text(&self) -> &str {
        &self.streaming_text
    }

    pub fn composer(&self) -> &str {
        &self.composer
    }

    /// True while a run is streaming.
    pub fn is_running(&self) -> bool {
        self.status == AgentStatus::Running
    }

    /// True when a prompt can be sent now (a process is up, idle or streaming).
    pub fn accepts_prompts(&self) -> bool {
        matches!(
            self.status,
            AgentStatus::Starting | AgentStatus::Ready | AgentStatus::Running
        )
    }

    /// Records that the window started the process and queues `get_state`.
    pub fn mark_started(&mut self) {
        self.status = AgentStatus::Starting;
        let id = self.next_command_id("s");
        self.outbox.push(get_state_command(&id));
    }

    /// True when the agent stopped or could not start, so the operator can restart it.
    pub fn can_restart(&self) -> bool {
        matches!(
            self.status,
            AgentStatus::Stopped(_) | AgentStatus::Unavailable(_)
        )
    }

    /// Asks the window to start a fresh agent process: back to `NotStarted`, keeping the
    /// transcript and the composer draft. Returns `false` unless [`Self::can_restart`].
    pub fn restart(&mut self) -> bool {
        if !self.can_restart() {
            return false;
        }
        self.outbox.clear();
        self.streaming_text.clear();
        self.model = None;
        self.push(EntryRole::Notice, RESTART_NOTICE.to_owned());
        self.status = AgentStatus::NotStarted;
        true
    }

    /// Records that no agent could be started.
    pub fn mark_unavailable(&mut self, reason: impl Into<String>) {
        let reason = reason.into();
        self.push(EntryRole::Error, reason.clone());
        self.status = AgentStatus::Unavailable(reason);
    }

    pub fn composer_insert(&mut self, text: &str) {
        self.composer
            .extend(text.chars().filter(|c| !c.is_control() || *c == '\n'));
    }

    /// Removes the last character; returns `false` when the composer was empty.
    pub fn composer_backspace(&mut self) -> bool {
        self.composer.pop().is_some()
    }

    /// Sends the composer text as a prompt. Returns `false` (and keeps the text) when it is blank
    /// or no agent is running; a prompt sent during a run is queued as a follow-up.
    pub fn submit(&mut self) -> bool {
        let message = self.composer.trim().to_owned();
        if message.is_empty() || !self.accepts_prompts() {
            return false;
        }
        let id = self.next_command_id("p");
        self.outbox
            .push(prompt_command(&id, &message, self.is_running()));
        self.push(EntryRole::Operator, message);
        self.composer.clear();
        true
    }

    /// Asks the agent to stop the current run; returns `false` when nothing is running.
    pub fn stop(&mut self) -> bool {
        if !self.is_running() {
            return false;
        }
        self.outbox.extend(stop_commands());
        self.push(EntryRole::Notice, "Stop requested.".to_owned());
        true
    }

    /// Command lines waiting to be written to the agent, oldest first.
    pub fn take_outbox(&mut self) -> Vec<String> {
        std::mem::take(&mut self.outbox)
    }

    /// Applies one update from the agent; returns `true` when the pane changed.
    pub fn apply(&mut self, update: AgentUpdate) -> bool {
        match update {
            AgentUpdate::Event(event) => self.apply_event(event),
            AgentUpdate::Failed(reason) => {
                self.flush_streaming();
                self.push(EntryRole::Error, format!("Agent session stopped: {reason}"));
                self.status = AgentStatus::Stopped(reason);
                true
            }
            AgentUpdate::Exited => {
                self.flush_streaming();
                self.status = AgentStatus::Stopped("the agent process exited".to_owned());
                self.push(EntryRole::Notice, "The agent process exited.".to_owned());
                true
            }
        }
    }

    fn apply_event(&mut self, event: AgentEvent) -> bool {
        match event {
            AgentEvent::RunStarted => {
                self.status = AgentStatus::Running;
                self.streaming_text.clear();
            }
            AgentEvent::TextDelta(delta) => self.streaming_text.push_str(&delta),
            AgentEvent::AssistantMessage {
                text,
                stop_reason,
                error,
            } => {
                self.streaming_text.clear();
                if !text.trim().is_empty() {
                    self.push(EntryRole::Agent, text);
                }
                match (stop_reason.as_str(), error) {
                    (_, Some(error)) => self.push(EntryRole::Error, format!("Agent error: {error}")),
                    ("aborted", None) => self.push(EntryRole::Notice, "Run stopped.".to_owned()),
                    _ => {}
                }
            }
            AgentEvent::ToolStarted { tool } => {
                self.push(EntryRole::Tool, format!("Running tool {tool}"));
            }
            AgentEvent::ToolFinished { tool, is_error } => {
                if is_error {
                    self.push(EntryRole::Error, format!("Tool {tool} failed"));
                }
            }
            AgentEvent::RetryScheduled {
                attempt,
                max_attempts,
                error,
            } => self.push(
                EntryRole::Notice,
                format!("Retrying ({attempt}/{max_attempts}) after: {error}"),
            ),
            AgentEvent::Settled { .. } => {
                self.flush_streaming();
                if self.status == AgentStatus::Running {
                    self.status = AgentStatus::Ready;
                }
            }
            AgentEvent::State { model, streaming } => {
                self.model = model;
                if matches!(self.status, AgentStatus::Starting | AgentStatus::Ready) {
                    self.status = if streaming {
                        AgentStatus::Running
                    } else {
                        AgentStatus::Ready
                    };
                }
            }
            AgentEvent::Response {
                success: false,
                command,
                error,
                ..
            } => self.push(
                EntryRole::Error,
                format!(
                    "The agent rejected `{command}`: {}",
                    error.unwrap_or_else(|| "no reason given".to_owned())
                ),
            ),
            AgentEvent::Response { .. } | AgentEvent::Other(_) => return false,
            AgentEvent::UiRequest {
                method,
                title,
                dialog: true,
                ..
            } => self.push(
                EntryRole::Notice,
                format!(
                    "The agent asked \"{title}\" ({method}). Declined: the window cannot answer agent dialogs yet."
                ),
            ),
            AgentEvent::UiRequest { title, .. } => {
                if title.trim().is_empty() {
                    return false;
                }
                self.push(EntryRole::Notice, title);
            }
            AgentEvent::ExtensionError(error) => {
                self.push(EntryRole::Error, format!("Extension error: {error}"));
            }
        }
        true
    }

    /// Keeps partial text when a run ends without its final message.
    fn flush_streaming(&mut self) {
        let partial = std::mem::take(&mut self.streaming_text);
        if !partial.trim().is_empty() {
            self.push(EntryRole::Agent, partial);
        }
    }

    fn push(&mut self, role: EntryRole, text: String) {
        self.transcript.push(TranscriptEntry { role, text });
        if self.transcript.len() > TRANSCRIPT_LIMIT {
            let excess = self.transcript.len() - TRANSCRIPT_LIMIT;
            self.transcript.drain(..excess);
        }
    }

    fn next_command_id(&mut self, prefix: &str) -> String {
        self.next_id += 1;
        format!("{prefix}-{}", self.next_id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;

    fn event(e: AgentEvent) -> AgentUpdate {
        AgentUpdate::Event(e)
    }

    fn ready_pane() -> AgentPane {
        let mut pane = AgentPane::default();
        pane.mark_started();
        pane.take_outbox();
        pane.apply(event(AgentEvent::State {
            model: Some("p/m".into()),
            streaming: false,
        }));
        pane
    }

    #[test]
    fn restart_is_offered_only_after_the_agent_stopped_or_was_unavailable() {
        let mut pane = ready_pane();
        assert!(!pane.can_restart());
        assert!(!pane.restart(), "a ready agent is not restarted");

        pane.composer_insert("borrador");
        pane.apply(AgentUpdate::Failed("broken pipe".into()));
        assert!(pane.can_restart());
        assert!(pane.restart());
        assert_eq!(pane.status(), &AgentStatus::NotStarted);
        assert!(
            pane.take_outbox().is_empty(),
            "the old session gets nothing"
        );
        assert_eq!(pane.composer(), "borrador", "the draft survives");
        let last = pane.transcript().last().expect("restart notice");
        assert_eq!(
            (last.role, last.text.as_str()),
            (EntryRole::Notice, RESTART_NOTICE)
        );
        assert!(
            pane.transcript()
                .iter()
                .any(|e| e.text == "Agent session stopped: broken pipe"),
            "history is kept"
        );

        let mut missing = AgentPane::default();
        missing.mark_unavailable(NO_AGENT_MESSAGE);
        assert!(missing.restart());
        assert!(!missing.restart(), "already waiting for a new start");
    }

    fn json(line: &str) -> Value {
        serde_json::from_str(line).expect("json")
    }

    #[test]
    fn starting_queues_get_state_and_the_reply_makes_the_pane_ready() {
        let mut pane = AgentPane::default();
        assert_eq!(pane.status(), &AgentStatus::NotStarted);
        assert!(!pane.accepts_prompts());
        pane.mark_started();
        assert_eq!(pane.status(), &AgentStatus::Starting);
        let outbox = pane.take_outbox();
        assert_eq!(outbox.len(), 1);
        assert_eq!(json(&outbox[0])["type"], "get_state");
        assert!(pane.take_outbox().is_empty(), "drained");

        pane.apply(event(AgentEvent::State {
            model: Some("p/m".into()),
            streaming: false,
        }));
        assert_eq!(pane.status(), &AgentStatus::Ready);
        assert_eq!(pane.model(), Some("p/m"));
    }

    #[test]
    fn submit_sends_trimmed_text_records_it_and_clears_the_composer() {
        let mut pane = ready_pane();
        pane.composer_insert("  ¿Cuál es el IGV?\u{7}  ");
        assert!(pane.submit());
        let outbox = pane.take_outbox();
        let command = json(&outbox[0]);
        assert_eq!(command["type"], "prompt");
        assert_eq!(command["message"], "¿Cuál es el IGV?");
        assert!(command.get("streamingBehavior").is_none());
        assert_eq!(pane.composer(), "");
        assert_eq!(
            pane.transcript().last(),
            Some(&TranscriptEntry {
                role: EntryRole::Operator,
                text: "¿Cuál es el IGV?".into()
            })
        );

        pane.composer_insert("   ");
        assert!(!pane.submit(), "blank prompt");
        assert!(pane.take_outbox().is_empty());
    }

    #[test]
    fn prompts_need_a_running_agent_and_queue_as_follow_ups_during_a_run() {
        let mut idle = AgentPane::default();
        idle.composer_insert("hola");
        assert!(!idle.submit(), "no process yet");
        assert_eq!(idle.composer(), "hola", "text is kept");

        let mut pane = ready_pane();
        pane.apply(event(AgentEvent::RunStarted));
        pane.composer_insert("y además");
        assert!(pane.submit());
        assert_eq!(
            json(&pane.take_outbox()[0])["streamingBehavior"],
            "followUp"
        );

        let mut gone = ready_pane();
        gone.apply(AgentUpdate::Exited);
        gone.composer_insert("hola");
        assert!(!gone.submit());
    }

    #[test]
    fn a_streamed_run_shows_deltas_then_the_final_message() {
        let mut pane = ready_pane();
        pane.apply(event(AgentEvent::RunStarted));
        assert!(pane.is_running());
        pane.apply(event(AgentEvent::TextDelta("Ho".into())));
        pane.apply(event(AgentEvent::TextDelta("la".into())));
        assert_eq!(pane.streaming_text(), "Hola");
        pane.apply(event(AgentEvent::ToolStarted {
            tool: "bash".into(),
        }));
        pane.apply(event(AgentEvent::ToolFinished {
            tool: "bash".into(),
            is_error: true,
        }));
        pane.apply(event(AgentEvent::AssistantMessage {
            text: "Hola, contador".into(),
            stop_reason: "stop".into(),
            error: None,
        }));
        assert_eq!(pane.streaming_text(), "");
        pane.apply(event(AgentEvent::Settled { aborted: false }));
        assert_eq!(pane.status(), &AgentStatus::Ready);
        let roles: Vec<_> = pane.transcript().iter().map(|e| e.role).collect();
        assert_eq!(
            roles,
            vec![EntryRole::Tool, EntryRole::Error, EntryRole::Agent]
        );
        assert_eq!(pane.transcript()[2].text, "Hola, contador");
    }

    #[test]
    fn errors_aborts_and_dialogs_are_reported_never_hidden() {
        let mut pane = ready_pane();
        pane.apply(event(AgentEvent::RunStarted));
        assert!(pane.stop());
        let stop: Vec<_> = pane
            .take_outbox()
            .iter()
            .map(|l| json(l)["type"].clone())
            .collect();
        assert_eq!(stop, vec!["clear_queue", "abort"]);
        pane.apply(event(AgentEvent::AssistantMessage {
            text: String::new(),
            stop_reason: "aborted".into(),
            error: None,
        }));
        pane.apply(event(AgentEvent::Settled { aborted: true }));
        assert!(!pane.stop(), "nothing running");

        pane.apply(event(AgentEvent::AssistantMessage {
            text: String::new(),
            stop_reason: "error".into(),
            error: Some("529 overloaded".into()),
        }));
        pane.apply(event(AgentEvent::UiRequest {
            id: "u".into(),
            method: "confirm".into(),
            title: "Post entry?".into(),
            dialog: true,
        }));
        pane.apply(event(AgentEvent::Response {
            id: None,
            command: "parse".into(),
            success: false,
            error: Some("bad".into()),
        }));
        let texts: Vec<_> = pane.transcript().iter().map(|e| e.text.as_str()).collect();
        assert_eq!(
            texts,
            vec![
                "Stop requested.",
                "Run stopped.",
                "Agent error: 529 overloaded",
                "The agent asked \"Post entry?\" (confirm). Declined: the window cannot answer agent dialogs yet.",
                "The agent rejected `parse`: bad",
            ]
        );
        assert!(!pane.apply(event(AgentEvent::Other("turn_start".into()))));
    }

    #[test]
    fn a_broken_or_exited_session_keeps_partial_text_and_blocks_prompts() {
        let mut pane = ready_pane();
        pane.apply(event(AgentEvent::RunStarted));
        pane.apply(event(AgentEvent::TextDelta("partial".into())));
        pane.apply(AgentUpdate::Failed(
            "agent wrote a line that is not JSON".into(),
        ));
        assert_eq!(
            pane.status(),
            &AgentStatus::Stopped("agent wrote a line that is not JSON".into())
        );
        assert_eq!(pane.transcript()[0].text, "partial");
        assert!(!pane.accepts_prompts());

        let mut none = AgentPane::default();
        none.mark_unavailable("No agent found: install pi");
        assert_eq!(
            none.status(),
            &AgentStatus::Unavailable("No agent found: install pi".into())
        );
        assert_eq!(none.transcript()[0].role, EntryRole::Error);
    }

    #[test]
    fn agent_launch_runs_pi_in_rpc_mode_or_explains_what_is_missing() {
        let config = agent_launch(|name| (name == "pi").then(|| PathBuf::from("/opt/bin/pi")))
            .expect("pi found");
        assert_eq!(config.program, "/opt/bin/pi");
        assert_eq!(config.args, vec!["--mode", "rpc"]);
        assert_eq!(agent_launch(|_| None), Err(NO_AGENT_MESSAGE));
    }

    #[test]
    fn transcript_is_capped_keeping_the_newest_entries() {
        let mut pane = ready_pane();
        for i in 0..TRANSCRIPT_LIMIT + 7 {
            pane.composer_insert(&format!("m{i}"));
            assert!(pane.submit());
        }
        assert_eq!(pane.transcript().len(), TRANSCRIPT_LIMIT);
        assert_eq!(pane.transcript()[0].text, "m7");
        let ids: Vec<_> = pane
            .take_outbox()
            .iter()
            .map(|l| json(l)["id"].clone())
            .collect();
        let mut unique = ids.clone();
        unique.dedup();
        assert_eq!(unique.len(), ids.len(), "command ids are unique");
    }
}
