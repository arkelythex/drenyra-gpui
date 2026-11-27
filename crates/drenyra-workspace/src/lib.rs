//! Fiscal Workspaces, Hyprland-style dwindle pane tiling, Accounting Sidebar, and RDA guardrails.

use drenyra_protocol::RiskTier;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkspaceActivity {
    Idle,
    Reconciling,
    AwaitingApproval,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PaneKind {
    FiscalAgent,
    LedgerViewer,
    EvidenceVault,
    FiscalCli,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SplitAxis {
    Horizontal,
    Vertical,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PaneLeaf {
    pub id: u64,
    pub kind: PaneKind,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PaneNode {
    Leaf(PaneLeaf),
    Split {
        axis: SplitAxis,
        first: Box<PaneNode>,
        second: Box<PaneNode>,
    },
}

impl PaneNode {
    fn count(&self) -> usize {
        match self {
            Self::Leaf(_) => 1,
            Self::Split { first, second, .. } => first.count() + second.count(),
        }
    }

    fn contains(&self, target_id: u64) -> bool {
        match self {
            Self::Leaf(leaf) => leaf.id == target_id,
            Self::Split { first, second, .. } => {
                first.contains(target_id) || second.contains(target_id)
            }
        }
    }

    fn first_leaf_id(&self) -> u64 {
        match self {
            Self::Leaf(leaf) => leaf.id,
            Self::Split { first, .. } => first.first_leaf_id(),
        }
    }

    fn split_leaf(&mut self, target_id: u64, axis: SplitAxis, new_leaf: PaneLeaf) -> bool {
        match self {
            Self::Leaf(existing) if existing.id == target_id => {
                let first = Box::new(Self::Leaf(existing.clone()));
                let second = Box::new(Self::Leaf(new_leaf));
                *self = Self::Split {
                    axis,
                    first,
                    second,
                };
                true
            }
            Self::Leaf(_) => false,
            Self::Split { first, second, .. } => {
                first.split_leaf(target_id, axis, new_leaf.clone())
                    || second.split_leaf(target_id, axis, new_leaf)
            }
        }
    }

    fn remove_leaf(self, target_id: u64) -> (Option<Self>, Option<PaneLeaf>) {
        match self {
            Self::Leaf(leaf) if leaf.id == target_id => (None, Some(leaf)),
            Self::Leaf(leaf) => (Some(Self::Leaf(leaf)), None),
            Self::Split {
                axis,
                first,
                second,
            } => {
                let (new_first, removed) = first.remove_leaf(target_id);
                if removed.is_some() {
                    let replacement = match new_first {
                        Some(f) => Self::Split {
                            axis,
                            first: Box::new(f),
                            second,
                        },
                        None => *second,
                    };
                    return (Some(replacement), removed);
                }

                let first_node = new_first.expect("first subtree preserved when target not found");
                let (new_second, removed) = second.remove_leaf(target_id);
                if removed.is_some() {
                    let replacement = match new_second {
                        Some(s) => Self::Split {
                            axis,
                            first: Box::new(first_node),
                            second: Box::new(s),
                        },
                        None => first_node,
                    };
                    return (Some(replacement), removed);
                }

                let second_node =
                    new_second.expect("second subtree preserved when target not found");
                (
                    Some(Self::Split {
                        axis,
                        first: Box::new(first_node),
                        second: Box::new(second_node),
                    }),
                    None,
                )
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SidebarTab {
    DocumentsAndLedgers,
    UnpostedEntries,
    FiscalPeriods,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionGuardrails {
    pub rda_enabled: bool,
    pub guardian_angel_active: bool,
    pub unposted_entries: usize,
    pub active_risk_tier: RiskTier,
}

impl Default for SessionGuardrails {
    fn default() -> Self {
        Self {
            rda_enabled: true,
            guardian_angel_active: true,
            unposted_entries: 0,
            active_risk_tier: RiskTier::R0Read,
        }
    }
}

/// Identity of a fiscal workspace: one company (RUC) in one fiscal period.
///
/// State that belongs to a company, such as its agent conversation, is keyed by this value, never
/// by the workspace's position in the switcher. Both parts are trimmed.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct WorkspaceKey {
    entity_ruc: String,
    fiscal_period: String,
}

impl WorkspaceKey {
    pub fn new(entity_ruc: &str, fiscal_period: &str) -> Self {
        Self {
            entity_ruc: entity_ruc.trim().to_owned(),
            fiscal_period: fiscal_period.trim().to_owned(),
        }
    }

    pub fn entity_ruc(&self) -> &str {
        &self.entity_ruc
    }

    pub fn fiscal_period(&self) -> &str {
        &self.fiscal_period
    }
}

/// A workspace for the same RUC and period is already open.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DuplicateWorkspace(pub WorkspaceKey);

impl std::fmt::Display for DuplicateWorkspace {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "a workspace for RUC {} and period {} is already open",
            self.0.entity_ruc, self.0.fiscal_period
        )
    }
}

impl std::error::Error for DuplicateWorkspace {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FiscalWorkspace {
    pub entity_ruc: String,
    pub legal_name: String,
    pub fiscal_period: String,
    pub activity: WorkspaceActivity,
    pub sidebar_open: bool,
    pub sidebar_tab: SidebarTab,
    pub guardrails: SessionGuardrails,
    layout: PaneNode,
    focused_pane_id: u64,
    next_pane_id: u64,
    closed_history: Vec<PaneLeaf>,
}

impl FiscalWorkspace {
    pub fn new(
        entity_ruc: impl Into<String>,
        legal_name: impl Into<String>,
        fiscal_period: impl Into<String>,
    ) -> Self {
        Self {
            entity_ruc: entity_ruc.into(),
            legal_name: legal_name.into(),
            fiscal_period: fiscal_period.into(),
            activity: WorkspaceActivity::Idle,
            sidebar_open: true,
            sidebar_tab: SidebarTab::DocumentsAndLedgers,
            guardrails: SessionGuardrails::default(),
            layout: PaneNode::Leaf(PaneLeaf {
                id: 1,
                kind: PaneKind::FiscalAgent,
            }),
            focused_pane_id: 1,
            next_pane_id: 2,
            closed_history: Vec::new(),
        }
    }

    /// This workspace's identity (RUC + fiscal period).
    pub fn key(&self) -> WorkspaceKey {
        WorkspaceKey::new(&self.entity_ruc, &self.fiscal_period)
    }

    pub fn layout(&self) -> &PaneNode {
        &self.layout
    }

    pub const fn focused_pane_id(&self) -> u64 {
        self.focused_pane_id
    }

    /// Moves focus to `pane_id` iff that pane exists in the layout tree.
    pub fn focus_pane(&mut self, pane_id: u64) -> bool {
        if self.layout.contains(pane_id) {
            self.focused_pane_id = pane_id;
            true
        } else {
            false
        }
    }

    /// Splits the focused pane along its longer side (Hyprland dwindle).
    pub fn split_focused(&mut self, kind: PaneKind, pane_width: u32, pane_height: u32) -> u64 {
        let axis = if pane_width >= pane_height {
            SplitAxis::Horizontal
        } else {
            SplitAxis::Vertical
        };
        let id = self.next_pane_id;
        let new_leaf = PaneLeaf { id, kind };
        if self.layout.split_leaf(self.focused_pane_id, axis, new_leaf) {
            self.next_pane_id += 1;
            self.focused_pane_id = id;
            id
        } else {
            self.focused_pane_id
        }
    }

    /// Closes the focused pane if more than one pane exists, recording it for undo_close.
    pub fn close_focused(&mut self) -> Option<PaneLeaf> {
        if self.pane_count() <= 1 {
            return None;
        }
        let dummy = PaneNode::Leaf(PaneLeaf {
            id: 0,
            kind: PaneKind::FiscalAgent,
        });
        let current = std::mem::replace(&mut self.layout, dummy);
        let (next_layout, removed) = current.remove_leaf(self.focused_pane_id);
        self.layout = next_layout.expect("non-singular layout always retains at least one pane");
        if let Some(leaf) = removed {
            self.focused_pane_id = self.layout.first_leaf_id();
            self.closed_history.push(leaf.clone());
            Some(leaf)
        } else {
            None
        }
    }

    /// Reopens the most recently closed pane by splitting the currently focused pane.
    pub fn undo_close(&mut self, pane_width: u32, pane_height: u32) -> Option<u64> {
        let leaf = self.closed_history.pop()?;
        Some(self.split_focused(leaf.kind, pane_width, pane_height))
    }

    pub fn pane_count(&self) -> usize {
        self.layout.count()
    }
}

/// Multi-entity workspace manager (analogous to gentle-shell-gpui's project switcher `⌃⌘K`).
#[derive(Debug, Clone)]
pub struct WorkspaceManager {
    workspaces: Vec<FiscalWorkspace>,
    active_index: usize,
}

impl WorkspaceManager {
    pub fn new(initial: FiscalWorkspace) -> Self {
        Self {
            workspaces: vec![initial],
            active_index: 0,
        }
    }

    /// Adds `workspace` and returns its index, or refuses it when a workspace for the same RUC and
    /// period is already open (two would share one identity and its agent conversation).
    pub fn add_workspace(
        &mut self,
        workspace: FiscalWorkspace,
    ) -> Result<usize, DuplicateWorkspace> {
        let key = workspace.key();
        if self.index_of(&key).is_some() {
            return Err(DuplicateWorkspace(key));
        }
        self.workspaces.push(workspace);
        Ok(self.workspaces.len() - 1)
    }

    /// Index of the workspace identified by `key`, if open.
    pub fn index_of(&self, key: &WorkspaceKey) -> Option<usize> {
        self.workspaces.iter().position(|ws| &ws.key() == key)
    }

    /// Identity of the active workspace.
    pub fn active_key(&self) -> WorkspaceKey {
        self.active().key()
    }

    pub fn switch_to(&mut self, index: usize) -> bool {
        if index < self.workspaces.len() {
            self.active_index = index;
            true
        } else {
            false
        }
    }

    /// Index of the active workspace; stable for the manager's lifetime because workspaces are never removed.
    pub fn active_index(&self) -> usize {
        self.active_index
    }

    /// All workspaces in switcher order.
    pub fn workspaces(&self) -> &[FiscalWorkspace] {
        &self.workspaces
    }

    /// Mutable access to the workspace at `index` without changing focus.
    pub fn get_mut(&mut self, index: usize) -> Option<&mut FiscalWorkspace> {
        self.workspaces.get_mut(index)
    }

    pub fn active(&self) -> &FiscalWorkspace {
        &self.workspaces[self.active_index]
    }

    pub fn active_mut(&mut self) -> &mut FiscalWorkspace {
        &mut self.workspaces[self.active_index]
    }

    /// Returns the highest-priority background activity across non-active workspaces
    /// (AwaitingApproval > Reconciling > Idle) for the workspace chip indicator dot.
    pub fn background_alert(&self) -> WorkspaceActivity {
        let mut alert = WorkspaceActivity::Idle;
        for (idx, ws) in self.workspaces.iter().enumerate() {
            if idx == self.active_index {
                continue;
            }
            match ws.activity {
                WorkspaceActivity::AwaitingApproval => return WorkspaceActivity::AwaitingApproval,
                WorkspaceActivity::Reconciling => alert = WorkspaceActivity::Reconciling,
                WorkspaceActivity::Idle => {}
            }
        }
        alert
    }

    /// Case-insensitive filter over RUC, legal name, or fiscal period for the `⌘K` switcher.
    pub fn search(&self, query: &str) -> Vec<usize> {
        let needle = query.trim().to_ascii_lowercase();
        if needle.is_empty() {
            return (0..self.workspaces.len()).collect();
        }
        self.workspaces
            .iter()
            .enumerate()
            .filter(|(_, ws)| {
                ws.entity_ruc.to_ascii_lowercase().contains(&needle)
                    || ws.legal_name.to_ascii_lowercase().contains(&needle)
                    || ws.fiscal_period.to_ascii_lowercase().contains(&needle)
            })
            .map(|(idx, _)| idx)
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dwindle_split_chooses_longer_axis_and_tracks_focus() {
        let mut ws = FiscalWorkspace::new("20123456789", "Andino SAC", "2026-10");
        assert_eq!(ws.pane_count(), 1);
        assert!(ws.guardrails.rda_enabled);

        let ledger_id = ws.split_focused(PaneKind::LedgerViewer, 1440, 900);
        assert_eq!(ws.pane_count(), 2);
        assert_eq!(ws.focused_pane_id(), ledger_id);
        match ws.layout() {
            PaneNode::Split { axis, .. } => assert_eq!(*axis, SplitAxis::Horizontal),
            _ => panic!("expected horizontal split when pane_width >= pane_height"),
        }

        let evidence_id = ws.split_focused(PaneKind::EvidenceVault, 700, 900);
        assert_eq!(ws.pane_count(), 3);
        assert_eq!(ws.focused_pane_id(), evidence_id);

        assert!(ws.focus_pane(1));
        assert_eq!(ws.focused_pane_id(), 1);
        assert!(!ws.focus_pane(999));
        assert_eq!(ws.focused_pane_id(), 1);
    }

    #[test]
    fn closing_pane_collapses_tree_and_undo_restores_it() {
        let mut ws = FiscalWorkspace::new("20123456789", "Andino SAC", "2026-10");
        let ledger_id = ws.split_focused(PaneKind::LedgerViewer, 1440, 900);
        assert_eq!(ws.focused_pane_id(), ledger_id);

        let closed = ws.close_focused().expect("should close second pane");
        assert_eq!(closed.kind, PaneKind::LedgerViewer);
        assert_eq!(ws.pane_count(), 1);
        assert_eq!(ws.focused_pane_id(), 1);

        // Cannot close the last remaining pane.
        assert!(ws.close_focused().is_none());

        let restored_id = ws
            .undo_close(1440, 900)
            .expect("should restore closed pane");
        assert_eq!(ws.pane_count(), 2);
        assert_eq!(ws.focused_pane_id(), restored_id);
    }

    #[test]
    fn workspace_manager_tracks_background_alerts_and_search() {
        let ws1 = FiscalWorkspace::new("20123456789", "Andino SAC", "2026-10");
        let mut ws2 = FiscalWorkspace::new("20987654321", "Sierra SRL", "2026-10");
        ws2.activity = WorkspaceActivity::Reconciling;
        let mut ws3 = FiscalWorkspace::new("20555555555", "Costa EIRL", "2026-11");
        ws3.activity = WorkspaceActivity::AwaitingApproval;

        let mut mgr = WorkspaceManager::new(ws1);
        let idx2 = mgr.add_workspace(ws2).expect("distinct workspace");
        let idx3 = mgr.add_workspace(ws3).expect("distinct workspace");

        // Active is ws1; ws3 awaits approval so background alert is AwaitingApproval.
        assert_eq!(mgr.background_alert(), WorkspaceActivity::AwaitingApproval);

        // Switch to ws3; remaining background alert drops to ws2's Reconciling.
        assert!(mgr.switch_to(idx3));
        assert_eq!(mgr.active().entity_ruc, "20555555555");
        assert_eq!(mgr.background_alert(), WorkspaceActivity::Reconciling);
        assert!(!mgr.switch_to(99));

        assert_eq!(mgr.search("sierra"), vec![idx2]);
        assert_eq!(mgr.search("2026-10"), vec![0, idx2]);
        assert_eq!(mgr.search(""), vec![0, idx2, idx3]);
    }

    #[test]
    fn workspaces_are_identified_by_ruc_and_period_and_duplicates_are_refused() {
        let mut mgr =
            WorkspaceManager::new(FiscalWorkspace::new("20123456789", "Andino SAC", "2026-10"));
        // Same RUC, another period: a different workspace.
        let next_period = mgr
            .add_workspace(FiscalWorkspace::new("20123456789", "Andino SAC", "2026-11"))
            .expect("another period is another workspace");
        assert_eq!(next_period, 1);

        // Same RUC and period (surrounding spaces ignored): refused, manager unchanged.
        let err = mgr
            .add_workspace(FiscalWorkspace::new(
                " 20123456789 ",
                "Otra SAC",
                "2026-10 ",
            ))
            .expect_err("duplicate RUC + period");
        assert_eq!(
            err.to_string(),
            "a workspace for RUC 20123456789 and period 2026-10 is already open"
        );
        assert_eq!(mgr.workspaces().len(), 2);

        let key = mgr.workspaces()[1].key();
        assert_eq!(key, WorkspaceKey::new("20123456789", "2026-11"));
        assert_eq!(mgr.index_of(&key), Some(1));
        assert_eq!(
            mgr.active_key(),
            WorkspaceKey::new("20123456789", "2026-10")
        );
        assert_eq!(
            mgr.index_of(&WorkspaceKey::new("20987654321", "2026-10")),
            None
        );
    }

    #[test]
    fn background_alert_ignores_active_workspace_activity() {
        let mut active_ws = FiscalWorkspace::new("20123456789", "Andino SAC", "2026-10");
        active_ws.activity = WorkspaceActivity::AwaitingApproval;
        let idle_ws = FiscalWorkspace::new("20987654321", "Sierra SRL", "2026-10");

        let mut mgr = WorkspaceManager::new(active_ws);
        let idx2 = mgr.add_workspace(idle_ws).expect("distinct workspace");

        // Active workspace is AwaitingApproval, other is Idle -> background_alert must be Idle.
        assert_eq!(mgr.background_alert(), WorkspaceActivity::Idle);

        // Switching to idx2 makes the AwaitingApproval workspace a background workspace.
        assert!(mgr.switch_to(idx2));
        assert_eq!(mgr.background_alert(), WorkspaceActivity::AwaitingApproval);
    }
}
