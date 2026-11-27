//! Scroll bookkeeping for pane bodies, kept free of GPUI so it is unit-tested without a display.
//!
//! The window owns one scroll handle per company (RUC + period) and pane kind, so switching
//! companies keeps where each pane was scrolled. The agent transcript follows new output only
//! when the operator was already reading its end; scrolling up stops the follow.

use std::collections::BTreeMap;

use drenyra_workspace::{PaneKind, WorkspaceKey};

/// How far above the end (in logical pixels) still counts as "at the bottom", so a fractional
/// layout rounding or a one-notch nudge does not stop the transcript from following.
pub const FOLLOW_TOLERANCE_PX: f32 = 4.0;

/// Whether a vertical scroll position is at (or within [`FOLLOW_TOLERANCE_PX`] of) the end.
///
/// Uses GPUI's convention: `offset_y` is zero at the top and grows negative as the content moves
/// up; `max_scroll_y` is how far the content can move (zero when it fits). Non-finite values,
/// which only appear before a first layout, count as at the bottom.
pub fn is_at_bottom(offset_y: f32, max_scroll_y: f32) -> bool {
    if !offset_y.is_finite() || !max_scroll_y.is_finite() {
        return true;
    }
    let distance_from_bottom = max_scroll_y.max(0.0) + offset_y.min(0.0);
    distance_from_bottom <= FOLLOW_TOLERANCE_PX
}

/// Whether the transcript should jump to its end after an update: only when the update changed
/// it and the operator was at the bottom before the update.
pub fn should_follow(was_at_bottom: bool, transcript_changed: bool) -> bool {
    was_at_bottom && transcript_changed
}

/// Pane kinds as an ordered map key (`PaneKind` is not `Ord`); the match keeps it exhaustive.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum PaneSlot {
    Agent,
    Ledger,
    Evidence,
    Cli,
}

impl From<PaneKind> for PaneSlot {
    fn from(kind: PaneKind) -> Self {
        match kind {
            PaneKind::FiscalAgent => Self::Agent,
            PaneKind::LedgerViewer => Self::Ledger,
            PaneKind::EvidenceVault => Self::Evidence,
            PaneKind::FiscalCli => Self::Cli,
        }
    }
}

/// One scroll handle per (company, pane kind), created on first use and kept for the session.
///
/// `H` is a cheaply cloned shared handle (GPUI's `ScrollHandle` is an `Rc`), so the clone returned
/// by [`ScrollSlots::handle`] and the stored one observe the same scroll position.
#[derive(Debug)]
pub struct ScrollSlots<H> {
    handles: BTreeMap<(WorkspaceKey, PaneSlot), H>,
}

impl<H> Default for ScrollSlots<H> {
    fn default() -> Self {
        Self {
            handles: BTreeMap::new(),
        }
    }
}

impl<H: Clone + Default> ScrollSlots<H> {
    /// The handle for this company's pane kind, created on first use.
    pub fn handle(&mut self, workspace: &WorkspaceKey, kind: PaneKind) -> H {
        self.handles
            .entry((workspace.clone(), PaneSlot::from(kind)))
            .or_default()
            .clone()
    }

    /// The handle for this company's pane kind if that pane was ever rendered.
    pub fn get(&self, workspace: &WorkspaceKey, kind: PaneKind) -> Option<&H> {
        self.handles.get(&(workspace.clone(), PaneSlot::from(kind)))
    }
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::rc::Rc;

    use super::*;

    /// Stand-in for GPUI's shared `ScrollHandle`: clones share one scroll offset.
    type FakeHandle = Rc<Cell<f32>>;

    fn andino() -> WorkspaceKey {
        WorkspaceKey::new("20123456789", "2026-11")
    }

    fn sierra() -> WorkspaceKey {
        WorkspaceKey::new("20987654321", "2026-11")
    }

    #[test]
    fn content_that_fits_is_at_the_bottom() {
        assert!(is_at_bottom(0.0, 0.0));
    }

    #[test]
    fn the_end_of_the_content_is_the_bottom() {
        assert!(is_at_bottom(-120.0, 120.0));
    }

    #[test]
    fn a_few_pixels_above_the_end_still_count_as_the_bottom() {
        assert!(is_at_bottom(-120.0 + FOLLOW_TOLERANCE_PX, 120.0));
        assert!(is_at_bottom(-117.5, 120.0));
    }

    #[test]
    fn scrolled_up_beyond_the_tolerance_is_not_the_bottom() {
        assert!(!is_at_bottom(-120.0 + FOLLOW_TOLERANCE_PX + 0.5, 120.0));
        assert!(!is_at_bottom(-40.0, 120.0));
        assert!(!is_at_bottom(0.0, 120.0));
    }

    #[test]
    fn a_position_before_the_first_layout_counts_as_the_bottom() {
        assert!(is_at_bottom(f32::NAN, 120.0));
        assert!(is_at_bottom(-10.0, f32::INFINITY));
    }

    #[test]
    fn new_output_follows_only_when_the_operator_was_at_the_bottom() {
        assert!(should_follow(true, true));
        assert!(
            !should_follow(false, true),
            "scrolled up: never yanked down"
        );
        assert!(
            !should_follow(true, false),
            "nothing new: nothing to follow"
        );
        assert!(!should_follow(false, false));
    }

    #[test]
    fn the_same_company_and_pane_get_the_same_handle() {
        let mut slots: ScrollSlots<FakeHandle> = ScrollSlots::default();
        slots.handle(&andino(), PaneKind::EvidenceVault).set(-80.0);
        assert_eq!(
            slots.handle(&andino(), PaneKind::EvidenceVault).get(),
            -80.0
        );
        assert_eq!(
            slots
                .get(&andino(), PaneKind::EvidenceVault)
                .map(|h| h.get()),
            Some(-80.0)
        );
    }

    #[test]
    fn switching_companies_keeps_each_scroll_position() {
        let mut slots: ScrollSlots<FakeHandle> = ScrollSlots::default();
        slots.handle(&andino(), PaneKind::LedgerViewer).set(-30.0);
        slots.handle(&sierra(), PaneKind::LedgerViewer).set(-200.0);
        assert_eq!(slots.handle(&andino(), PaneKind::LedgerViewer).get(), -30.0);
        assert_eq!(
            slots.handle(&sierra(), PaneKind::LedgerViewer).get(),
            -200.0
        );
    }

    #[test]
    fn each_pane_kind_scrolls_on_its_own() {
        let mut slots: ScrollSlots<FakeHandle> = ScrollSlots::default();
        let kinds = [
            PaneKind::FiscalAgent,
            PaneKind::LedgerViewer,
            PaneKind::EvidenceVault,
            PaneKind::FiscalCli,
        ];
        for (i, kind) in kinds.into_iter().enumerate() {
            slots.handle(&andino(), kind).set(-(i as f32) - 1.0);
        }
        for (i, kind) in kinds.into_iter().enumerate() {
            assert_eq!(slots.handle(&andino(), kind).get(), -(i as f32) - 1.0);
        }
    }

    #[test]
    fn a_pane_never_rendered_has_no_handle() {
        let mut slots: ScrollSlots<FakeHandle> = ScrollSlots::default();
        slots.handle(&andino(), PaneKind::FiscalAgent);
        assert!(slots.get(&sierra(), PaneKind::FiscalAgent).is_none());
        assert!(slots.get(&andino(), PaneKind::FiscalCli).is_none());
    }
}
