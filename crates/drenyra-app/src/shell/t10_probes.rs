//! Permanent probes for T10 (approval replacement and the session evidence log), rebuilt from
//! the independent verifier's findings (its scratch files were lost) and its advisories A1/A2.
//!
//! 1. Every refused operation leaves the controller's `Debug` snapshot identical and appends no
//!    evidence.
//! 2. Evidence `seq` is strictly increasing 1..=n across workspaces and every event lands under
//!    the key of the workspace that owns its proposal, even when another workspace is active.
//! 3. A1: the posted record is what the `ledger_post` frame carried. A2: an open overlay blocks
//!    workspace switches, and the approval dialog compares against the approver it showed.

use super::commands::{ShellCommand, ShellState, APPROVAL_CLEARED_MESSAGE};
use super::demo::{demo_controller, demo_proposal, demo_workspace};
use super::view_model::Rect;
use crate::controller::{DrenyraHostController, EvidenceKind};
use drenyra_protocol::JournalEntryProposal;
use drenyra_ui::{ReplaceApprovalError, MAX_APPROVER_ID_CHARS};
use drenyra_workspace::{FiscalWorkspace, WorkspaceKey};

const WIDE: Rect = Rect {
    x: 0,
    y: 0,
    width: 1200,
    height: 800,
};
const DEMO_ID: &str = "je-demo-2026-10-001";
const SIERRA: (&str, &str, &str) = ("20987654321", "Sierra SRL", "2026-11");

fn andino_key() -> WorkspaceKey {
    demo_workspace().key()
}

fn sierra_key() -> WorkspaceKey {
    WorkspaceKey::new(SIERRA.0, SIERRA.2)
}

/// Andino (active, index 0) and Sierra (index 1), nothing staged.
fn two_empty_workspaces() -> DrenyraHostController {
    let mut controller = DrenyraHostController::new(demo_workspace());
    controller
        .workspaces_mut()
        .add_workspace(FiscalWorkspace::new(SIERRA.0, SIERRA.1, SIERRA.2))
        .expect("distinct workspace");
    controller
}

fn sierra_proposal(id: &str) -> JournalEntryProposal {
    let mut proposal = demo_proposal();
    (proposal.id, proposal.entity_ruc, proposal.fiscal_period) =
        (id.into(), SIERRA.0.into(), SIERRA.2.into());
    proposal
}

fn evidence_total(controller: &DrenyraHostController) -> usize {
    controller.evidence(&andino_key()).len() + controller.evidence(&sierra_key()).len()
}

/// Runs `refused` (which must report that the operation was refused) and asserts the controller
/// is byte-for-byte the same afterwards, evidence included.
fn assert_refused(
    controller: &mut DrenyraHostController,
    label: &str,
    refused: impl FnOnce(&mut DrenyraHostController) -> bool,
) {
    let before = format!("{controller:?}");
    let events = evidence_total(controller);
    assert!(refused(controller), "{label}: the operation was accepted");
    assert_eq!(
        format!("{controller:?}"),
        before,
        "{label}: a refused operation changed the controller"
    );
    assert_eq!(evidence_total(controller), events, "{label}: evidence grew");
}

/// Approver IDs the domain must refuse: blank, hidden characters, over the length cap.
fn invalid_approver_ids() -> Vec<String> {
    let mut ids: Vec<String> = [
        "",
        "   ",
        "\u{200B}",
        "CPA\u{202E}1",
        "\u{FEFF}CPA",
        "CPA\n1",
        "CPA\u{0}1",
        "CPA\u{E0041}",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect();
    ids.push("A".repeat(MAX_APPROVER_ID_CHARS + 1));
    ids
}

/// Every review operation refused on the active workspace, which has no staged review.
fn assert_everything_refused_without_a_review(controller: &mut DrenyraHostController) {
    assert_refused(controller, "approve without review", |c| {
        !c.approve_active("CPA-1")
    });
    assert_refused(controller, "replace without review", |c| {
        c.replace_active_approval("CPA-1", "CPA-2") == Err(ReplaceApprovalError::NothingToReplace)
    });
    assert_refused(controller, "accept without review", |c| {
        !c.accept_active_line(0)
    });
    assert_refused(controller, "reject without review", |c| {
        !c.reject_active_line(0, "wrong account")
    });
    assert_refused(controller, "build without review", |c| {
        c.build_post_request().is_err()
    });
    assert_refused(controller, "confirm unknown id", |c| {
        c.confirm_posted(42).is_err()
    });
}

// 1. Refused operations change nothing.

#[test]
fn every_refused_operation_leaves_the_controller_identical_and_logs_nothing() {
    let mut c = two_empty_workspaces();
    assert_everything_refused_without_a_review(&mut c);
    assert_refused(&mut c, "stage another company's proposal", |c| {
        c.stage_proposal(sierra_proposal("je-x")).is_err()
    });

    c.stage_proposal(demo_proposal()).expect("stage Andino");
    assert_refused(&mut c, "duplicate stage", |c| {
        c.stage_proposal(demo_proposal()).is_err()
    });
    assert_refused(&mut c, "build with pending lines", |c| {
        c.build_post_request().is_err()
    });
    for line in [3, usize::MAX] {
        assert_refused(&mut c, "accept out-of-range line", |c| {
            !c.accept_active_line(line)
        });
        assert_refused(&mut c, "reject out-of-range line", |c| {
            !c.reject_active_line(line, "wrong account")
        });
    }
    for comment in ["", "   ", "\t\n"] {
        assert_refused(&mut c, "reject with a blank comment", |c| {
            !c.reject_active_line(0, comment)
        });
    }
    assert_refused(&mut c, "replace with nothing recorded", |c| {
        c.replace_active_approval("CPA-1", "CPA-2") == Err(ReplaceApprovalError::NothingToReplace)
    });
    for line in 0..3 {
        assert!(c.accept_active_line(line));
    }
    for id in invalid_approver_ids() {
        assert_refused(&mut c, &format!("approve {id:?}"), |c| {
            !c.approve_active(&id)
        });
    }

    assert!(c.approve_active("CPA-1"));
    assert_refused(&mut c, "second approve without replace", |c| {
        !c.approve_active("CPA-2")
    });
    for id in invalid_approver_ids() {
        assert_refused(&mut c, &format!("replace with {id:?}"), |c| {
            c.replace_active_approval("CPA-1", &id) == Err(ReplaceApprovalError::InvalidApprover)
        });
    }
    assert_refused(&mut c, "stale expected approver", |c| {
        c.replace_active_approval("CPA-9", "CPA-2")
            == Err(ReplaceApprovalError::StaleApproval {
                current: "CPA-1".into(),
            })
    });
    for same in ["CPA-1", "  CPA-1 ", "\tCPA-1\n"] {
        assert_refused(&mut c, &format!("same approver {same:?}"), |c| {
            c.replace_active_approval("CPA-1", same) == Err(ReplaceApprovalError::SameApprover)
        });
    }
    assert_refused(&mut c, "confirm unknown id with a review", |c| {
        c.confirm_posted(7).is_err()
    });

    // A frame superseded by a retry can no longer be confirmed.
    c.build_post_request().expect("first frame");
    let superseded = c.pending_post_request().expect("first id in flight");
    c.build_post_request().expect("retry frame");
    assert_ne!(
        c.pending_post_request(),
        Some(superseded),
        "retry gets a new id"
    );
    assert_refused(&mut c, "confirm superseded id", |c| {
        c.confirm_posted(superseded).is_err()
    });

    // The workspace without a review refuses everything, even while Andino has one in flight.
    assert!(c.workspaces_mut().switch_to(1));
    assert_everything_refused_without_a_review(&mut c);

    // A period change after staging blocks building and staging in the old scope.
    assert!(c.workspaces_mut().switch_to(0));
    c.workspaces_mut().active_mut().fiscal_period = "2026-11".into();
    assert_refused(&mut c, "build after a scope change", |c| {
        c.build_post_request().is_err()
    });
    assert_refused(&mut c, "stage after a scope change", |c| {
        c.stage_proposal(demo_proposal()).is_err()
    });
}

// 2. Sequence numbers and ownership.

fn proposal_id(kind: &EvidenceKind) -> &str {
    match kind {
        EvidenceKind::Staged { proposal_id, .. }
        | EvidenceKind::Approved { proposal_id, .. }
        | EvidenceKind::ApprovalReplaced { proposal_id, .. }
        | EvidenceKind::ApprovalCleared { proposal_id, .. }
        | EvidenceKind::PostRequested { proposal_id, .. }
        | EvidenceKind::Posted { proposal_id, .. } => proposal_id,
    }
}

#[test]
fn evidence_seq_is_one_to_n_across_workspaces_and_each_event_lands_under_its_owner() {
    let mut c = two_empty_workspaces();
    c.stage_proposal(demo_proposal()).expect("stage Andino");
    assert!(c.workspaces_mut().switch_to(1));
    c.stage_proposal(sierra_proposal("je-sierra"))
        .expect("stage Sierra");
    for line in 0..3 {
        assert!(c.accept_active_line(line));
    }
    assert!(c.approve_active("CPA-S"));
    c.build_post_request().expect("Sierra frame");
    let sierra_request = c.pending_post_request().expect("Sierra in flight");

    assert!(c.workspaces_mut().switch_to(0));
    for line in 0..3 {
        assert!(c.accept_active_line(line));
    }
    assert!(c.approve_active("CPA-1"));
    assert_eq!(c.replace_active_approval("CPA-1", "CPA-2"), Ok(()));
    assert!(c.reject_active_line(1, "wrong account"));
    assert!(c.accept_active_line(1));
    assert!(c.approve_active("CPA-3"));
    // Sierra's post is confirmed while Andino is active.
    c.confirm_posted(sierra_request).expect("Sierra confirmed");
    c.build_post_request().expect("Andino frame");
    let andino_request = c.pending_post_request().expect("Andino in flight");
    assert!(c.workspaces_mut().switch_to(1));
    // Andino's post is confirmed while Sierra is active.
    c.confirm_posted(andino_request).expect("Andino confirmed");

    let mut all = Vec::new();
    for (key, owned_id) in [(andino_key(), DEMO_ID), (sierra_key(), "je-sierra")] {
        let events = c.evidence(&key);
        assert!(!events.is_empty());
        assert!(
            events.windows(2).all(|pair| pair[0].seq < pair[1].seq),
            "seq strictly increasing within {key:?}"
        );
        for event in events {
            assert_eq!(
                proposal_id(&event.kind),
                owned_id,
                "{event:?} under {key:?}"
            );
        }
        all.extend(events.iter().map(|event| event.seq));
    }
    all.sort_unstable();
    let n = all.len() as u64;
    assert_eq!(all, (1..=n).collect::<Vec<_>>());
    assert_eq!(n, 11, "4 Sierra events + 7 Andino events");
    assert!(matches!(
        c.evidence(&sierra_key()).last().map(|e| &e.kind),
        Some(EvidenceKind::Posted { .. })
    ));
    assert_eq!(c.posted(&andino_key())[0].proposal.id, DEMO_ID);
    assert_eq!(c.posted(&sierra_key())[0].proposal.id, "je-sierra");
}

// 3. A1: the posted record is what the frame carried.

fn approved_and_built(approver: &str) -> (DrenyraHostController, u64) {
    let mut c = demo_controller();
    for line in 0..3 {
        assert!(c.accept_active_line(line));
    }
    assert!(c.approve_active(approver));
    let frame = c.build_post_request().expect("ready to post");
    assert!(frame.contains(&format!("\"approved_by\":\"{approver}\"")));
    let request = c.pending_post_request().expect("in flight");
    (c, request)
}

fn assert_posted_as_sent(c: &DrenyraHostController, request: u64, approver: &str) {
    let posted = c.posted(&andino_key());
    assert_eq!(posted.len(), 1);
    assert_eq!(posted[0].proposal, demo_proposal());
    assert_eq!(posted[0].approved_by.as_deref(), Some(approver));
    assert_eq!(posted[0].request_id, request);
    assert_eq!(
        c.evidence(&andino_key()).last().map(|e| &e.kind),
        Some(&EvidenceKind::Posted {
            proposal_id: DEMO_ID.into(),
            request_id: request,
        })
    );
}

#[test]
fn posted_entry_keeps_the_frame_s_approver_after_a_replacement_in_flight() {
    let (mut c, request) = approved_and_built("CPA-1");
    assert_eq!(c.replace_active_approval("CPA-1", "CPA-2"), Ok(()));
    c.confirm_posted(request).expect("confirmed");
    assert_posted_as_sent(&c, request, "CPA-1");
}

#[test]
fn posted_entry_keeps_the_frame_s_approver_after_a_line_change_in_flight() {
    let (mut c, request) = approved_and_built("CPA-1");
    assert!(c.reject_active_line(0, "wrong account"));
    assert_eq!(
        c.active_review().and_then(|review| review.approved_by()),
        None,
        "the line change cleared the approval"
    );
    c.confirm_posted(request).expect("confirmed");
    assert_posted_as_sent(&c, request, "CPA-1");
}

// 3. A2: overlays block workspace switches; the dialog compares against what it showed.

fn accept_every_line(s: &mut ShellState) {
    for _ in 0..3 {
        s.apply(ShellCommand::AcceptLine, WIDE);
        s.apply(ShellCommand::ReviewCursorDown, WIDE);
    }
}

fn approver(s: &ShellState) -> Option<String> {
    s.controller()
        .active_review()
        .and_then(|review| review.approved_by().map(str::to_owned))
}

#[test]
fn sidebar_switch_is_refused_while_any_overlay_is_open() {
    let overlays = [
        ShellCommand::OpenApproval,
        ShellCommand::OpenPalette,
        ShellCommand::OpenSettings,
    ];
    for open in overlays {
        let mut s = ShellState::demo();
        accept_every_line(&mut s);
        assert!(s.apply(open, WIDE), "{open:?} opens");
        s.type_text("CPA-X");
        let before = format!("{:?}", s.view(WIDE));
        assert!(!s.switch_workspace(1), "{open:?}: switch refused");
        assert_eq!(s.active_workspace(), 0);
        assert_eq!(format!("{:?}", s.view(WIDE)), before, "{open:?}: unchanged");
        assert!(s.apply(ShellCommand::CloseOverlay, WIDE));
        assert!(s.switch_workspace(1), "{open:?}: switch works once closed");
    }
}

#[test]
fn text_typed_in_the_dialog_never_approves_another_workspace_s_entry() {
    let mut c = demo_controller();
    assert!(c.workspaces_mut().switch_to(1));
    c.stage_proposal(sierra_proposal("je-sierra"))
        .expect("stage Sierra");
    for line in 0..3 {
        assert!(c.accept_active_line(line));
    }
    assert!(c.workspaces_mut().switch_to(0));
    let mut s = ShellState::new(c, true);
    accept_every_line(&mut s);
    assert!(s.apply(ShellCommand::OpenApproval, WIDE));
    s.type_text("CPA-ANDINO");
    assert!(!s.switch_workspace(1), "sidebar click refused");
    assert!(s.apply(ShellCommand::Confirm, WIDE));
    assert_eq!(approver(&s).as_deref(), Some("CPA-ANDINO"));
    assert!(s.switch_workspace(1));
    assert_eq!(approver(&s), None, "Sierra unaffected");
}

#[test]
fn palette_switch_entries_still_work_because_the_palette_closes_first() {
    let mut s = ShellState::demo();
    assert!(s.apply(ShellCommand::OpenPalette, WIDE));
    s.type_text("Switch to Sierra");
    assert!(s.apply(ShellCommand::Confirm, WIDE));
    assert_eq!(s.active_workspace(), 1);
    assert!(s.view(WIDE).palette.is_none());
}

const STALE_CPA_2: &str =
    "The approval changed to CPA-2 while this dialog was open. Close it and review again.";

/// Confirms `typed` in the open dialog and asserts it was refused with `STALE_CPA_2`, leaving
/// the controller and the dialog's input as they were.
fn assert_confirm_refused_as_stale(s: &mut ShellState, typed: &str) {
    s.type_text(typed);
    let before = format!("{:?}", s.controller());
    assert!(s.apply(ShellCommand::Confirm, WIDE));
    let dialog = s.view(WIDE).approval.expect("stays open");
    assert_eq!(dialog.error.as_deref(), Some(STALE_CPA_2));
    assert_eq!(dialog.approver, typed);
    assert_eq!(format!("{:?}", s.controller()), before, "nothing changed");
    assert_eq!(approver(s).as_deref(), Some("CPA-2"));
}

#[test]
fn dialog_refuses_when_the_shown_approver_was_replaced_meanwhile() {
    let mut s = ShellState::demo();
    accept_every_line(&mut s);
    assert!(s.controller_mut().approve_active("CPA-1"));
    assert!(s.apply(ShellCommand::OpenApproval, WIDE));
    assert_eq!(
        s.view(WIDE)
            .approval
            .expect("open")
            .current_approver
            .as_deref(),
        Some("CPA-1")
    );
    assert_eq!(
        s.controller_mut().replace_active_approval("CPA-1", "CPA-2"),
        Ok(())
    );
    assert_confirm_refused_as_stale(&mut s, "CPA-3");
}

#[test]
fn dialog_opened_without_an_approval_refuses_when_one_appeared_meanwhile() {
    let mut s = ShellState::demo();
    accept_every_line(&mut s);
    assert!(s.apply(ShellCommand::OpenApproval, WIDE));
    assert!(s.controller_mut().approve_active("CPA-2"));
    assert_confirm_refused_as_stale(&mut s, "CPA-3");
}

#[test]
fn dialog_refuses_with_guidance_when_the_shown_approval_was_cleared_meanwhile() {
    let mut s = ShellState::demo();
    accept_every_line(&mut s);
    assert!(s.controller_mut().approve_active("CPA-1"));
    assert!(s.apply(ShellCommand::OpenApproval, WIDE));
    // A line change behind the dialog clears the approval; re-accepting unblocks approval.
    assert!(s.controller_mut().reject_active_line(0, "check"));
    assert!(s.controller_mut().accept_active_line(0));
    let evidence_before = s.controller().evidence(&s.active_workspace_key()).len();

    assert!(s.type_text("CPA-3"));
    assert!(s.apply(ShellCommand::Confirm, WIDE));
    let dialog = s.view(WIDE).approval.expect("stays open");
    assert_eq!(
        dialog.error.as_deref(),
        Some(APPROVAL_CLEARED_MESSAGE),
        "cleared approval gets the stale-approval guidance, not a raw domain error"
    );
    assert_eq!(
        s.controller()
            .active_review()
            .and_then(|review| review.approved_by()),
        None
    );
    assert_eq!(
        s.controller().evidence(&s.active_workspace_key()).len(),
        evidence_before,
        "nothing logged"
    );
}
