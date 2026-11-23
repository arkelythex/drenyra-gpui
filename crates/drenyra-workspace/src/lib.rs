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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FiscalWorkspace {
    pub entity_ruc: String,
    pub legal_name: String,
    pub fiscal_period: String,
    pub activity: WorkspaceActivity,
    pub sidebar_open: bool,
    pub sidebar_tab: SidebarTab,
    pub guardrails: SessionGuardrails,
    pub layout: PaneNode,
    pub focused_pane_id: u64,
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

    /// Splits the focused pane along its longer side (Hyprland dwindle).
    pub fn split_focused(
        &mut self,
        kind: PaneKind,
        viewport_width: u32,
        viewport_height: u32,
    ) -> u64 {
        let axis = if viewport_width >= viewport_height {
            SplitAxis::Horizontal
        } else {
            SplitAxis::Vertical
        };
        let id = self.next_pane_id;
        self.next_pane_id += 1;

        let new_leaf = PaneLeaf { id, kind };
        let inserted = self
            .layout
            .split_leaf(self.focused_pane_id, axis, new_leaf);
        debug_assert!(inserted, "focused pane must exist in layout");
        self.focused_pane_id = id;
        id
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
    pub fn undo_close(&mut self, viewport_width: u32, viewport_height: u32) -> Option<u64> {
        let leaf = self.closed_history.pop()?;
        Some(self.split_focused(leaf.kind, viewport_width, viewport_height))
    }

    pub fn pane_count(&self) -> usize {
        self.layout.count()
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
        assert_eq!(ws.focused_pane_id, ledger_id);
        match &ws.layout {
            PaneNode::Split { axis, .. } => assert_eq!(*axis, SplitAxis::Horizontal),
            _ => panic!("expected horizontal split when width >= height"),
        }

        let evidence_id = ws.split_focused(PaneKind::EvidenceVault, 700, 900);
        assert_eq!(ws.pane_count(), 3);
        assert_eq!(ws.focused_pane_id, evidence_id);
    }

    #[test]
    fn closing_pane_collapses_tree_and_undo_restores_it() {
        let mut ws = FiscalWorkspace::new("20123456789", "Andino SAC", "2026-10");
        let ledger_id = ws.split_focused(PaneKind::LedgerViewer, 1440, 900);
        assert_eq!(ws.focused_pane_id, ledger_id);

        let closed = ws.close_focused().expect("should close second pane");
        assert_eq!(closed.kind, PaneKind::LedgerViewer);
        assert_eq!(ws.pane_count(), 1);
        assert_eq!(ws.focused_pane_id, 1);

        // Cannot close the last remaining pane.
        assert!(ws.close_focused().is_none());

        let restored_id = ws
            .undo_close(1440, 900)
            .expect("should restore closed pane");
        assert_eq!(ws.pane_count(), 2);
        assert_eq!(ws.focused_pane_id, restored_id);
    }
}
