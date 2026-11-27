//! Regression tests contributed by independent verification of the native shell (phase 5).

use super::commands::{ShellCommand, ShellState, KEY_BINDINGS};
use super::demo;
use super::view_model::{pane_area, PaneTile, Rect, ShellViewModel};
use crate::controller::{ControllerError, DrenyraHostController};
use drenyra_protocol::RiskTier;
use drenyra_ui::LineDecision;
use drenyra_workspace::PaneKind;

fn assert_tiles(area: Rect, tiles: &[PaneTile]) {
    let total: u64 = tiles
        .iter()
        .map(|t| t.rect.width as u64 * t.rect.height as u64)
        .sum();
    assert_eq!(
        total,
        area.width as u64 * area.height as u64,
        "area sum {area:?} {tiles:?}"
    );
    for t in tiles {
        assert!(t.rect.x >= area.x && t.rect.y >= area.y);
        assert!(t.rect.x + t.rect.width <= area.x + area.width);
        assert!(t.rect.y + t.rect.height <= area.y + area.height);
    }
    for (i, a) in tiles.iter().enumerate() {
        for b in &tiles[i + 1..] {
            let ox = a.rect.x < b.rect.x + b.rect.width && b.rect.x < a.rect.x + a.rect.width;
            let oy = a.rect.y < b.rect.y + b.rect.height && b.rect.y < a.rect.y + a.rect.height;
            assert!(!(ox && oy), "overlap {a:?} {b:?}");
        }
    }
    assert_eq!(tiles.iter().filter(|t| t.focused).count(), 1);
}

#[test]
fn probe_layout_tiles_exactly_for_odd_zero_and_deep_trees() {
    for &(w, h) in &[
        (1201u32, 799u32),
        (1, 1),
        (0, 0),
        (3, 1000),
        (7, 7),
        (1919, 1037),
    ] {
        let area = Rect {
            x: 0,
            y: 0,
            width: w,
            height: h,
        };
        let mut state = ShellState::new(DrenyraHostController::new(demo::demo_workspace()), false);
        for step in 0..40u32 {
            let cmd = match step % 5 {
                0 | 1 => ShellCommand::SplitFocused,
                2 => ShellCommand::FocusNextPane,
                3 => ShellCommand::SplitFocused,
                _ => {
                    if step % 3 == 0 {
                        ShellCommand::CloseFocused
                    } else {
                        ShellCommand::UndoClose
                    }
                }
            };
            state.apply(cmd, area);
            let vm = state.view(area);
            assert_eq!(
                vm.panes.len(),
                state.controller().workspaces().active().pane_count()
            );
            if w > 0 && h > 0 {
                assert_tiles(area, &vm.panes);
            }
        }
    }
}

#[test]
fn probe_split_uses_real_longest_side() {
    let tall = Rect {
        x: 0,
        y: 0,
        width: 300,
        height: 900,
    };
    let mut s = ShellState::new(DrenyraHostController::new(demo::demo_workspace()), false);
    assert!(s.apply(ShellCommand::SplitFocused, tall));
    let vm = s.view(tall);
    assert_eq!(
        vm.panes[0].rect.width, 300,
        "tall pane must stack vertically: {:?}",
        vm.panes
    );
    assert_eq!(vm.panes[1].rect.y, 450);
    assert_eq!(vm.panes[1].kind, PaneKind::LedgerViewer);
}

#[test]
fn probe_pane_area_never_underflows() {
    assert_eq!(
        pane_area(100, 50, true),
        Rect {
            x: 0,
            y: 0,
            width: 0,
            height: 0
        }
    );
    assert_eq!(
        pane_area(0, 0, false),
        Rect {
            x: 0,
            y: 0,
            width: 0,
            height: 0
        }
    );
    let mut s = ShellState::demo();
    let a = pane_area(0, 0, true);
    s.apply(ShellCommand::SplitFocused, a);
    s.apply(ShellCommand::FocusNextPane, a);
    let _ = s.view(a);
}

#[test]
fn probe_every_s4_shortcut_is_bound() {
    for (k, cmd) in [
        ("ctrl-d", ShellCommand::SplitFocused),
        ("ctrl-w", ShellCommand::CloseFocused),
        ("ctrl-shift-t", ShellCommand::UndoClose),
        ("ctrl-tab", ShellCommand::FocusNextPane),
        ("ctrl-k", ShellCommand::NextWorkspace),
        ("j", ShellCommand::ReviewCursorDown),
        ("k", ShellCommand::ReviewCursorUp),
        ("a", ShellCommand::AcceptLine),
        ("r", ShellCommand::RejectLine),
    ] {
        assert!(KEY_BINDINGS.contains(&(k, cmd)), "S4 binding {k}");
    }
}

#[test]
fn probe_review_cursor_clamps_and_targets_lines() {
    let area = pane_area(1440, 900, true);
    let mut s = ShellState::demo();
    assert!(!s.apply(ShellCommand::ReviewCursorUp, area));
    assert!(s.apply(ShellCommand::ReviewCursorDown, area));
    assert!(s.apply(ShellCommand::ReviewCursorDown, area));
    assert!(!s.apply(ShellCommand::ReviewCursorDown, area));
    assert!(s.apply(ShellCommand::RejectLine, area));
    assert!(s.apply(ShellCommand::ReviewCursorUp, area));
    assert!(s.apply(ShellCommand::AcceptLine, area));
    let d = s.controller().active_review().unwrap().decisions().to_vec();
    assert_eq!(d[0], LineDecision::Pending);
    assert_eq!(d[1], LineDecision::Accepted);
    assert!(matches!(d[2], LineDecision::Rejected { .. }));
    let vm = ShellViewModel::build(s.controller(), area, 999, true);
    let rows = &vm.review.unwrap().rows;
    assert_eq!(rows.iter().filter(|r| r.cursor).count(), 1);
    assert!(rows[2].cursor);
    let vm = s.view(area);
    assert!(!vm.review.unwrap().can_post);
}

#[test]
fn probe_workspace_switch_isolates_review_of_other_tenant() {
    let area = pane_area(1440, 900, true);
    let mut s = ShellState::demo();
    s.apply(ShellCommand::ReviewCursorDown, area);
    let before = s.controller().active_review().unwrap().decisions().to_vec();
    assert!(s.apply(ShellCommand::NextWorkspace, area));
    let vm = s.view(area);
    assert!(vm.review.is_none());
    assert_eq!(vm.guardrails.risk_tier_label, "R0 Read");
    assert_eq!(vm.guardrails.unposted_entries, 0);
    assert!(vm.workspaces[1].active && vm.workspaces[1].legal_name == "Sierra SRL");
    for c in [
        ShellCommand::AcceptLine,
        ShellCommand::RejectLine,
        ShellCommand::ReviewCursorDown,
        ShellCommand::ReviewCursorUp,
    ] {
        assert!(!s.apply(c, area), "{c:?} must be a no-op without a review");
    }
    assert!(s.apply(ShellCommand::NextWorkspace, area));
    assert_eq!(
        s.controller().active_review().unwrap().decisions(),
        &before[..]
    );
    let vm = s.view(area);
    assert!(vm.review.unwrap().rows[0].cursor, "cursor resets on switch");
    assert_eq!(vm.guardrails.risk_tier_label, "R2 Material");
}

#[test]
fn probe_single_workspace_next_is_noop() {
    let mut s = ShellState::new(DrenyraHostController::new(demo::demo_workspace()), false);
    assert!(!s.apply(ShellCommand::NextWorkspace, pane_area(800, 600, false)));
    assert!(!s.apply(ShellCommand::CloseFocused, pane_area(800, 600, false)));
    assert!(!s.apply(ShellCommand::UndoClose, pane_area(800, 600, false)));
    assert!(!s.apply(ShellCommand::FocusNextPane, pane_area(800, 600, false)));
}

#[test]
fn probe_risk_tier_untouched_on_error_paths_and_reset_only_for_owner() {
    let mut c = demo::demo_controller();
    let mut other = demo::demo_proposal();
    other.id = "x".into();
    assert_eq!(
        c.stage_proposal(other.clone()),
        Err(ControllerError::ReviewAlreadyStaged)
    );
    assert_eq!(
        c.workspaces().active().guardrails.active_risk_tier,
        RiskTier::R2Material
    );
    // Mismatched tenant on Sierra keeps R0.
    c.workspaces_mut().switch_to(1);
    let mut r3 = demo::demo_proposal();
    r3.risk_tier = RiskTier::R3Irreversible;
    assert!(c.stage_proposal(r3).is_err());
    assert_eq!(
        c.workspaces().active().guardrails.active_risk_tier,
        RiskTier::R0Read
    );
    c.workspaces_mut().switch_to(0);
    {
        let r = c.active_review_mut().unwrap();
        r.accept_all();
        assert!(r.record_human_approval("CPA"));
    }
    c.build_post_request().expect("frame");
    let id = c.pending_post_request().unwrap();
    assert_eq!(
        c.confirm_posted(id + 100),
        Err(ControllerError::UnknownPostRequest)
    );
    assert_eq!(
        c.workspaces().active().guardrails.active_risk_tier,
        RiskTier::R2Material
    );
    // Stage R1 on Sierra, then confirm Andino while Sierra is active.
    c.workspaces_mut().switch_to(1);
    let mut s = demo::demo_proposal();
    s.entity_ruc = "20987654321".into();
    s.fiscal_period = "2026-11".into();
    s.risk_tier = RiskTier::R1Draft;
    c.stage_proposal(s).unwrap();
    assert_eq!(c.confirm_posted(id), Ok(()));
    assert_eq!(
        c.workspaces().active().guardrails.active_risk_tier,
        RiskTier::R1Draft
    );
    assert_eq!(
        c.workspaces().workspaces()[0].guardrails.active_risk_tier,
        RiskTier::R0Read
    );
    assert_eq!(
        c.confirm_posted(id),
        Err(ControllerError::UnknownPostRequest)
    );
}
