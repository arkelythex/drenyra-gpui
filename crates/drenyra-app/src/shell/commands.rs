//! GPUI-free operator commands applied to the shell state.

use drenyra_workspace::PaneKind;

use super::demo;
use super::view_model::{Rect, ShellViewModel};
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
}

/// Window-independent shell state: the host controller plus presentation cursors.
#[derive(Debug, Clone)]
pub struct ShellState {
    controller: DrenyraHostController,
    review_cursor: usize,
    demo: bool,
}

impl ShellState {
    pub fn new(controller: DrenyraHostController, demo: bool) -> Self {
        Self {
            controller,
            review_cursor: 0,
            demo,
        }
    }

    /// Deterministic demo state: two fiscal workspaces and one staged R2 proposal.
    pub fn demo() -> Self {
        Self::new(demo::demo_controller(), true)
    }

    pub fn controller(&self) -> &DrenyraHostController {
        &self.controller
    }

    pub fn view(&self, pane_area: Rect) -> ShellViewModel {
        ShellViewModel::build(&self.controller, pane_area, self.review_cursor, self.demo)
    }

    /// Applies `command`; returns `true` when the state changed and the window should re-render.
    pub fn apply(&mut self, command: ShellCommand, pane_area: Rect) -> bool {
        match command {
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
}
