//! GPUI-free operator commands applied to the shell state.

use drenyra_workspace::{PaneKind, WorkspaceKey};

use std::collections::BTreeMap;

use super::agent_pane::AgentPane;
use super::demo;
use super::settings::Settings;
use super::theme::ThemeId;
use super::view_model::{
    risk_tier_label, AgentPaneView, ApprovalDialogView, PaletteEntry, PaletteView, Rect,
    SettingsPanel, ShellViewModel, SidebarCompany, SidebarEntry, SidebarView, ThemeOption,
};
use crate::controller::DrenyraHostController;
use drenyra_ui::{EntryDiffReview, LineDecision};

/// Comment recorded when a line is rejected from the keyboard (no free-text input yet).
pub const REJECT_COMMENT: &str = "Rejected by operator in Entry Diff Review";

/// Shown when the approval dialog opens before every line is accepted.
pub const APPROVAL_NEEDS_ACCEPTED_LINES: &str = "Accept every line before approving.";
/// Shown when the entry does not balance.
pub const APPROVAL_NOT_BALANCED: &str = "The entry is not balanced, so it cannot be approved.";
/// Shown when Enter is pressed without an approver ID.
pub const APPROVAL_NEEDS_APPROVER: &str = "Enter the approver's ID: 1 to 128 visible characters.";

/// The approval dialog's input state.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct ApprovalDraft {
    approver: String,
    error: Option<&'static str>,
}

/// Sidebar status of a staged entry with pending lines.
pub const ENTRY_IN_REVIEW: &str = "In review";
/// Sidebar status of a staged entry with a rejected line.
pub const ENTRY_CHANGES_REQUESTED: &str = "Changes requested";
/// Sidebar status of a staged entry whose lines are all accepted but which still needs approval.
pub const ENTRY_AWAITING_APPROVAL: &str = "Awaiting approval";
/// Sidebar status of a staged entry that can be posted.
pub const ENTRY_READY_TO_POST: &str = "Ready to post";
/// Sidebar status of a staged entry that does not balance.
pub const ENTRY_NOT_BALANCED: &str = "Not balanced";

/// Sidebar status of a staged review.
fn entry_status(review: &EntryDiffReview) -> &'static str {
    let decisions = review.decisions();
    if !review.proposal().is_balanced() {
        ENTRY_NOT_BALANCED
    } else if review.can_post() {
        ENTRY_READY_TO_POST
    } else if decisions
        .iter()
        .any(|d| matches!(d, LineDecision::Rejected { .. }))
    {
        ENTRY_CHANGES_REQUESTED
    } else if decisions
        .iter()
        .all(|d| matches!(d, LineDecision::Accepted))
    {
        ENTRY_AWAITING_APPROVAL
    } else {
        ENTRY_IN_REVIEW
    }
}

/// What a command palette entry does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PaletteAction {
    Run(ShellCommand),
    SwitchWorkspace(usize),
    ApplyTheme(ThemeId),
}

/// The command palette's input state.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct PaletteDraft {
    query: String,
    cursor: usize,
}

/// Palette label of a command, or `None` for commands that only make sense inside an overlay.
const fn command_label(command: ShellCommand) -> Option<&'static str> {
    Some(match command {
        ShellCommand::SplitFocused => "Split focused pane",
        ShellCommand::CloseFocused => "Close focused pane",
        ShellCommand::UndoClose => "Reopen closed pane",
        ShellCommand::FocusNextPane => "Focus next pane",
        ShellCommand::NextWorkspace => "Next workspace",
        ShellCommand::ReviewCursorDown => "Next review line",
        ShellCommand::ReviewCursorUp => "Previous review line",
        ShellCommand::AcceptLine => "Accept review line",
        ShellCommand::RejectLine => "Reject review line",
        ShellCommand::OpenSettings => "Open settings",
        ShellCommand::FocusComposer => "Ask the Fiscal Agent",
        ShellCommand::StopAgent => "Stop the running agent",
        ShellCommand::OpenApproval => "Record human approval…",
        ShellCommand::ToggleSidebar => "Toggle sidebar",
        ShellCommand::CloseOverlay | ShellCommand::Confirm | ShellCommand::OpenPalette => {
            return None
        }
    })
}

/// Human-readable form of the first key bound to `command` (`ctrl-shift-t` → `Ctrl+Shift+T`).
pub fn key_hint(command: ShellCommand) -> String {
    let Some((key, _)) = KEY_BINDINGS.iter().find(|(_, bound)| *bound == command) else {
        return String::new();
    };
    key.split('-')
        .map(|part| match part {
            "ctrl" => "Ctrl".to_owned(),
            "shift" => "Shift".to_owned(),
            "alt" => "Alt".to_owned(),
            "escape" => "Esc".to_owned(),
            other => {
                let mut chars = other.chars();
                chars.next().map_or_else(String::new, |first| {
                    first.to_uppercase().chain(chars).collect()
                })
            }
        })
        .collect::<Vec<_>>()
        .join("+")
}

/// Text a key press types into a text field, or `None` when it is a shortcut or has no
/// character. AltGr chords (reported as Alt, or Ctrl+Alt on some layouts) still type, so `@`
/// and `€` work on Latin American and European keyboards; Ctrl-only and Super chords never do.
pub fn typed_text(
    control: bool,
    alt: bool,
    platform: bool,
    key_char: Option<&str>,
) -> Option<&str> {
    if platform || (control && !alt) {
        return None;
    }
    key_char.filter(|text| !text.is_empty())
}

/// Case-insensitive subsequence match: every query character appears in `label`, in order.
fn fuzzy_matches(label: &str, query: &str) -> bool {
    let mut haystack = label.chars().flat_map(char::to_lowercase);
    query
        .chars()
        .flat_map(char::to_lowercase)
        .filter(|c| !c.is_whitespace())
        .all(|needle| haystack.any(|c| c == needle))
}

/// Operator intents the native window maps from key bindings.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShellCommand {
    /// Split the focused pane along its real longest side (dwindle).
    SplitFocused,
    CloseFocused,
    UndoClose,
    FocusNextPane,
    NextWorkspace,
    ReviewCursorDown,
    ReviewCursorUp,
    AcceptLine,
    RejectLine,
    /// Open the Settings overlay.
    OpenSettings,
    /// Close the open overlay.
    CloseOverlay,
    /// Apply the option under the cursor in the open overlay, or send the composer text.
    Confirm,
    /// Start typing a prompt in the focused Fiscal Agent pane.
    FocusComposer,
    /// Stop the active workspace's running agent.
    StopAgent,
    /// Open the human-approval dialog for the staged entry.
    OpenApproval,
    /// Open the command palette.
    OpenPalette,
    /// Show or hide the sidebar.
    ToggleSidebar,
}

/// Every command, for exhaustiveness checks.
pub const ALL_COMMANDS: &[ShellCommand] = &[
    ShellCommand::SplitFocused,
    ShellCommand::CloseFocused,
    ShellCommand::UndoClose,
    ShellCommand::FocusNextPane,
    ShellCommand::NextWorkspace,
    ShellCommand::ReviewCursorDown,
    ShellCommand::ReviewCursorUp,
    ShellCommand::AcceptLine,
    ShellCommand::RejectLine,
    ShellCommand::OpenSettings,
    ShellCommand::CloseOverlay,
    ShellCommand::Confirm,
    ShellCommand::FocusComposer,
    ShellCommand::StopAgent,
    ShellCommand::OpenApproval,
    ShellCommand::OpenPalette,
    ShellCommand::ToggleSidebar,
];

/// Single source of truth for window key bindings (GPUI keystroke syntax).
pub const KEY_BINDINGS: &[(&str, ShellCommand)] = &[
    ("ctrl-d", ShellCommand::SplitFocused),
    ("ctrl-w", ShellCommand::CloseFocused),
    ("ctrl-shift-t", ShellCommand::UndoClose),
    ("ctrl-tab", ShellCommand::FocusNextPane),
    ("ctrl-k", ShellCommand::NextWorkspace),
    ("j", ShellCommand::ReviewCursorDown),
    ("down", ShellCommand::ReviewCursorDown),
    ("k", ShellCommand::ReviewCursorUp),
    ("up", ShellCommand::ReviewCursorUp),
    ("a", ShellCommand::AcceptLine),
    ("r", ShellCommand::RejectLine),
    ("ctrl-,", ShellCommand::OpenSettings),
    ("escape", ShellCommand::CloseOverlay),
    ("enter", ShellCommand::Confirm),
    ("i", ShellCommand::FocusComposer),
    ("ctrl-c", ShellCommand::StopAgent),
    ("ctrl-p", ShellCommand::OpenApproval),
    ("ctrl-shift-p", ShellCommand::OpenPalette),
    ("ctrl-b", ShellCommand::ToggleSidebar),
];

/// Window-independent shell state: the host controller plus presentation cursors.
#[derive(Debug, Clone)]
pub struct ShellState {
    controller: DrenyraHostController,
    review_cursor: usize,
    demo: bool,
    settings: Settings,
    /// Cursor into [`ThemeId::ALL`] while the Settings overlay is open; `None` when closed.
    settings_cursor: Option<usize>,
    /// One Fiscal Agent pane per company and period, so conversations never cross companies.
    agents: BTreeMap<WorkspaceKey, AgentPane>,
    /// True while keystrokes go to the active workspace's composer.
    composer_active: bool,
    /// The approval dialog, when open.
    approval: Option<ApprovalDraft>,
    /// The command palette, when open.
    palette: Option<PaletteDraft>,
    sidebar_open: bool,
}

impl ShellState {
    pub fn new(controller: DrenyraHostController, demo: bool) -> Self {
        Self {
            controller,
            review_cursor: 0,
            demo,
            settings: Settings::default(),
            settings_cursor: None,
            agents: BTreeMap::new(),
            composer_active: false,
            approval: None,
            palette: None,
            sidebar_open: true,
        }
    }

    /// True while the sidebar is shown (it takes width from the panes).
    pub fn sidebar_open(&self) -> bool {
        self.sidebar_open
    }

    /// Makes workspace `index` active (sidebar click). Leaves the composer, which belongs to the
    /// previous company, and resets the review cursor. Returns `false` when nothing changed.
    pub fn switch_workspace(&mut self, index: usize) -> bool {
        let manager = self.controller.workspaces_mut();
        if index == manager.active_index() || index >= manager.workspaces().len() {
            return false;
        }
        self.composer_active = false;
        self.review_cursor = 0;
        manager.switch_to(index)
    }

    /// The Fiscal Agent pane of the workspace `key` (empty until the window starts its agent).
    pub fn agent_pane(&self, key: &WorkspaceKey) -> AgentPane {
        self.agents.get(key).cloned().unwrap_or_default()
    }

    pub fn agent_pane_mut(&mut self, key: &WorkspaceKey) -> &mut AgentPane {
        self.agents.entry(key.clone()).or_default()
    }

    /// Identity of the active workspace, which owns the visible agent pane.
    pub fn active_workspace_key(&self) -> WorkspaceKey {
        self.controller.workspaces().active_key()
    }

    /// Index of the active workspace in the switcher.
    pub fn active_workspace(&self) -> usize {
        self.controller.workspaces().active_index()
    }

    /// True while keystrokes go to a text field (the composer or the approval dialog) instead of
    /// key bindings.
    pub fn text_input_active(&self) -> bool {
        self.approval.is_some() || self.palette.is_some() || self.composer_active
    }

    /// Types into the active text field; returns `false` when no text field is active.
    pub fn type_text(&mut self, text: &str) -> bool {
        if text.is_empty() {
            return false;
        }
        if let Some(draft) = &mut self.approval {
            draft
                .approver
                .extend(text.chars().filter(|c| !c.is_control()));
            draft.error = None;
            return true;
        }
        if let Some(palette) = &mut self.palette {
            palette
                .query
                .extend(text.chars().filter(|c| !c.is_control()));
            palette.cursor = 0;
            return true;
        }
        if !self.composer_active {
            return false;
        }
        let key = self.active_workspace_key();
        self.agent_pane_mut(&key).composer_insert(text);
        true
    }

    /// Deletes the last character of the active text field; returns `false` when nothing changed.
    pub fn backspace(&mut self) -> bool {
        if let Some(draft) = &mut self.approval {
            return draft.approver.pop().is_some();
        }
        if let Some(palette) = &mut self.palette {
            palette.cursor = 0;
            return palette.query.pop().is_some();
        }
        if !self.composer_active {
            return false;
        }
        let key = self.active_workspace_key();
        self.agent_pane_mut(&key).composer_backspace()
    }

    /// Every palette entry with its action, in display order.
    fn palette_items(&self) -> Vec<(String, String, PaletteAction)> {
        let commands = ALL_COMMANDS.iter().filter_map(|&command| {
            command_label(command).map(|label| {
                (
                    label.to_owned(),
                    key_hint(command),
                    PaletteAction::Run(command),
                )
            })
        });
        let workspaces = self
            .controller
            .workspaces()
            .workspaces()
            .iter()
            .enumerate()
            .map(|(index, ws)| {
                (
                    format!(
                        "Switch to {} · {} · {}",
                        ws.legal_name, ws.entity_ruc, ws.fiscal_period
                    ),
                    String::new(),
                    PaletteAction::SwitchWorkspace(index),
                )
            });
        let themes = ThemeId::ALL.iter().map(|&theme| {
            (
                format!("Theme: {}", theme.label()),
                String::new(),
                PaletteAction::ApplyTheme(theme),
            )
        });
        commands.chain(workspaces).chain(themes).collect()
    }

    /// Palette entries matching the current query.
    fn palette_matches(&self) -> Vec<(String, String, PaletteAction)> {
        let query = self
            .palette
            .as_ref()
            .map(|p| p.query.as_str())
            .unwrap_or_default();
        self.palette_items()
            .into_iter()
            .filter(|(label, _, _)| fuzzy_matches(label, query))
            .collect()
    }

    /// Why the staged entry cannot be approved now, or `None` when it can.
    fn approval_blocker(&self) -> Option<&'static str> {
        let review = self.controller.active_review()?;
        if !review.proposal().is_balanced() {
            return Some(APPROVAL_NOT_BALANCED);
        }
        let all_accepted = review
            .decisions()
            .iter()
            .all(|decision| matches!(decision, LineDecision::Accepted));
        (!all_accepted).then_some(APPROVAL_NEEDS_ACCEPTED_LINES)
    }

    /// Replaces the operator settings (for example, the ones loaded from disk).
    pub fn with_settings(mut self, settings: Settings) -> Self {
        self.settings = settings;
        self
    }

    pub fn settings(&self) -> Settings {
        self.settings
    }

    /// Applies `theme`; returns `true` when it changed.
    pub fn select_theme(&mut self, theme: ThemeId) -> bool {
        if self.settings.theme == theme {
            return false;
        }
        self.settings.theme = theme;
        true
    }

    /// Deterministic demo state: two fiscal workspaces and one staged R2 proposal.
    pub fn demo() -> Self {
        Self::new(demo::demo_controller(), true)
    }

    pub fn controller(&self) -> &DrenyraHostController {
        &self.controller
    }

    pub fn view(&self, pane_area: Rect) -> ShellViewModel {
        let mut view =
            ShellViewModel::build(&self.controller, pane_area, self.review_cursor, self.demo);
        view.theme = self.settings.theme;
        let pane = self.agent_pane(&self.active_workspace_key());
        view.agent = Some(AgentPaneView::from_pane(&pane, self.composer_active));
        view.sidebar = self.sidebar_open.then(|| SidebarView {
            companies: self
                .controller
                .workspaces()
                .workspaces()
                .iter()
                .enumerate()
                .map(|(index, ws)| SidebarCompany {
                    index,
                    legal_name: ws.legal_name.clone(),
                    entity_ruc: ws.entity_ruc.clone(),
                    fiscal_period: ws.fiscal_period.clone(),
                    activity: ws.activity,
                    active: index == self.active_workspace(),
                    unposted_entries: ws.guardrails.unposted_entries,
                    staged: self.controller.review(index).map(|review| SidebarEntry {
                        proposal_id: review.proposal().id.clone(),
                        risk_tier_label: risk_tier_label(review.proposal().risk_tier),
                        status: entry_status(review),
                    }),
                })
                .collect(),
        });
        view.palette = self.palette.as_ref().map(|draft| PaletteView {
            query: draft.query.clone(),
            entries: self
                .palette_matches()
                .into_iter()
                .enumerate()
                .map(|(index, (label, keys, _))| PaletteEntry {
                    label,
                    keys,
                    selected: index == draft.cursor,
                })
                .collect(),
        });
        view.approval = match (
            &self.approval,
            &view.review,
            self.controller.active_review(),
        ) {
            (Some(draft), Some(panel), Some(review)) => {
                let blocker = self.approval_blocker();
                Some(ApprovalDialogView {
                    proposal_id: panel.proposal_id.clone(),
                    risk_tier_label: panel.risk_tier_label,
                    total_debit: panel.total_debit.clone(),
                    total_credit: panel.total_credit.clone(),
                    required: review.proposal().risk_tier.requires_human_approval(),
                    approver: draft.approver.clone(),
                    ready: blocker.is_none(),
                    blocker,
                    error: draft.error,
                })
            }
            _ => None,
        };
        view.settings = self.settings_cursor.map(|cursor| SettingsPanel {
            sections: vec!["Themes"],
            active_section: 0,
            themes: ThemeId::ALL
                .iter()
                .enumerate()
                .map(|(index, &id)| ThemeOption {
                    id,
                    label: id.label(),
                    applied: id == self.settings.theme,
                    cursor: index == cursor,
                })
                .collect(),
        });
        view
    }

    /// Applies `command`; returns `true` when the state changed and the window should re-render.
    ///
    /// While the Settings overlay is open it is modal: J/K move its cursor, Enter applies,
    /// Escape closes, and workspace or review commands do nothing.
    pub fn apply(&mut self, command: ShellCommand, pane_area: Rect) -> bool {
        if let Some(cursor) = self.settings_cursor {
            return self.apply_in_settings(command, cursor);
        }
        if self.approval.is_some() {
            return self.apply_in_approval(command);
        }
        if self.palette.is_some() {
            return self.apply_in_palette(command, pane_area);
        }
        if self.composer_active {
            return self.apply_in_composer(command);
        }
        match command {
            ShellCommand::OpenPalette => {
                self.palette = Some(PaletteDraft::default());
                true
            }
            ShellCommand::ToggleSidebar => {
                self.sidebar_open = !self.sidebar_open;
                true
            }
            ShellCommand::OpenApproval => {
                if self.controller.active_review().is_none() {
                    return false;
                }
                self.approval = Some(ApprovalDraft::default());
                true
            }
            ShellCommand::FocusComposer => {
                let focused_is_agent = self
                    .view(pane_area)
                    .panes
                    .iter()
                    .any(|p| p.focused && p.kind == PaneKind::FiscalAgent);
                self.composer_active = focused_is_agent;
                focused_is_agent
            }
            ShellCommand::StopAgent => {
                let key = self.active_workspace_key();
                self.agent_pane_mut(&key).stop()
            }
            ShellCommand::OpenSettings => {
                let current = ThemeId::ALL
                    .iter()
                    .position(|&id| id == self.settings.theme)
                    .unwrap_or(0);
                self.settings_cursor = Some(current);
                true
            }
            ShellCommand::CloseOverlay | ShellCommand::Confirm => false,
            ShellCommand::SplitFocused => self.split_focused(pane_area),
            ShellCommand::CloseFocused => self
                .controller
                .workspaces_mut()
                .active_mut()
                .close_focused()
                .is_some(),
            ShellCommand::UndoClose => {
                let rect = self.focused_rect(pane_area);
                self.controller
                    .workspaces_mut()
                    .active_mut()
                    .undo_close(rect.width, rect.height)
                    .is_some()
            }
            ShellCommand::FocusNextPane => self.focus_next_pane(pane_area),
            ShellCommand::NextWorkspace => self.next_workspace(),
            ShellCommand::ReviewCursorDown => self.move_cursor_down(),
            ShellCommand::ReviewCursorUp => self.move_cursor_up(),
            ShellCommand::AcceptLine => {
                let cursor = self.review_cursor;
                self.controller
                    .active_review_mut()
                    .is_some_and(|review| review.accept_line(cursor))
            }
            ShellCommand::RejectLine => {
                let cursor = self.review_cursor;
                self.controller
                    .active_review_mut()
                    .is_some_and(|review| review.reject_line(cursor, REJECT_COMMENT))
            }
        }
    }

    /// The palette is modal: J/K or arrows move, Enter runs the selected entry, Escape closes.
    fn apply_in_palette(&mut self, command: ShellCommand, pane_area: Rect) -> bool {
        let matches = self.palette_matches();
        let Some(draft) = &mut self.palette else {
            return false;
        };
        match command {
            ShellCommand::ReviewCursorDown if draft.cursor + 1 < matches.len() => {
                draft.cursor += 1;
                true
            }
            ShellCommand::ReviewCursorUp if draft.cursor > 0 => {
                draft.cursor -= 1;
                true
            }
            ShellCommand::CloseOverlay => {
                self.palette = None;
                true
            }
            ShellCommand::Confirm => {
                let Some(&(_, _, action)) = matches.get(draft.cursor) else {
                    return false;
                };
                self.palette = None;
                match action {
                    PaletteAction::Run(command) => {
                        self.apply(command, pane_area);
                    }
                    PaletteAction::SwitchWorkspace(index) => {
                        self.switch_workspace(index);
                    }
                    PaletteAction::ApplyTheme(theme) => {
                        self.select_theme(theme);
                    }
                }
                true
            }
            _ => false,
        }
    }

    /// The approval dialog is modal: Enter records the approval, Escape closes.
    fn apply_in_approval(&mut self, command: ShellCommand) -> bool {
        match command {
            ShellCommand::CloseOverlay => {
                self.approval = None;
                true
            }
            ShellCommand::Confirm => {
                if let Some(blocker) = self.approval_blocker() {
                    self.set_approval_error(blocker);
                    return true;
                }
                let approver = self
                    .approval
                    .as_ref()
                    .map(|draft| draft.approver.clone())
                    .unwrap_or_default();
                let recorded = self
                    .controller
                    .active_review_mut()
                    .is_some_and(|review| review.record_human_approval(approver));
                if recorded {
                    self.approval = None;
                } else {
                    self.set_approval_error(APPROVAL_NEEDS_APPROVER);
                }
                true
            }
            _ => false,
        }
    }

    fn set_approval_error(&mut self, error: &'static str) {
        if let Some(draft) = &mut self.approval {
            draft.error = Some(error);
        }
    }

    fn apply_in_composer(&mut self, command: ShellCommand) -> bool {
        let key = self.active_workspace_key();
        match command {
            ShellCommand::Confirm => self.agent_pane_mut(&key).submit(),
            ShellCommand::StopAgent => self.agent_pane_mut(&key).stop(),
            ShellCommand::CloseOverlay => {
                self.composer_active = false;
                true
            }
            _ => false,
        }
    }

    fn apply_in_settings(&mut self, command: ShellCommand, cursor: usize) -> bool {
        match command {
            ShellCommand::ReviewCursorDown if cursor + 1 < ThemeId::ALL.len() => {
                self.settings_cursor = Some(cursor + 1);
                true
            }
            ShellCommand::ReviewCursorUp if cursor > 0 => {
                self.settings_cursor = Some(cursor - 1);
                true
            }
            ShellCommand::Confirm => self.select_theme(ThemeId::ALL[cursor]),
            ShellCommand::CloseOverlay => {
                self.settings_cursor = None;
                true
            }
            _ => false,
        }
    }

    fn focused_rect(&self, pane_area: Rect) -> Rect {
        self.view(pane_area).focused_rect().unwrap_or(pane_area)
    }

    fn split_focused(&mut self, pane_area: Rect) -> bool {
        let view = self.view(pane_area);
        let Some(focused) = view.panes.iter().find(|p| p.focused) else {
            return false;
        };
        let (rect, kind) = (focused.rect, next_pane_kind(focused.kind));
        let workspace = self.controller.workspaces_mut().active_mut();
        let before = workspace.pane_count();
        workspace.split_focused(kind, rect.width, rect.height);
        workspace.pane_count() > before
    }

    fn focus_next_pane(&mut self, pane_area: Rect) -> bool {
        let panes = self.view(pane_area).panes;
        if panes.len() < 2 {
            return false;
        }
        let current = panes.iter().position(|p| p.focused).unwrap_or(0);
        let next = panes[(current + 1) % panes.len()].id;
        self.controller
            .workspaces_mut()
            .active_mut()
            .focus_pane(next)
    }

    fn next_workspace(&mut self) -> bool {
        let manager = self.controller.workspaces_mut();
        let count = manager.workspaces().len();
        if count < 2 {
            return false;
        }
        let next = (manager.active_index() + 1) % count;
        self.review_cursor = 0;
        manager.switch_to(next)
    }

    fn review_line_count(&self) -> usize {
        self.controller
            .active_review()
            .map_or(0, |review| review.proposal().lines.len())
    }

    fn move_cursor_down(&mut self) -> bool {
        let lines = self.review_line_count();
        if self.review_cursor + 1 < lines {
            self.review_cursor += 1;
            true
        } else {
            false
        }
    }

    fn move_cursor_up(&mut self) -> bool {
        if self.review_line_count() > 0 && self.review_cursor > 0 {
            self.review_cursor -= 1;
            true
        } else {
            false
        }
    }
}

/// Pane kind opened by a split: cycles Fiscal Agent → Ledger → Evidence Vault → Fiscal CLI.
const fn next_pane_kind(kind: PaneKind) -> PaneKind {
    match kind {
        PaneKind::FiscalAgent => PaneKind::LedgerViewer,
        PaneKind::LedgerViewer => PaneKind::EvidenceVault,
        PaneKind::EvidenceVault => PaneKind::FiscalCli,
        PaneKind::FiscalCli => PaneKind::FiscalAgent,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shell::agent_pane::AgentStatus;
    use crate::shell::settings::Settings;
    use crate::shell::theme::ThemeId;
    use crate::shell::view_model::{Rect, RowDecision};
    use drenyra_workspace::{PaneKind, SplitAxis, WorkspaceActivity, WorkspaceKey};

    const WIDE: Rect = Rect {
        x: 0,
        y: 0,
        width: 1200,
        height: 800,
    };

    fn state() -> ShellState {
        ShellState::demo()
    }

    fn key(state: &ShellState, index: usize) -> WorkspaceKey {
        state.controller().workspaces().workspaces()[index].key()
    }

    #[test]
    fn agent_panes_belong_to_the_company_and_period_not_the_switcher_position() {
        let mut s = state();
        let andino = WorkspaceKey::new("20123456789", "2026-10");
        let sierra = WorkspaceKey::new("20987654321", "2026-11");
        assert_eq!(s.active_workspace_key(), andino);
        s.agent_pane_mut(&sierra).mark_unavailable("sierra only");

        assert!(s.agent_pane(&andino).transcript().is_empty());
        assert_eq!(s.agent_pane(&andino).status(), &AgentStatus::NotStarted);
        assert!(s.apply(ShellCommand::NextWorkspace, WIDE));
        assert_eq!(s.active_workspace_key(), sierra);
        assert_eq!(
            s.agent_pane(&sierra).status(),
            &AgentStatus::Unavailable("sierra only".into())
        );
    }

    fn pane_kinds(state: &ShellState) -> Vec<PaneKind> {
        state.view(WIDE).panes.iter().map(|p| p.kind).collect()
    }

    #[test]
    fn demo_state_has_two_workspaces_and_a_staged_r2_review() {
        let s = state();
        let vm = s.view(WIDE);
        assert!(vm.demo);
        assert_eq!(vm.workspaces.len(), 2);
        assert_eq!(vm.workspaces[0].legal_name, "Andino SAC");
        assert_eq!(vm.workspaces[1].legal_name, "Sierra SRL");
        let panel = vm.review.expect("staged demo review");
        assert_eq!(panel.risk_tier_label, "R2 Material");
        assert!(panel.balanced);
        assert!(!panel.can_post);
    }

    #[test]
    fn split_uses_real_focused_rect_and_cycles_pane_kind() {
        let mut s = state();
        assert!(s.apply(ShellCommand::SplitFocused, WIDE));
        assert_eq!(
            pane_kinds(&s),
            vec![PaneKind::FiscalAgent, PaneKind::LedgerViewer]
        );
        // Focused pane is now 600x800 (taller than wide), so the next split stacks vertically.
        assert!(s.apply(ShellCommand::SplitFocused, WIDE));
        assert_eq!(
            pane_kinds(&s),
            vec![
                PaneKind::FiscalAgent,
                PaneKind::LedgerViewer,
                PaneKind::EvidenceVault
            ]
        );
        let ws = s.controller().workspaces().active();
        match ws.layout() {
            drenyra_workspace::PaneNode::Split { axis, second, .. } => {
                assert!(matches!(axis, SplitAxis::Horizontal));
                assert!(matches!(
                    second.as_ref(),
                    drenyra_workspace::PaneNode::Split {
                        axis: SplitAxis::Vertical,
                        ..
                    }
                ));
            }
            other => panic!("expected split layout, got {other:?}"),
        }
    }

    #[test]
    fn close_and_undo_close_round_trip() {
        let mut s = state();
        assert!(
            !s.apply(ShellCommand::CloseFocused, WIDE),
            "last pane never closes"
        );
        s.apply(ShellCommand::SplitFocused, WIDE);
        assert!(s.apply(ShellCommand::CloseFocused, WIDE));
        assert_eq!(pane_kinds(&s), vec![PaneKind::FiscalAgent]);
        assert!(s.apply(ShellCommand::UndoClose, WIDE));
        assert_eq!(
            pane_kinds(&s),
            vec![PaneKind::FiscalAgent, PaneKind::LedgerViewer]
        );
        assert!(!s.apply(ShellCommand::UndoClose, WIDE), "history is empty");
    }

    #[test]
    fn focus_next_pane_wraps_in_layout_order() {
        let mut s = state();
        assert!(!s.apply(ShellCommand::FocusNextPane, WIDE), "single pane");
        s.apply(ShellCommand::SplitFocused, WIDE);
        let focused = |s: &ShellState| {
            s.view(WIDE)
                .panes
                .iter()
                .position(|p| p.focused)
                .expect("one focused pane")
        };
        assert_eq!(focused(&s), 1);
        assert!(s.apply(ShellCommand::FocusNextPane, WIDE));
        assert_eq!(focused(&s), 0);
        assert!(s.apply(ShellCommand::FocusNextPane, WIDE));
        assert_eq!(focused(&s), 1);
    }

    #[test]
    fn next_workspace_wraps_and_resets_review_cursor() {
        let mut s = state();
        s.apply(ShellCommand::ReviewCursorDown, WIDE);
        assert!(s.apply(ShellCommand::NextWorkspace, WIDE));
        let vm = s.view(WIDE);
        assert!(vm.workspaces[1].active);
        assert!(vm.review.is_none());
        assert_eq!(
            vm.guardrails.background_alert,
            WorkspaceActivity::AwaitingApproval
        );
        assert!(s.apply(ShellCommand::NextWorkspace, WIDE));
        let panel = s.view(WIDE).review.expect("back on Andino");
        assert!(panel.rows[0].cursor, "cursor reset on workspace switch");
    }

    #[test]
    fn review_cursor_moves_within_bounds() {
        let mut s = state();
        assert!(!s.apply(ShellCommand::ReviewCursorUp, WIDE));
        assert!(s.apply(ShellCommand::ReviewCursorDown, WIDE));
        assert!(s.apply(ShellCommand::ReviewCursorDown, WIDE));
        assert!(
            !s.apply(ShellCommand::ReviewCursorDown, WIDE),
            "stays on last line"
        );
        let panel = s.view(WIDE).review.expect("review");
        assert!(panel.rows[2].cursor);
        assert!(s.apply(ShellCommand::ReviewCursorUp, WIDE));
        assert!(s.view(WIDE).review.expect("review").rows[1].cursor);
    }

    #[test]
    fn accept_and_reject_apply_to_line_under_cursor() {
        let mut s = state();
        assert!(s.apply(ShellCommand::AcceptLine, WIDE));
        s.apply(ShellCommand::ReviewCursorDown, WIDE);
        assert!(s.apply(ShellCommand::RejectLine, WIDE));
        let panel = s.view(WIDE).review.expect("review");
        assert_eq!(panel.rows[0].decision, RowDecision::Accepted);
        assert_eq!(
            panel.rows[1].decision,
            RowDecision::Rejected {
                comment: REJECT_COMMENT.into()
            }
        );
        assert_eq!(panel.rows[2].decision, RowDecision::Pending);
    }

    #[test]
    fn review_commands_are_no_ops_without_a_staged_review() {
        let mut s = state();
        s.apply(ShellCommand::NextWorkspace, WIDE);
        for cmd in [
            ShellCommand::AcceptLine,
            ShellCommand::RejectLine,
            ShellCommand::ReviewCursorDown,
            ShellCommand::ReviewCursorUp,
        ] {
            assert!(!s.apply(cmd, WIDE), "{cmd:?}");
        }
    }

    #[test]
    fn settings_overlay_opens_moves_applies_theme_and_closes() {
        let mut s = state();
        assert_eq!(s.settings().theme, ThemeId::DreamcoderDark);
        assert!(s.view(WIDE).settings.is_none());

        assert!(s.apply(ShellCommand::OpenSettings, WIDE));
        assert!(!s.apply(ShellCommand::OpenSettings, WIDE), "already open");
        let panel = s.view(WIDE).settings.expect("settings open");
        assert_eq!(panel.sections, vec!["Themes"]);
        let labels: Vec<_> = panel.themes.iter().map(|t| t.label).collect();
        assert_eq!(labels, vec!["Dreamcoder Dark", "Dreamcoder Light"]);
        assert!(panel.themes[0].applied && panel.themes[0].cursor);

        // J/K move the theme cursor while the overlay is open, not the review cursor.
        assert!(s.apply(ShellCommand::ReviewCursorDown, WIDE));
        assert!(
            !s.apply(ShellCommand::ReviewCursorDown, WIDE),
            "last option"
        );
        assert!(s.view(WIDE).review.expect("review").rows[0].cursor);
        assert_eq!(
            s.view(WIDE).theme,
            ThemeId::DreamcoderDark,
            "moving is not applying"
        );

        assert!(s.apply(ShellCommand::Confirm, WIDE));
        assert_eq!(s.settings().theme, ThemeId::DreamcoderLight);
        assert_eq!(s.view(WIDE).theme, ThemeId::DreamcoderLight);
        assert!(!s.apply(ShellCommand::Confirm, WIDE), "same theme again");
        assert!(s.apply(ShellCommand::ReviewCursorUp, WIDE));

        assert!(s.apply(ShellCommand::CloseOverlay, WIDE));
        assert!(s.view(WIDE).settings.is_none());
        assert!(
            !s.apply(ShellCommand::CloseOverlay, WIDE),
            "nothing to close"
        );
        assert_eq!(
            s.settings().theme,
            ThemeId::DreamcoderLight,
            "kept after close"
        );
    }

    #[test]
    fn settings_overlay_is_modal_for_workspace_and_review_commands() {
        let mut s = state();
        s.apply(ShellCommand::OpenSettings, WIDE);
        let before = s.view(WIDE);
        for cmd in [
            ShellCommand::SplitFocused,
            ShellCommand::CloseFocused,
            ShellCommand::UndoClose,
            ShellCommand::FocusNextPane,
            ShellCommand::NextWorkspace,
            ShellCommand::AcceptLine,
            ShellCommand::RejectLine,
        ] {
            assert!(
                !s.apply(cmd, WIDE),
                "{cmd:?} must be blocked by the overlay"
            );
        }
        let after = s.view(WIDE);
        assert_eq!(after.panes, before.panes);
        assert_eq!(after.review, before.review);
        assert_eq!(after.workspaces, before.workspaces);
    }

    #[test]
    fn select_theme_applies_directly_and_confirm_is_a_no_op_outside_settings() {
        let mut s = ShellState::demo().with_settings(Settings {
            theme: ThemeId::DreamcoderLight,
        });
        assert_eq!(s.view(WIDE).theme, ThemeId::DreamcoderLight);
        assert!(!s.apply(ShellCommand::Confirm, WIDE));
        assert!(s.select_theme(ThemeId::DreamcoderDark));
        assert!(!s.select_theme(ThemeId::DreamcoderDark));
        assert_eq!(s.settings().theme, ThemeId::DreamcoderDark);

        // Opening the overlay puts the cursor on the applied theme.
        s.select_theme(ThemeId::DreamcoderLight);
        s.apply(ShellCommand::OpenSettings, WIDE);
        let panel = s.view(WIDE).settings.expect("open");
        assert!(panel.themes[1].cursor && panel.themes[1].applied);
    }

    #[test]
    fn composer_opens_only_on_a_focused_agent_pane_and_captures_text() {
        let mut s = state();
        assert!(!s.type_text("x"), "typing needs the composer");
        assert!(!s.text_input_active());
        assert!(s.apply(ShellCommand::FocusComposer, WIDE));
        assert!(s.text_input_active());
        assert!(s.view(WIDE).agent.expect("agent pane").composer_active);
        assert!(s.type_text("hola"));
        assert!(s.backspace());
        assert_eq!(s.view(WIDE).agent.expect("agent").composer, "hol");

        // Focus a non-agent pane: the composer cannot open there.
        assert!(s.apply(ShellCommand::CloseOverlay, WIDE));
        assert!(s.apply(ShellCommand::SplitFocused, WIDE));
        assert_eq!(pane_kinds(&s)[1], PaneKind::LedgerViewer);
        assert!(!s.apply(ShellCommand::FocusComposer, WIDE));
    }

    #[test]
    fn composer_mode_is_modal_and_enter_submits_to_the_active_workspace_only() {
        let mut s = state();
        s.agent_pane_mut(&key(&s, 0)).mark_started();
        s.agent_pane_mut(&key(&s, 0)).take_outbox();
        s.agent_pane_mut(&key(&s, 0))
            .apply(drenyra_rpc::agent::AgentUpdate::Event(
                drenyra_rpc::agent::AgentEvent::State {
                    model: None,
                    streaming: false,
                },
            ));
        s.apply(ShellCommand::FocusComposer, WIDE);
        s.type_text("¿saldo de caja?");
        for cmd in [
            ShellCommand::SplitFocused,
            ShellCommand::NextWorkspace,
            ShellCommand::AcceptLine,
            ShellCommand::OpenSettings,
        ] {
            assert!(
                !s.apply(cmd, WIDE),
                "{cmd:?} must be blocked while composing"
            );
        }
        assert!(s.apply(ShellCommand::Confirm, WIDE));
        assert_eq!(s.agent_pane_mut(&key(&s, 0)).take_outbox().len(), 1);
        assert!(
            s.agent_pane_mut(&key(&s, 1)).take_outbox().is_empty(),
            "other company untouched"
        );
        assert!(s.agent_pane(&key(&s, 1)).transcript().is_empty());

        assert!(s.apply(ShellCommand::CloseOverlay, WIDE));
        assert!(!s.view(WIDE).agent.expect("agent").composer_active);
        assert!(s.apply(ShellCommand::NextWorkspace, WIDE));
        assert!(
            s.view(WIDE).agent.expect("agent").transcript.is_empty(),
            "Sierra has its own pane"
        );
    }

    #[test]
    fn stop_agent_only_acts_on_a_running_agent() {
        let mut s = state();
        assert!(!s.apply(ShellCommand::StopAgent, WIDE));
        s.agent_pane_mut(&key(&s, 0)).mark_started();
        s.agent_pane_mut(&key(&s, 0)).take_outbox();
        s.agent_pane_mut(&key(&s, 0))
            .apply(drenyra_rpc::agent::AgentUpdate::Event(
                drenyra_rpc::agent::AgentEvent::RunStarted,
            ));
        assert!(s.apply(ShellCommand::StopAgent, WIDE));
        assert_eq!(s.agent_pane_mut(&key(&s, 0)).take_outbox().len(), 2);
    }

    #[test]
    fn approval_dialog_records_the_typed_approver_only_when_every_line_is_accepted() {
        let mut s = state();
        assert!(s.apply(ShellCommand::OpenApproval, WIDE));
        assert!(s.text_input_active());
        let dialog = s.view(WIDE).approval.expect("dialog open");
        assert_eq!(dialog.proposal_id, "je-demo-2026-10-001");
        assert_eq!(dialog.risk_tier_label, "R2 Material");
        assert_eq!(dialog.total_debit, "118.00");
        assert!(!dialog.ready);
        assert_eq!(dialog.blocker, Some(APPROVAL_NEEDS_ACCEPTED_LINES));

        // Not ready: Enter explains instead of approving, and other commands stay blocked.
        assert!(s.type_text("CPA-PERU-001"));
        assert!(s.apply(ShellCommand::Confirm, WIDE));
        assert!(s
            .controller()
            .active_review()
            .expect("review")
            .approved_by()
            .is_none());
        assert!(!s.apply(ShellCommand::AcceptLine, WIDE), "modal");
        assert!(s.apply(ShellCommand::CloseOverlay, WIDE));
        assert!(s.view(WIDE).approval.is_none());

        for _ in 0..3 {
            s.apply(ShellCommand::AcceptLine, WIDE);
            s.apply(ShellCommand::ReviewCursorDown, WIDE);
        }
        assert!(s.apply(ShellCommand::OpenApproval, WIDE));
        let dialog = s.view(WIDE).approval.expect("open");
        assert!(dialog.ready && dialog.blocker.is_none());
        assert_eq!(dialog.approver, "", "input starts empty each time");

        assert!(s.type_text("   "));
        assert!(s.apply(ShellCommand::Confirm, WIDE));
        assert_eq!(
            s.view(WIDE).approval.expect("still open").error,
            Some(APPROVAL_NEEDS_APPROVER)
        );

        assert!(s.backspace());
        s.type_text("  CPA-PERU-001 ");
        assert!(s.apply(ShellCommand::Confirm, WIDE));
        assert!(s.view(WIDE).approval.is_none(), "closes after approving");
        let review = s.controller().active_review().expect("review");
        assert_eq!(review.approved_by(), Some("CPA-PERU-001"));
        assert!(review.can_post());
        assert!(s.view(WIDE).review.expect("panel").can_post);

        // Editing a line afterwards clears the approval (guard from drenyra-ui).
        s.apply(ShellCommand::RejectLine, WIDE);
        assert!(s
            .controller()
            .active_review()
            .expect("review")
            .approved_by()
            .is_none());
    }

    #[test]
    fn approval_dialog_needs_a_staged_review_in_the_active_workspace() {
        let mut s = state();
        s.apply(ShellCommand::NextWorkspace, WIDE);
        assert!(
            !s.apply(ShellCommand::OpenApproval, WIDE),
            "Sierra has no staged review"
        );
        assert!(s.view(WIDE).approval.is_none());
        assert!(!s.type_text("x"));
    }

    #[test]
    fn palette_lists_commands_workspaces_and_themes_and_filters_by_typed_text() {
        let mut s = state();
        assert!(s.apply(ShellCommand::OpenPalette, WIDE));
        assert!(s.text_input_active());
        let palette = s.view(WIDE).palette.expect("open");
        assert_eq!(palette.query, "");
        let labels: Vec<_> = palette.entries.iter().map(|e| e.label.as_str()).collect();
        assert!(labels.contains(&"Split focused pane"));
        assert!(labels.contains(&"Switch to Sierra SRL · 20987654321 · 2026-11"));
        assert!(labels.contains(&"Theme: Dreamcoder Light"));
        assert!(
            !labels.iter().any(|l| l.contains("palette")),
            "no self entry"
        );
        assert!(palette.entries[0].selected);
        assert_eq!(
            palette
                .entries
                .iter()
                .find(|e| e.label == "Split focused pane")
                .map(|e| e.keys.as_str()),
            Some("Ctrl+D")
        );

        s.type_text("splt");
        let palette = s.view(WIDE).palette.expect("open");
        assert_eq!(palette.entries.len(), 1, "fuzzy subsequence match");
        assert_eq!(palette.entries[0].label, "Split focused pane");

        let before = s.view(WIDE).panes.len();
        assert!(s.apply(ShellCommand::Confirm, WIDE));
        assert!(s.view(WIDE).palette.is_none(), "closes after running");
        assert_eq!(s.view(WIDE).panes.len(), before + 1);
    }

    #[test]
    fn palette_switches_workspace_applies_theme_and_handles_no_matches() {
        let mut s = state();
        s.apply(ShellCommand::OpenPalette, WIDE);
        s.type_text("SIERRA");
        assert!(s.apply(ShellCommand::Confirm, WIDE));
        assert_eq!(s.active_workspace(), 1);

        s.apply(ShellCommand::OpenPalette, WIDE);
        s.type_text("theme");
        assert_eq!(s.view(WIDE).palette.expect("open").entries.len(), 2);
        assert!(s.apply(ShellCommand::ReviewCursorDown, WIDE));
        assert!(!s.apply(ShellCommand::ReviewCursorDown, WIDE), "last entry");
        assert!(s.apply(ShellCommand::Confirm, WIDE));
        assert_eq!(s.settings().theme, ThemeId::DreamcoderLight);

        s.apply(ShellCommand::OpenPalette, WIDE);
        s.type_text("zzzz");
        assert!(s.view(WIDE).palette.expect("open").entries.is_empty());
        assert!(!s.apply(ShellCommand::Confirm, WIDE), "nothing to run");
        assert!(s.backspace());
        assert!(s.apply(ShellCommand::CloseOverlay, WIDE));
        assert!(s.view(WIDE).palette.is_none());
    }

    #[test]
    fn palette_is_modal_and_reopens_with_an_empty_query() {
        let mut s = state();
        s.apply(ShellCommand::OpenPalette, WIDE);
        s.type_text("acc");
        for cmd in [
            ShellCommand::AcceptLine,
            ShellCommand::NextWorkspace,
            ShellCommand::OpenSettings,
            ShellCommand::OpenApproval,
        ] {
            assert!(
                !s.apply(cmd, WIDE),
                "{cmd:?} blocked while the palette is open"
            );
        }
        assert!(
            s.agent_pane(&key(&s, 0)).composer().is_empty(),
            "typing went to the query"
        );
        s.apply(ShellCommand::CloseOverlay, WIDE);
        s.apply(ShellCommand::OpenPalette, WIDE);
        assert_eq!(s.view(WIDE).palette.expect("open").query, "");
    }

    #[test]
    fn key_hints_and_fuzzy_matching_follow_their_rules() {
        assert_eq!(key_hint(ShellCommand::UndoClose), "Ctrl+Shift+T");
        assert_eq!(key_hint(ShellCommand::OpenSettings), "Ctrl+,");
        assert_eq!(key_hint(ShellCommand::ReviewCursorDown), "J");
        assert_eq!(key_hint(ShellCommand::CloseOverlay), "Esc");
        assert!(fuzzy_matches("Split focused pane", "SpLt"));
        assert!(fuzzy_matches("Split focused pane", " f p "));
        assert!(fuzzy_matches("Anything", ""));
        assert!(!fuzzy_matches("Split focused pane", "tps"), "order matters");
    }

    #[test]
    fn sidebar_lists_every_company_with_its_staged_entry_and_toggles() {
        let mut s = state();
        let sidebar = s.view(WIDE).sidebar.expect("open by default");
        assert_eq!(sidebar.companies.len(), 2);
        let andino = &sidebar.companies[0];
        assert_eq!(andino.legal_name, "Andino SAC");
        assert!(andino.active);
        assert_eq!(andino.unposted_entries, 1);
        let entry = andino.staged.as_ref().expect("staged entry");
        assert_eq!(entry.proposal_id, "je-demo-2026-10-001");
        assert_eq!(entry.risk_tier_label, "R2 Material");
        assert_eq!(entry.status, ENTRY_IN_REVIEW);
        let sierra = &sidebar.companies[1];
        assert!(!sierra.active);
        assert!(sierra.staged.is_none());
        assert_eq!(sierra.unposted_entries, 0);

        for _ in 0..3 {
            s.apply(ShellCommand::AcceptLine, WIDE);
            s.apply(ShellCommand::ReviewCursorDown, WIDE);
        }
        let status = |s: &ShellState| {
            s.view(WIDE).sidebar.expect("open").companies[0]
                .staged
                .as_ref()
                .expect("staged")
                .status
        };
        assert_eq!(status(&s), ENTRY_AWAITING_APPROVAL);
        s.apply(ShellCommand::RejectLine, WIDE);
        assert_eq!(status(&s), ENTRY_CHANGES_REQUESTED);

        assert!(s.sidebar_open());
        assert!(s.apply(ShellCommand::ToggleSidebar, WIDE));
        assert!(!s.sidebar_open());
        assert!(s.view(WIDE).sidebar.is_none());
        assert!(s.apply(ShellCommand::ToggleSidebar, WIDE));
        assert!(s.view(WIDE).sidebar.is_some());
    }

    #[test]
    fn switch_workspace_from_the_sidebar_resets_cursor_and_leaves_the_composer() {
        let mut s = state();
        s.apply(ShellCommand::ReviewCursorDown, WIDE);
        s.apply(ShellCommand::FocusComposer, WIDE);
        assert!(s.switch_workspace(1));
        assert_eq!(s.active_workspace(), 1);
        assert!(!s.text_input_active(), "the composer belonged to Andino");
        assert!(!s.switch_workspace(1), "already active");
        assert!(!s.switch_workspace(9), "no such workspace");
        assert!(s.switch_workspace(0));
        assert!(s.view(WIDE).review.expect("review").rows[0].cursor);
    }

    #[test]
    fn typed_text_keeps_altgr_characters_and_drops_shortcuts() {
        // Plain and Shift keys type.
        assert_eq!(typed_text(false, false, false, Some("a")), Some("a"));
        // AltGr (reported as Alt, or as Ctrl+Alt on some layouts) types `@`, `€`, `[`.
        assert_eq!(typed_text(false, true, false, Some("@")), Some("@"));
        assert_eq!(typed_text(true, true, false, Some("€")), Some("€"));
        // Ctrl and Super chords are shortcuts, never text.
        assert_eq!(typed_text(true, false, false, Some("c")), None);
        assert_eq!(typed_text(false, false, true, Some("v")), None);
        // Keys without a character (arrows, F-keys) type nothing.
        assert_eq!(typed_text(false, false, false, None), None);
        assert_eq!(typed_text(false, false, false, Some("")), None);
    }

    #[test]
    fn key_bindings_cover_every_command_without_duplicate_keys() {
        for command in ALL_COMMANDS {
            assert!(
                KEY_BINDINGS.iter().any(|(_, bound)| bound == command),
                "{command:?} has no key binding"
            );
        }
        let mut keys: Vec<&str> = KEY_BINDINGS.iter().map(|(key, _)| *key).collect();
        keys.sort_unstable();
        let before = keys.len();
        keys.dedup();
        assert_eq!(keys.len(), before, "duplicate key binding");
        assert!(KEY_BINDINGS.contains(&("ctrl-d", ShellCommand::SplitFocused)));
        assert!(KEY_BINDINGS.contains(&("a", ShellCommand::AcceptLine)));
        assert!(KEY_BINDINGS.contains(&("ctrl-,", ShellCommand::OpenSettings)));
        assert!(KEY_BINDINGS.contains(&("escape", ShellCommand::CloseOverlay)));
        assert!(KEY_BINDINGS.contains(&("enter", ShellCommand::Confirm)));
        assert!(KEY_BINDINGS.contains(&("i", ShellCommand::FocusComposer)));
        assert!(KEY_BINDINGS.contains(&("ctrl-c", ShellCommand::StopAgent)));
        assert!(KEY_BINDINGS.contains(&("ctrl-p", ShellCommand::OpenApproval)));
        assert!(KEY_BINDINGS.contains(&("ctrl-shift-p", ShellCommand::OpenPalette)));
        assert!(KEY_BINDINGS.contains(&("ctrl-b", ShellCommand::ToggleSidebar)));
    }

    // Regression tests adopted from the independent verifier (T4).

    fn accept_every_line(s: &mut ShellState) {
        for _ in 0..3 {
            s.apply(ShellCommand::AcceptLine, WIDE);
            s.apply(ShellCommand::ReviewCursorDown, WIDE);
        }
    }

    #[test]
    fn approval_lands_only_on_the_active_workspace_and_reaches_the_post_frame() {
        use crate::controller::{ControllerError, DrenyraHostController};
        use crate::shell::demo::{demo_proposal, demo_workspace};
        let mut c = DrenyraHostController::new(demo_workspace());
        c.workspaces_mut()
            .add_workspace(drenyra_workspace::FiscalWorkspace::new(
                "20987654321",
                "Sierra SRL",
                "2026-11",
            ))
            .expect("distinct workspace");
        c.stage_proposal(demo_proposal()).expect("stage Andino");
        c.workspaces_mut().switch_to(1);
        let mut sierra = demo_proposal();
        (sierra.id, sierra.entity_ruc, sierra.fiscal_period) = (
            "je-sierra-001".into(),
            "20987654321".into(),
            "2026-11".into(),
        );
        c.stage_proposal(sierra).expect("stage Sierra");
        c.workspaces_mut().switch_to(0);
        let mut s = ShellState::new(c, true);
        accept_every_line(&mut s);
        s.apply(ShellCommand::NextWorkspace, WIDE);
        accept_every_line(&mut s);
        s.apply(ShellCommand::NextWorkspace, WIDE);
        assert!(s.apply(ShellCommand::OpenApproval, WIDE));
        assert!(
            !s.apply(ShellCommand::NextWorkspace, WIDE),
            "switch blocked while modal"
        );
        s.type_text("  CPA-ANDINO ");
        assert!(s.apply(ShellCommand::Confirm, WIDE));
        let frame = s
            .controller()
            .clone()
            .build_post_request()
            .expect("Andino posts");
        assert!(frame.contains("\"approved_by\":\"CPA-ANDINO\""));
        s.apply(ShellCommand::NextWorkspace, WIDE);
        assert_eq!(
            s.controller()
                .active_review()
                .expect("Sierra review")
                .approved_by(),
            None,
            "Sierra unaffected"
        );
        assert_eq!(
            s.controller().clone().build_post_request(),
            Err(ControllerError::ReviewNotReadyToPost)
        );
        s.apply(ShellCommand::OpenApproval, WIDE);
        assert_eq!(s.view(WIDE).approval.expect("open").approver, "");
    }

    #[test]
    fn unbalanced_entry_cannot_be_approved_from_the_dialog() {
        let mut p = crate::shell::demo::demo_proposal();
        p.lines[2].credit = drenyra_protocol::MoneyCents::from_cents(1_700);
        let mut c =
            crate::controller::DrenyraHostController::new(crate::shell::demo::demo_workspace());
        c.stage_proposal(p).expect("stage");
        let mut s = ShellState::new(c, true);
        accept_every_line(&mut s);
        s.apply(ShellCommand::OpenApproval, WIDE);
        s.type_text("CPA-1");
        assert!(s.apply(ShellCommand::Confirm, WIDE));
        assert_eq!(
            s.controller()
                .active_review()
                .expect("review")
                .approved_by(),
            None
        );
        assert_eq!(
            s.view(WIDE).approval.expect("open").error,
            Some(APPROVAL_NOT_BALANCED)
        );
    }

    #[test]
    fn dialog_and_composer_text_stay_separate_and_controls_are_dropped() {
        let mut s = state();
        s.apply(ShellCommand::FocusComposer, WIDE);
        s.type_text("hola");
        assert!(!s.apply(ShellCommand::OpenApproval, WIDE));
        s.apply(ShellCommand::CloseOverlay, WIDE);
        s.apply(ShellCommand::OpenApproval, WIDE);
        s.type_text("CPA\n-0\t0\r1\u{7}");
        assert_eq!(s.view(WIDE).approval.expect("open").approver, "CPA-001");
        assert_eq!(s.view(WIDE).agent.expect("agent").composer, "hola");
        s.apply(ShellCommand::CloseOverlay, WIDE);
        s.apply(ShellCommand::OpenApproval, WIDE);
        assert_eq!(s.view(WIDE).approval.expect("open").approver, "");
    }

    #[test]
    fn invisible_approver_ids_are_refused_with_the_documented_limit() {
        let mut s = state();
        accept_every_line(&mut s);
        s.apply(ShellCommand::OpenApproval, WIDE);
        s.type_text("\u{200B}");
        assert!(s.apply(ShellCommand::Confirm, WIDE));
        assert_eq!(
            s.controller()
                .active_review()
                .expect("review")
                .approved_by(),
            None
        );
        assert_eq!(
            s.view(WIDE).approval.expect("open").error,
            Some(APPROVAL_NEEDS_APPROVER)
        );
        assert!(APPROVAL_NEEDS_APPROVER.contains(&drenyra_ui::MAX_APPROVER_ID_CHARS.to_string()));
    }
}
