//! GPUI-free view model: everything the native window renders, derived from the controller.

use drenyra_protocol::{MoneyCents, RiskTier};
use drenyra_ui::{EntryDiffReview, LineDecision};
use drenyra_workspace::{PaneKind, PaneNode, SplitAxis, WorkspaceActivity};

use super::agent_pane::{AgentPane, AgentStatus, TranscriptEntry};
use super::theme::ThemeId;
use crate::controller::DrenyraHostController;

/// Height of the workspace and guardrail chip bar, in pixels.
pub const TOP_BAR_HEIGHT: u32 = 44;
/// Height of the key-hint footer, in pixels.
pub const FOOTER_HEIGHT: u32 = 28;
/// Width of the Entry Diff Review panel when a review is staged, in pixels.
pub const REVIEW_PANEL_WIDTH: u32 = 440;

/// Region left for the dwindle panes in a `width` x `height` viewport (pane-area coordinates).
pub const fn pane_area(width: u32, height: u32, has_review: bool) -> Rect {
    pane_area_with(width, height, has_review, false)
}

/// Width of the sidebar, in pixels.
pub const SIDEBAR_WIDTH: u32 = 260;

/// Like [`pane_area`], also excluding the sidebar when it is open.
pub const fn pane_area_with(width: u32, height: u32, has_review: bool, has_sidebar: bool) -> Rect {
    let panel = if has_review { REVIEW_PANEL_WIDTH } else { 0 };
    let sidebar = if has_sidebar { SIDEBAR_WIDTH } else { 0 };
    Rect {
        x: 0,
        y: 0,
        width: width.saturating_sub(panel).saturating_sub(sidebar),
        height: height.saturating_sub(TOP_BAR_HEIGHT + FOOTER_HEIGHT),
    }
}

/// Integer-pixel rectangle in window coordinates.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rect {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

/// One tile of the dwindle layout.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PaneTile {
    pub id: u64,
    pub kind: PaneKind,
    pub title: &'static str,
    pub rect: Rect,
    pub focused: bool,
}

/// Workspace switcher chip.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspaceChip {
    pub index: usize,
    pub legal_name: String,
    pub entity_ruc: String,
    pub fiscal_period: String,
    pub activity: WorkspaceActivity,
    pub active: bool,
}

/// Session guardrail chips of the active workspace.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GuardrailChips {
    pub rda_enabled: bool,
    pub guardian_angel_active: bool,
    pub risk_tier_label: &'static str,
    pub unposted_entries: usize,
    /// Highest-priority activity among the non-active workspaces.
    pub background_alert: WorkspaceActivity,
}

/// Human decision on a review line, as displayed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RowDecision {
    Pending,
    Accepted,
    Rejected { comment: String },
}

/// One journal line of the Entry Diff Review panel; amounts are exact `MoneyCents` text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReviewRow {
    pub index: usize,
    pub account_code: String,
    pub description: String,
    /// Empty when the line has no debit.
    pub debit: String,
    /// Empty when the line has no credit.
    pub credit: String,
    pub decision: RowDecision,
    pub cursor: bool,
}

/// Entry Diff Review panel of the active workspace's staged proposal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReviewPanel {
    pub proposal_id: String,
    pub risk_tier_label: &'static str,
    pub rows: Vec<ReviewRow>,
    /// `"overflow"` when the sum does not fit in `MoneyCents`.
    pub total_debit: String,
    pub total_credit: String,
    pub balanced: bool,
    pub can_post: bool,
    pub approved_by: Option<String>,
}

/// The human-approval dialog for the active workspace's staged entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApprovalDialogView {
    pub proposal_id: String,
    pub risk_tier_label: &'static str,
    pub total_debit: String,
    pub total_credit: String,
    /// Whether this risk tier requires a recorded approval before posting.
    pub required: bool,
    /// Approver ID typed so far.
    pub approver: String,
    /// True when the entry can be approved now.
    pub ready: bool,
    /// Why the entry cannot be approved yet.
    pub blocker: Option<&'static str>,
    /// Feedback from the last Enter.
    pub error: Option<&'static str>,
}

/// A staged journal entry listed in the sidebar.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SidebarEntry {
    pub proposal_id: String,
    pub risk_tier_label: &'static str,
    pub status: &'static str,
}

/// One fiscal workspace in the sidebar.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SidebarCompany {
    pub index: usize,
    pub legal_name: String,
    pub entity_ruc: String,
    pub fiscal_period: String,
    pub activity: WorkspaceActivity,
    pub active: bool,
    pub unposted_entries: usize,
    pub staged: Option<SidebarEntry>,
}

/// The sidebar: every company and its unposted entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SidebarView {
    pub companies: Vec<SidebarCompany>,
}

/// One row of the command palette.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PaletteEntry {
    pub label: String,
    /// Key hint such as `Ctrl+D`; empty when the action has no shortcut.
    pub keys: String,
    pub selected: bool,
}

/// The command palette: typed query and the entries that match it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PaletteView {
    pub query: String,
    pub entries: Vec<PaletteEntry>,
}

/// Transcript entries rendered per frame (the pane keeps more; the oldest scroll out first).
pub const VISIBLE_TRANSCRIPT: usize = 200;

/// How the agent status chip is colored.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StatusTone {
    Muted,
    Busy,
    Ok,
    Warning,
    Error,
}

/// The Fiscal Agent pane of the active workspace.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentPaneView {
    pub status_label: String,
    pub status_tone: StatusTone,
    pub model: Option<String>,
    pub transcript: Vec<TranscriptEntry>,
    pub streaming_text: String,
    pub composer: String,
    pub composer_active: bool,
    pub can_send: bool,
    /// How to restart a stopped agent; `None` while it runs.
    pub restart_hint: Option<String>,
}

impl AgentPaneView {
    pub fn from_pane(pane: &AgentPane, composer_active: bool) -> Self {
        let (status_label, status_tone) = match pane.status() {
            AgentStatus::NotStarted => ("Not started".to_owned(), StatusTone::Muted),
            AgentStatus::Starting => ("Starting agent".to_owned(), StatusTone::Busy),
            AgentStatus::Ready => ("Ready".to_owned(), StatusTone::Ok),
            AgentStatus::Running => ("Working".to_owned(), StatusTone::Busy),
            AgentStatus::Stopped(reason) => (format!("Stopped: {reason}"), StatusTone::Warning),
            AgentStatus::Unavailable(reason) => {
                (format!("Unavailable: {reason}"), StatusTone::Error)
            }
        };
        let transcript = pane.transcript();
        let start = transcript.len().saturating_sub(VISIBLE_TRANSCRIPT);
        Self {
            status_label,
            status_tone,
            model: pane.model().map(str::to_owned),
            transcript: transcript[start..].to_vec(),
            streaming_text: pane.streaming_text().to_owned(),
            composer: pane.composer().to_owned(),
            composer_active,
            can_send: pane.accepts_prompts(),
            restart_hint: None,
        }
    }
}

/// One theme choice in Settings › Themes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ThemeOption {
    pub id: ThemeId,
    pub label: &'static str,
    /// The theme currently in use.
    pub applied: bool,
    /// The option under the keyboard cursor.
    pub cursor: bool,
}

/// The Settings overlay. `sections` lists the navigation entries; Themes is the only one today.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SettingsPanel {
    pub sections: Vec<&'static str>,
    pub active_section: usize,
    pub themes: Vec<ThemeOption>,
}

/// Everything the native window renders for one frame.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShellViewModel {
    pub workspaces: Vec<WorkspaceChip>,
    pub guardrails: GuardrailChips,
    pub panes: Vec<PaneTile>,
    pub review: Option<ReviewPanel>,
    /// True when the window runs on deterministic demo data instead of a live shell.
    pub demo: bool,
    /// Theme the window draws with.
    pub theme: ThemeId,
    /// The Settings overlay, when open.
    pub settings: Option<SettingsPanel>,
    /// The active workspace's Fiscal Agent pane.
    pub agent: Option<AgentPaneView>,
    /// The approval dialog, when open.
    pub approval: Option<ApprovalDialogView>,
    /// The command palette, when open.
    pub palette: Option<PaletteView>,
    /// The sidebar, when open.
    pub sidebar: Option<SidebarView>,
}

impl ShellViewModel {
    /// Derives the view model; `pane_area` is the region available to the dwindle layout and
    /// `review_cursor` is clamped to the last review line.
    pub fn build(
        controller: &DrenyraHostController,
        pane_area: Rect,
        review_cursor: usize,
        demo: bool,
    ) -> Self {
        let manager = controller.workspaces();
        let active_index = manager.active_index();
        let active = manager.active();

        let workspaces = manager
            .workspaces()
            .iter()
            .enumerate()
            .map(|(index, ws)| WorkspaceChip {
                index,
                legal_name: ws.legal_name.clone(),
                entity_ruc: ws.entity_ruc.clone(),
                fiscal_period: ws.fiscal_period.clone(),
                activity: ws.activity,
                active: index == active_index,
            })
            .collect();

        let guardrails = GuardrailChips {
            rda_enabled: active.guardrails.rda_enabled,
            guardian_angel_active: active.guardrails.guardian_angel_active,
            risk_tier_label: risk_tier_label(active.guardrails.active_risk_tier),
            unposted_entries: active.guardrails.unposted_entries,
            background_alert: manager.background_alert(),
        };

        let mut panes = Vec::with_capacity(active.pane_count());
        layout_panes(
            active.layout(),
            pane_area,
            active.focused_pane_id(),
            &mut panes,
        );

        Self {
            workspaces,
            guardrails,
            panes,
            review: controller
                .active_review()
                .map(|review| review_panel(review, review_cursor)),
            demo,
            theme: ThemeId::default(),
            settings: None,
            agent: None,
            approval: None,
            palette: None,
            sidebar: None,
        }
    }

    /// Rectangle of the focused pane, used by the dwindle split to pick its axis.
    pub fn focused_rect(&self) -> Option<Rect> {
        self.panes.iter().find(|p| p.focused).map(|p| p.rect)
    }
}

/// Display title of a pane kind.
pub const fn pane_title(kind: PaneKind) -> &'static str {
    match kind {
        PaneKind::FiscalAgent => "Fiscal Agent",
        PaneKind::LedgerViewer => "Ledger",
        PaneKind::EvidenceVault => "Evidence Vault",
        PaneKind::FiscalCli => "Fiscal CLI",
    }
}

/// Display label of a risk tier.
pub const fn risk_tier_label(tier: RiskTier) -> &'static str {
    match tier {
        RiskTier::R0Read => "R0 Read",
        RiskTier::R1Draft => "R1 Draft",
        RiskTier::R2Material => "R2 Material",
        RiskTier::R3Irreversible => "R3 Irreversible",
    }
}

/// Lays out the dwindle tree: `Horizontal` splits side by side, `Vertical` stacks; the first
/// child gets the floor half and the second the remainder, so tiles never gap or overlap.
fn layout_panes(node: &PaneNode, area: Rect, focused_id: u64, out: &mut Vec<PaneTile>) {
    match node {
        PaneNode::Leaf(leaf) => out.push(PaneTile {
            id: leaf.id,
            kind: leaf.kind,
            title: pane_title(leaf.kind),
            rect: area,
            focused: leaf.id == focused_id,
        }),
        PaneNode::Split {
            axis,
            first,
            second,
        } => {
            let (a, b) = match axis {
                SplitAxis::Horizontal => {
                    let left = area.width / 2;
                    (
                        Rect {
                            width: left,
                            ..area
                        },
                        Rect {
                            x: area.x + left,
                            width: area.width - left,
                            ..area
                        },
                    )
                }
                SplitAxis::Vertical => {
                    let top = area.height / 2;
                    (
                        Rect {
                            height: top,
                            ..area
                        },
                        Rect {
                            y: area.y + top,
                            height: area.height - top,
                            ..area
                        },
                    )
                }
            };
            layout_panes(first, a, focused_id, out);
            layout_panes(second, b, focused_id, out);
        }
    }
}

fn review_panel(review: &EntryDiffReview, cursor: usize) -> ReviewPanel {
    let proposal = review.proposal();
    let cursor = cursor.min(proposal.lines.len().saturating_sub(1));
    let rows = proposal
        .lines
        .iter()
        .zip(review.decisions())
        .enumerate()
        .map(|(index, (line, decision))| ReviewRow {
            index,
            account_code: line.account_code.clone(),
            description: line.description.clone(),
            debit: amount_text(line.debit),
            credit: amount_text(line.credit),
            decision: match decision {
                LineDecision::Pending => RowDecision::Pending,
                LineDecision::Accepted => RowDecision::Accepted,
                LineDecision::Rejected { comment } => RowDecision::Rejected {
                    comment: comment.clone(),
                },
            },
            cursor: index == cursor,
        })
        .collect();

    ReviewPanel {
        proposal_id: proposal.id.clone(),
        risk_tier_label: risk_tier_label(proposal.risk_tier),
        rows,
        total_debit: total_text(proposal.lines.iter().map(|l| l.debit)),
        total_credit: total_text(proposal.lines.iter().map(|l| l.credit)),
        balanced: proposal.is_balanced(),
        can_post: review.can_post(),
        approved_by: review.approved_by().map(str::to_owned),
    }
}

fn amount_text(amount: MoneyCents) -> String {
    if amount == MoneyCents::ZERO {
        String::new()
    } else {
        amount.format_decimal()
    }
}

fn total_text(mut amounts: impl Iterator<Item = MoneyCents>) -> String {
    amounts
        .try_fold(MoneyCents::ZERO, MoneyCents::checked_add)
        .map_or_else(|| "overflow".to_owned(), MoneyCents::format_decimal)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::controller::DrenyraHostController;
    use drenyra_protocol::{JournalEntryProposal, JournalLine, MoneyCents, RiskTier};
    use drenyra_workspace::{FiscalWorkspace, PaneKind, WorkspaceActivity};

    const AREA: Rect = Rect {
        x: 0,
        y: 0,
        width: 1200,
        height: 800,
    };

    fn proposal() -> JournalEntryProposal {
        JournalEntryProposal {
            id: "je-1".into(),
            entity_ruc: "20123456789".into(),
            fiscal_period: "2026-10".into(),
            risk_tier: RiskTier::R2Material,
            lines: vec![
                JournalLine {
                    account_code: "1212".into(),
                    debit: MoneyCents::from_cents(11_800),
                    credit: MoneyCents::ZERO,
                    description: "Accounts receivable".into(),
                },
                JournalLine {
                    account_code: "7011".into(),
                    debit: MoneyCents::ZERO,
                    credit: MoneyCents::from_cents(10_000),
                    description: "Sales revenue".into(),
                },
                JournalLine {
                    account_code: "40111".into(),
                    debit: MoneyCents::ZERO,
                    credit: MoneyCents::from_cents(1_800),
                    description: "IGV 18%".into(),
                },
            ],
        }
    }

    fn controller() -> DrenyraHostController {
        let mut controller = DrenyraHostController::new(FiscalWorkspace::new(
            "20123456789",
            "Andino SAC",
            "2026-10",
        ));
        controller
            .workspaces_mut()
            .add_workspace(FiscalWorkspace::new("20987654321", "Sierra SRL", "2026-11"))
            .expect("distinct workspace");
        controller
    }

    #[test]
    fn single_pane_fills_the_area_and_is_focused() {
        let vm = ShellViewModel::build(&controller(), AREA, 0, true);
        assert_eq!(vm.panes.len(), 1);
        assert_eq!(vm.panes[0].rect, AREA);
        assert!(vm.panes[0].focused);
        assert_eq!(vm.panes[0].title, "Fiscal Agent");
        assert!(vm.demo);
    }

    #[test]
    fn horizontal_split_places_panes_side_by_side_without_gaps() {
        let mut c = controller();
        c.workspaces_mut()
            .active_mut()
            .split_focused(PaneKind::LedgerViewer, 1201, 800);
        let area = Rect {
            x: 10,
            y: 20,
            width: 1201,
            height: 800,
        };
        let vm = ShellViewModel::build(&c, area, 0, false);
        let rects: Vec<Rect> = vm.panes.iter().map(|p| p.rect).collect();
        assert_eq!(
            rects,
            vec![
                Rect {
                    x: 10,
                    y: 20,
                    width: 600,
                    height: 800
                },
                Rect {
                    x: 610,
                    y: 20,
                    width: 601,
                    height: 800
                },
            ]
        );
        assert!(!vm.panes[0].focused);
        assert!(vm.panes[1].focused);
        assert_eq!(vm.panes[1].title, "Ledger");
    }

    #[test]
    fn vertical_split_stacks_panes() {
        let mut c = controller();
        c.workspaces_mut()
            .active_mut()
            .split_focused(PaneKind::EvidenceVault, 400, 900);
        let vm = ShellViewModel::build(&c, AREA, 0, false);
        let rects: Vec<Rect> = vm.panes.iter().map(|p| p.rect).collect();
        assert_eq!(
            rects,
            vec![
                Rect {
                    x: 0,
                    y: 0,
                    width: 1200,
                    height: 400
                },
                Rect {
                    x: 0,
                    y: 400,
                    width: 1200,
                    height: 400
                },
            ]
        );
        assert_eq!(vm.panes[1].title, "Evidence Vault");
    }

    #[test]
    fn workspace_chips_mark_active_and_activity() {
        let mut c = controller();
        c.stage_proposal(proposal()).expect("scope matches");
        let vm = ShellViewModel::build(&c, AREA, 0, false);
        assert_eq!(vm.workspaces.len(), 2);
        assert_eq!(
            vm.workspaces[0],
            WorkspaceChip {
                index: 0,
                legal_name: "Andino SAC".into(),
                entity_ruc: "20123456789".into(),
                fiscal_period: "2026-10".into(),
                activity: WorkspaceActivity::AwaitingApproval,
                active: true,
            }
        );
        assert!(!vm.workspaces[1].active);
        assert_eq!(vm.workspaces[1].activity, WorkspaceActivity::Idle);
    }

    #[test]
    fn guardrail_chips_reflect_active_workspace_and_background_alert() {
        let mut c = controller();
        c.stage_proposal(proposal()).expect("scope matches");
        let vm = ShellViewModel::build(&c, AREA, 0, false);
        assert_eq!(vm.guardrails.unposted_entries, 1);
        assert!(vm.guardrails.rda_enabled);
        assert!(vm.guardrails.guardian_angel_active);
        // The staged R2 proposal raises the session approval gate.
        assert_eq!(vm.guardrails.risk_tier_label, "R2 Material");
        assert_eq!(vm.guardrails.background_alert, WorkspaceActivity::Idle);

        assert!(c.workspaces_mut().switch_to(1));
        let vm = ShellViewModel::build(&c, AREA, 0, false);
        assert_eq!(vm.guardrails.unposted_entries, 0);
        assert_eq!(vm.guardrails.risk_tier_label, "R0 Read");
        assert_eq!(
            vm.guardrails.background_alert,
            WorkspaceActivity::AwaitingApproval
        );
        assert!(vm.review.is_none());
    }

    #[test]
    fn review_panel_formats_money_decisions_cursor_and_totals() {
        let mut c = controller();
        c.stage_proposal(proposal()).expect("scope matches");
        {
            let review = c.active_review_mut().expect("staged");
            assert!(review.accept_line(0));
            assert!(review.reject_line(1, "wrong account"));
        }
        let vm = ShellViewModel::build(&c, AREA, 1, false);
        let panel = vm.review.expect("review panel");
        assert_eq!(panel.proposal_id, "je-1");
        assert_eq!(panel.risk_tier_label, "R2 Material");
        assert_eq!(panel.rows.len(), 3);
        assert_eq!(
            panel.rows[0],
            ReviewRow {
                index: 0,
                account_code: "1212".into(),
                description: "Accounts receivable".into(),
                debit: "118.00".into(),
                credit: String::new(),
                decision: RowDecision::Accepted,
                cursor: false,
            }
        );
        assert_eq!(
            panel.rows[1].decision,
            RowDecision::Rejected {
                comment: "wrong account".into()
            }
        );
        assert!(panel.rows[1].cursor);
        assert_eq!(panel.rows[1].credit, "100.00");
        assert_eq!(panel.rows[2].decision, RowDecision::Pending);
        assert_eq!(panel.total_debit, "118.00");
        assert_eq!(panel.total_credit, "118.00");
        assert!(panel.balanced);
        assert!(!panel.can_post);
        assert_eq!(panel.approved_by, None);
    }

    #[test]
    fn review_cursor_is_clamped_to_last_line() {
        let mut c = controller();
        c.stage_proposal(proposal()).expect("scope matches");
        let vm = ShellViewModel::build(&c, AREA, 99, false);
        let panel = vm.review.expect("review panel");
        assert!(panel.rows[2].cursor);
        assert_eq!(panel.rows.iter().filter(|r| r.cursor).count(), 1);
    }

    #[test]
    fn pane_area_with_sidebar_also_excludes_the_sidebar() {
        assert_eq!(
            pane_area_with(1440, 900, true, true).width,
            1440 - REVIEW_PANEL_WIDTH - SIDEBAR_WIDTH
        );
        assert_eq!(
            pane_area_with(1440, 900, false, false),
            pane_area(1440, 900, false)
        );
        assert_eq!(pane_area_with(100, 10, true, true).width, 0, "no underflow");
    }

    #[test]
    fn pane_area_excludes_top_bar_footer_and_review_panel() {
        assert_eq!(
            pane_area(1440, 900, false),
            Rect {
                x: 0,
                y: 0,
                width: 1440,
                height: 900 - TOP_BAR_HEIGHT - FOOTER_HEIGHT,
            }
        );
        assert_eq!(pane_area(1440, 900, true).width, 1440 - REVIEW_PANEL_WIDTH);
        // Tiny windows never underflow.
        assert_eq!(
            pane_area(10, 10, true),
            Rect {
                x: 0,
                y: 0,
                width: 0,
                height: 0
            }
        );
    }
}
