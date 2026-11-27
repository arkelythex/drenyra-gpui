//! Scroll bookkeeping for pane bodies, kept free of GPUI so it is unit-tested without a display.
//!
//! The window owns one scroll slot per pane tile (company RUC + period, pane id): two tiles of the
//! same kind scroll on their own, and switching companies keeps where each tile was scrolled.
//! Each agent transcript view carries an explicit [`Follow`] intent that only the operator's own
//! scrolling changes; while it is on, every layout (new output, resize) pins the view to its end.

use std::collections::BTreeMap;

use drenyra_workspace::WorkspaceKey;

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

/// Whether a transcript view keeps showing its newest entry: the operator's intent, not a
/// reading of the geometry.
///
/// It starts on. Only the operator's own scrolling changes it: landing above the end (beyond
/// [`FOLLOW_TOLERANCE_PX`]) turns it off, landing at the end turns it on. Layout never changes it,
/// so a resize that leaves the old offset above the end, or new output that grows the content,
/// keeps following and is re-pinned. GPUI applies the wheel's own scroll, so the window notes the
/// scroll in its wheel listener and the landing position is read at the next frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Follow {
    following: bool,
    operator_scrolled: bool,
}

impl Default for Follow {
    fn default() -> Self {
        Self {
            following: true,
            operator_scrolled: false,
        }
    }
}

impl Follow {
    /// Whether the view follows new output.
    pub fn is_following(&self) -> bool {
        self.following
    }

    /// The operator scrolled this view (mouse wheel or touchpad); resolved by the next
    /// [`Follow::frame`].
    pub fn note_operator_scroll(&mut self) {
        self.operator_scrolled = true;
    }

    /// Called once per frame, before layout, with the view's current scroll position (see
    /// [`is_at_bottom`] for the convention). A pending operator scroll is resolved against that
    /// position; without one the position is ignored. Returns whether this frame's layout must
    /// pin the view to its end, which is whenever it follows: that covers the first layout, new
    /// output and resizes alike.
    pub fn frame(&mut self, offset_y: f32, max_scroll_y: f32) -> bool {
        if std::mem::take(&mut self.operator_scrolled) {
            self.following = is_at_bottom(offset_y, max_scroll_y);
        }
        self.following
    }
}

/// Scroll state of one pane tile: GPUI's shared handle and, for agent transcripts, the follow
/// intent (unused by the other pane kinds).
#[derive(Debug, Default)]
pub struct ScrollSlot<H> {
    pub handle: H,
    pub follow: Follow,
}

/// One scroll slot per (company, pane id), created on first use and kept for the session.
///
/// `H` is a cheaply cloned shared handle (GPUI's `ScrollHandle` is an `Rc`), so a clone of
/// [`ScrollSlot::handle`] and the stored one observe the same scroll position.
#[derive(Debug)]
pub struct ScrollSlots<H> {
    slots: BTreeMap<(WorkspaceKey, u64), ScrollSlot<H>>,
}

impl<H> Default for ScrollSlots<H> {
    fn default() -> Self {
        Self {
            slots: BTreeMap::new(),
        }
    }
}

impl<H: Default> ScrollSlots<H> {
    /// The slot of this company's pane tile, created on first use.
    ///
    /// Slots are never dropped: a closed pane keeps its slot, so undo close (which restores the
    /// same pane id) brings back its scroll position. The cost is one small entry per pane ever
    /// rendered in the session.
    pub fn slot(&mut self, workspace: &WorkspaceKey, pane_id: u64) -> &mut ScrollSlot<H> {
        self.slots.entry((workspace.clone(), pane_id)).or_default()
    }

    /// The slot of this company's pane tile if that tile was ever rendered.
    #[cfg(test)]
    pub fn get(&self, workspace: &WorkspaceKey, pane_id: u64) -> Option<&ScrollSlot<H>> {
        self.slots.get(&(workspace.clone(), pane_id))
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

    fn offset(slots: &mut ScrollSlots<FakeHandle>, ws: &WorkspaceKey, pane_id: u64) -> f32 {
        slots.slot(ws, pane_id).handle.get()
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
    fn a_new_transcript_view_follows_and_pins_its_first_layout() {
        let mut follow = Follow::default();
        assert!(follow.is_following());
        assert!(
            follow.frame(0.0, 300.0),
            "first layout opens on the newest entry"
        );
        assert!(follow.is_following());
    }

    #[test]
    fn operator_scrolling_that_leaves_the_bottom_stops_the_follow() {
        let mut follow = Follow::default();
        follow.note_operator_scroll();
        assert!(!follow.frame(-40.0, 120.0), "scrolled up: not pinned");
        assert!(!follow.is_following());
    }

    #[test]
    fn operator_scrolling_within_the_tolerance_keeps_the_follow() {
        let mut follow = Follow::default();
        follow.note_operator_scroll();
        assert!(follow.frame(-120.0 + FOLLOW_TOLERANCE_PX, 120.0));
        assert!(follow.is_following());
    }

    #[test]
    fn operator_scrolling_back_to_the_bottom_resumes_the_follow() {
        let mut follow = Follow::default();
        follow.note_operator_scroll();
        follow.frame(-40.0, 120.0);
        follow.note_operator_scroll();
        assert!(
            follow.frame(-118.0, 120.0),
            "within the tolerance of the end"
        );
        assert!(follow.is_following());

        follow.note_operator_scroll();
        follow.frame(-40.0, 120.0);
        follow.note_operator_scroll();
        // GPUI clamps a wheel overshoot only at the next layout.
        assert!(follow.frame(-150.0, 120.0), "past the end");
        assert!(follow.is_following());
    }

    #[test]
    fn a_resize_that_shrinks_the_pane_while_following_keeps_following_and_re_pins() {
        let mut follow = Follow::default();
        assert!(follow.frame(-100.0, 100.0));
        // The pane got shorter: the same offset is now 200 px above the end.
        assert!(!is_at_bottom(-100.0, 300.0));
        assert!(follow.frame(-100.0, 300.0), "re-pinned to the end");
        assert!(
            follow.is_following(),
            "a resize is not the operator scrolling up"
        );
    }

    #[test]
    fn new_output_while_following_re_pins() {
        let mut follow = Follow::default();
        assert!(follow.frame(-120.0, 120.0));
        // New entries grew the content by 280 px since the last layout.
        assert!(follow.frame(-120.0, 400.0));
        assert!(follow.is_following());
    }

    #[test]
    fn layout_never_resumes_a_follow_the_operator_stopped() {
        let mut follow = Follow::default();
        follow.note_operator_scroll();
        follow.frame(-40.0, 300.0);
        // The pane grew until the content fits: the position is the end, but not by the operator.
        assert!(!follow.frame(0.0, 0.0));
        assert!(!follow.is_following());
        // New output makes it overflow again: still not yanked down.
        assert!(!follow.frame(0.0, 400.0));
        assert!(!follow.is_following());
    }

    #[test]
    fn an_operator_scroll_is_resolved_once() {
        let mut follow = Follow::default();
        follow.note_operator_scroll();
        follow.frame(-40.0, 300.0);
        // GPUI clamps the offset to the end after content shrank; no new operator scroll.
        assert!(!follow.frame(-300.0, 300.0));
        assert!(!follow.is_following());
    }

    #[test]
    fn scrolling_up_in_the_frame_that_brings_new_output_wins() {
        let mut follow = Follow::default();
        assert!(follow.frame(-120.0, 120.0));
        follow.note_operator_scroll();
        assert!(
            !follow.frame(-80.0, 400.0),
            "never yanked down by the same frame's output"
        );
        assert!(!follow.is_following());
    }

    #[test]
    fn the_same_company_and_pane_get_the_same_handle() {
        let mut slots: ScrollSlots<FakeHandle> = ScrollSlots::default();
        slots.slot(&andino(), 3).handle.set(-80.0);
        assert_eq!(offset(&mut slots, &andino(), 3), -80.0);
        assert_eq!(slots.get(&andino(), 3).map(|s| s.handle.get()), Some(-80.0));
    }

    #[test]
    fn two_tiles_of_the_same_kind_scroll_on_their_own() {
        // Two Evidence tiles (ids 2 and 5) in one company: a short one is never clamped by the
        // taller one's maximum offset.
        let mut slots: ScrollSlots<FakeHandle> = ScrollSlots::default();
        slots.slot(&andino(), 2).handle.set(-300.0);
        slots.slot(&andino(), 5).handle.set(-20.0);
        assert_eq!(offset(&mut slots, &andino(), 2), -300.0);
        assert_eq!(offset(&mut slots, &andino(), 5), -20.0);
    }

    #[test]
    fn two_agent_tiles_follow_on_their_own() {
        let mut slots: ScrollSlots<FakeHandle> = ScrollSlots::default();
        let first = &mut slots.slot(&andino(), 1).follow;
        first.note_operator_scroll();
        first.frame(-40.0, 300.0);
        assert!(!slots.slot(&andino(), 1).follow.is_following());
        assert!(slots.slot(&andino(), 4).follow.is_following());
    }

    #[test]
    fn switching_companies_keeps_each_scroll_position() {
        // Pane ids are per workspace layout, so both companies can have a pane 1.
        let mut slots: ScrollSlots<FakeHandle> = ScrollSlots::default();
        slots.slot(&andino(), 1).handle.set(-30.0);
        slots.slot(&sierra(), 1).handle.set(-200.0);
        assert_eq!(offset(&mut slots, &andino(), 1), -30.0);
        assert_eq!(offset(&mut slots, &sierra(), 1), -200.0);
    }

    #[test]
    fn a_closed_pane_keeps_its_slot_so_undo_close_restores_its_position() {
        // The window never drops slots: undo close brings the pane back with the same id.
        let mut slots: ScrollSlots<FakeHandle> = ScrollSlots::default();
        slots.slot(&andino(), 2).handle.set(-90.0);
        slots.slot(&andino(), 3); // pane 2 closed; only pane 3 rendered
        assert_eq!(slots.get(&andino(), 2).map(|s| s.handle.get()), Some(-90.0));
    }

    #[test]
    fn a_pane_never_rendered_has_no_slot() {
        let mut slots: ScrollSlots<FakeHandle> = ScrollSlots::default();
        slots.slot(&andino(), 1);
        assert!(slots.get(&sierra(), 1).is_none());
        assert!(slots.get(&andino(), 2).is_none());
    }
}
