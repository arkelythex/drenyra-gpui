//! GPUI-free operator commands applied to the shell state.

use drenyra_workspace::PaneKind;

use super::demo;
use super::settings::Settings;
use super::theme::ThemeId;
use super::view_model::{Rect, SettingsPanel, ShellViewModel, ThemeOption};
use crate::controller::DrenyraHostController;

/// Comment recorded when a line is rejected from the keyboard (no free-text input yet).
pub const REJECT_COMMENT: &str = "Rejected by operator in Entry Diff Review";

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
    /// Apply the option under the cursor in the open overlay.
    Confirm,
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
}

impl ShellState {
    pub fn new(controller: DrenyraHostController, demo: bool) -> Self {
        Self {
            controller,
            review_cursor: 0,
            demo,
            settings: Settings::default(),
            settings_cursor: None,
        }
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
        match command {
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
    use crate::shell::settings::Settings;
    use crate::shell::theme::ThemeId;
    use crate::shell::view_model::{Rect, RowDecision};
    use drenyra_workspace::{PaneKind, SplitAxis, WorkspaceActivity};

    const WIDE: Rect = Rect {
        x: 0,
        y: 0,
        width: 1200,
        height: 800,
    };

    fn state() -> ShellState {
        ShellState::demo()
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
    }
}
