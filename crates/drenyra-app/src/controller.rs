//! Unified host controller coordinating workspaces, accounting reviews, reconciliation, and RPC frames.

use std::collections::BTreeMap;
use std::fmt;

use drenyra_protocol::{
    escape_json_str, JournalEntryProposal, JsonRpcRequest, ProtocolError, RiskTier,
};
use drenyra_ui::{EntryDiffReview, ReconciliationTable};
use drenyra_workspace::{FiscalWorkspace, WorkspaceActivity, WorkspaceManager};

/// Errors produced by [`DrenyraHostController`] when gating or framing host operations.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ControllerError {
    NoStagedReview,
    ReviewNotReadyToPost,
    ReviewAlreadyStaged,
    ProposalScopeMismatch,
    UnknownPostRequest,
    Protocol(ProtocolError),
}

impl fmt::Display for ControllerError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoStagedReview => {
                f.write_str("no journal entry proposal is currently staged for review")
            }
            Self::ReviewNotReadyToPost => {
                f.write_str("staged journal entry review is not approved and balanced for posting")
            }
            Self::ReviewAlreadyStaged => {
                f.write_str("the active workspace already has an unposted journal entry review")
            }
            Self::ProposalScopeMismatch => f.write_str(
                "journal entry proposal RUC or fiscal period does not match the active workspace",
            ),
            Self::UnknownPostRequest => {
                f.write_str("no in-flight ledger_post request matches the confirmed request id")
            }
            Self::Protocol(err) => write!(f, "protocol framing error: {err}"),
        }
    }
}

impl std::error::Error for ControllerError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Protocol(err) => Some(err),
            Self::NoStagedReview
            | Self::ReviewNotReadyToPost
            | Self::ReviewAlreadyStaged
            | Self::ProposalScopeMismatch
            | Self::UnknownPostRequest => None,
        }
    }
}

impl From<ProtocolError> for ControllerError {
    fn from(err: ProtocolError) -> Self {
        Self::Protocol(err)
    }
}

/// Unified controller uniting [`WorkspaceManager`], [`EntryDiffReview`], [`ReconciliationTable`],
/// and validated JSON-RPC 2.0 request emission.
///
/// Reviews are tenant-scoped: each workspace (entity RUC + fiscal period) owns at most one
/// staged review, which is only visible and postable while that workspace is active.
#[derive(Debug, Clone)]
pub struct DrenyraHostController {
    workspaces: WorkspaceManager,
    reviews: BTreeMap<usize, EntryDiffReview>,
    /// In-flight `ledger_post` request id per workspace index, awaiting transport confirmation.
    pending_posts: BTreeMap<usize, u64>,
    reconciliation: ReconciliationTable,
    next_request_id: u64,
}

impl DrenyraHostController {
    pub fn new(initial_workspace: FiscalWorkspace) -> Self {
        Self {
            workspaces: WorkspaceManager::new(initial_workspace),
            reviews: BTreeMap::new(),
            pending_posts: BTreeMap::new(),
            reconciliation: ReconciliationTable::default(),
            next_request_id: 1,
        }
    }

    pub fn workspaces(&self) -> &WorkspaceManager {
        &self.workspaces
    }

    pub fn workspaces_mut(&mut self) -> &mut WorkspaceManager {
        &mut self.workspaces
    }

    /// Stages `proposal` for human review in the active workspace.
    ///
    /// Rejects proposals whose RUC or fiscal period differ from the active workspace, and
    /// refuses to replace a review that has not been posted yet.
    pub fn stage_proposal(
        &mut self,
        proposal: JournalEntryProposal,
    ) -> Result<(), ControllerError> {
        let index = self.workspaces.active_index();
        self.ensure_proposal_matches_active_workspace(&proposal)?;
        if self.reviews.contains_key(&index) {
            return Err(ControllerError::ReviewAlreadyStaged);
        }

        let risk_tier = proposal.risk_tier;
        self.reviews.insert(index, EntryDiffReview::new(proposal));
        let active = self.workspaces.active_mut();
        active.activity = WorkspaceActivity::AwaitingApproval;
        active.guardrails.unposted_entries = 1;
        active.guardrails.active_risk_tier = risk_tier;
        Ok(())
    }

    /// Staged review of the active workspace, if any.
    pub fn active_review(&self) -> Option<&EntryDiffReview> {
        self.reviews.get(&self.workspaces.active_index())
    }

    /// Staged review of the workspace at `index`, read-only (for overviews such as the sidebar).
    pub fn review(&self, index: usize) -> Option<&EntryDiffReview> {
        self.reviews.get(&index)
    }

    pub fn active_review_mut(&mut self) -> Option<&mut EntryDiffReview> {
        self.reviews.get_mut(&self.workspaces.active_index())
    }

    pub fn set_reconciliation(&mut self, table: ReconciliationTable) {
        self.reconciliation = table;
    }

    pub fn reconciliation(&self) -> &ReconciliationTable {
        &self.reconciliation
    }

    fn ensure_proposal_matches_active_workspace(
        &self,
        proposal: &JournalEntryProposal,
    ) -> Result<(), ControllerError> {
        let active = self.workspaces.active();
        if proposal.entity_ruc == active.entity_ruc
            && proposal.fiscal_period == active.fiscal_period
        {
            Ok(())
        } else {
            Err(ControllerError::ProposalScopeMismatch)
        }
    }

    /// Builds the `ledger_post` NDJSON frame for the active workspace's approved review.
    ///
    /// Building does not post: the review stays staged and the workspace stays
    /// `AwaitingApproval` until [`Self::confirm_posted`] is called with the frame's request id
    /// after the transport succeeds, so a failed send can be retried without a new approval.
    /// Each call (including a retry) uses a fresh request id that replaces the pending one.
    ///
    /// A retry re-sends the same `proposal_id`, so the `ledger_post` server must treat
    /// `proposal_id` as its idempotency key: a send whose confirmation was lost must not post
    /// the entry twice.
    ///
    /// Fails with [`ControllerError::ProposalScopeMismatch`] if the active workspace's RUC or
    /// fiscal period changed since staging. On error, all state is left untouched.
    pub fn build_post_request(&mut self) -> Result<String, ControllerError> {
        let review = self
            .active_review()
            .ok_or(ControllerError::NoStagedReview)?;
        // Re-check scope: workspace identity is mutable (e.g. period roll-over) after staging.
        self.ensure_proposal_matches_active_workspace(review.proposal())?;
        if !review.can_post() {
            return Err(ControllerError::ReviewNotReadyToPost);
        }

        let proposal = review.proposal();
        // Tiers that need no human approval emit an explicit `null`, never an empty string.
        let approved_by = review.approved_by().map_or_else(
            || "null".to_owned(),
            |who| format!("\"{}\"", escape_json_str(who)),
        );
        let params_json = format!(
            r#"{{"name":"ledger_post","arguments":{{"proposal_id":"{}","entity_ruc":"{}","fiscal_period":"{}","approved_by":{}}}}}"#,
            escape_json_str(&proposal.id),
            escape_json_str(&proposal.entity_ruc),
            escape_json_str(&proposal.fiscal_period),
            approved_by,
        );

        let request = JsonRpcRequest {
            id: self.next_request_id,
            method: "tools/call".into(),
            params_json,
        };
        let frame = request.to_ndjson()?;

        self.pending_posts
            .insert(self.workspaces.active_index(), request.id);
        self.next_request_id = self.next_request_id.saturating_add(1);
        Ok(frame)
    }

    /// Request id of the active workspace's in-flight `ledger_post`, if a frame was built and
    /// not yet confirmed.
    pub fn pending_post_request(&self) -> Option<u64> {
        self.pending_posts
            .get(&self.workspaces.active_index())
            .copied()
    }

    /// Records that the transport confirmed the `ledger_post` request `request_id`, clearing the
    /// review and guardrails of the workspace that built it, even if another workspace is active.
    ///
    /// Fails with [`ControllerError::UnknownPostRequest`] for ids that are not in flight, such as
    /// one superseded by a retry or already confirmed.
    pub fn confirm_posted(&mut self, request_id: u64) -> Result<(), ControllerError> {
        let index = self
            .pending_posts
            .iter()
            .find_map(|(&index, &pending)| (pending == request_id).then_some(index))
            .ok_or(ControllerError::UnknownPostRequest)?;

        self.pending_posts.remove(&index);
        self.reviews.remove(&index);
        if let Some(owner) = self.workspaces.get_mut(index) {
            owner.guardrails.unposted_entries = 0;
            owner.guardrails.active_risk_tier = RiskTier::R0Read;
            owner.activity = WorkspaceActivity::Idle;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use drenyra_protocol::{validate_json_container, JournalLine, MoneyCents, RiskTier};
    use drenyra_ui::{ReconciliationRow, ReconciliationStatus};

    fn sample_proposal(id: &str, risk_tier: RiskTier, balanced: bool) -> JournalEntryProposal {
        let tax = if balanced { 1_800 } else { 1_799 };
        JournalEntryProposal {
            id: id.into(),
            entity_ruc: "20123456789".into(),
            fiscal_period: "2026-10".into(),
            risk_tier,
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
                    credit: MoneyCents::from_cents(tax),
                    description: "IGV 18%".into(),
                },
            ],
        }
    }

    #[test]
    fn build_post_request_requires_staged_review() {
        let ws = FiscalWorkspace::new("20123456789", "Andino SAC", "2026-10");
        let mut controller = DrenyraHostController::new(ws);

        assert_eq!(
            controller.build_post_request(),
            Err(ControllerError::NoStagedReview)
        );
        assert_eq!(
            controller.workspaces().active().activity,
            WorkspaceActivity::Idle
        );
        assert_eq!(
            controller.workspaces().active().guardrails.unposted_entries,
            0
        );
    }

    #[test]
    fn build_post_request_is_blocked_before_full_acceptance_and_human_approval() {
        let ws = FiscalWorkspace::new("20123456789", "Andino SAC", "2026-10");
        let mut controller = DrenyraHostController::new(ws);
        controller
            .stage_proposal(sample_proposal("je-001", RiskTier::R2Material, true))
            .expect("proposal matches workspace");

        assert!(controller.active_review().is_some());
        assert_eq!(
            controller.workspaces().active().activity,
            WorkspaceActivity::AwaitingApproval
        );
        assert_eq!(
            controller.workspaces().active().guardrails.unposted_entries,
            1
        );

        // Blocked while lines are still pending.
        assert_eq!(
            controller.build_post_request(),
            Err(ControllerError::ReviewNotReadyToPost)
        );

        // Still blocked after accepting all lines without human approval for R2Material.
        controller
            .active_review_mut()
            .expect("staged review")
            .accept_all();
        assert_eq!(
            controller.build_post_request(),
            Err(ControllerError::ReviewNotReadyToPost)
        );

        // State must remain staged and unposted after failed post attempt.
        assert!(controller.active_review().is_some());
        assert_eq!(
            controller.workspaces().active().activity,
            WorkspaceActivity::AwaitingApproval
        );
        assert_eq!(
            controller.workspaces().active().guardrails.unposted_entries,
            1
        );
    }

    #[test]
    fn build_post_request_emits_ndjson_and_resets_workspace_after_post() {
        let ws = FiscalWorkspace::new("20123456789", "Andino SAC", "2026-10");
        let mut controller = DrenyraHostController::new(ws);
        controller
            .stage_proposal(sample_proposal("je-001", RiskTier::R2Material, true))
            .expect("proposal matches workspace");

        {
            let review = controller.active_review_mut().expect("staged review");
            review.accept_all();
            assert!(review.record_human_approval("CPA-PERU-001"));
        }

        let frame = controller
            .build_post_request()
            .expect("approved balanced entry should build post frame");
        assert!(!frame.contains('\n') && !frame.contains('\r'));
        assert_eq!(validate_json_container(&frame), Ok(()));
        assert!(frame.contains(r#""jsonrpc":"2.0""#));
        assert!(frame.contains(r#""id":1"#));
        assert!(frame.contains(r#""method":"tools/call""#));
        assert!(frame.contains(r#""name":"ledger_post""#));
        assert!(frame.contains(r#""proposal_id":"je-001""#));
        assert!(frame.contains(r#""entity_ruc":"20123456789""#));
        assert!(frame.contains(r#""fiscal_period":"2026-10""#));
        assert!(frame.contains(r#""approved_by":"CPA-PERU-001""#));

        // Building the frame does not post: the review stays staged until the transport confirms.
        assert!(controller.active_review().is_some());
        assert_eq!(
            controller.workspaces().active().activity,
            WorkspaceActivity::AwaitingApproval
        );
        assert_eq!(
            controller.workspaces().active().guardrails.unposted_entries,
            1
        );

        assert_eq!(controller.pending_post_request(), Some(1));

        // Workspace state resets after the transport confirms the post.
        assert_eq!(controller.confirm_posted(1), Ok(()));
        assert_eq!(controller.pending_post_request(), None);
        assert!(controller.active_review().is_none());
        assert_eq!(
            controller.workspaces().active().activity,
            WorkspaceActivity::Idle
        );
        assert_eq!(
            controller.workspaces().active().guardrails.unposted_entries,
            0
        );
        assert_eq!(
            controller.build_post_request(),
            Err(ControllerError::NoStagedReview)
        );

        // Next request increments request ID.
        controller
            .stage_proposal(sample_proposal("je-002", RiskTier::R1Draft, true))
            .expect("proposal matches workspace");
        controller
            .active_review_mut()
            .expect("staged second review")
            .accept_all();
        let second_frame = controller
            .build_post_request()
            .expect("r1 draft posts after all lines accepted");
        assert!(second_frame.contains(r#""id":2"#));
        assert!(second_frame.contains(r#""proposal_id":"je-002""#));
        // R1 drafts need no human approval: the frame says so explicitly with `null`.
        assert!(second_frame.contains(r#""approved_by":null"#));
        assert_eq!(validate_json_container(&second_frame), Ok(()));
    }

    #[test]
    fn confirm_posted_rejects_unknown_or_already_confirmed_request_ids() {
        let ws = FiscalWorkspace::new("20123456789", "Andino SAC", "2026-10");
        let mut controller = DrenyraHostController::new(ws);
        assert_eq!(
            controller.confirm_posted(1),
            Err(ControllerError::UnknownPostRequest)
        );

        controller
            .stage_proposal(sample_proposal("je-001", RiskTier::R1Draft, true))
            .expect("proposal matches workspace");
        controller
            .active_review_mut()
            .expect("staged review")
            .accept_all();
        controller.build_post_request().expect("first attempt");
        // A retry after a failed send replaces the pending request id.
        let retry = controller.build_post_request().expect("retry attempt");
        assert!(retry.contains(r#""id":2"#));

        assert_eq!(
            controller.confirm_posted(1),
            Err(ControllerError::UnknownPostRequest)
        );
        assert!(controller.active_review().is_some());
        assert_eq!(controller.confirm_posted(2), Ok(()));
        assert!(controller.active_review().is_none());
        assert_eq!(
            controller.confirm_posted(2),
            Err(ControllerError::UnknownPostRequest)
        );
        assert_eq!(
            ControllerError::UnknownPostRequest.to_string(),
            "no in-flight ledger_post request matches the confirmed request id"
        );
    }

    #[test]
    fn confirm_posted_clears_the_owning_workspace_even_after_a_switch() {
        let ws_a = FiscalWorkspace::new("20123456789", "Andino SAC", "2026-10");
        let ws_b = FiscalWorkspace::new("20987654321", "Sierra SRL", "2026-11");
        let mut controller = DrenyraHostController::new(ws_a);
        let idx_b = controller.workspaces_mut().add_workspace(ws_b);
        controller
            .stage_proposal(sample_proposal("je-a", RiskTier::R1Draft, true))
            .expect("proposal matches workspace A");
        controller
            .active_review_mut()
            .expect("staged review")
            .accept_all();
        controller.build_post_request().expect("frame for A");

        assert!(controller.workspaces_mut().switch_to(idx_b));
        assert_eq!(controller.confirm_posted(1), Ok(()));
        assert_eq!(
            controller.workspaces().background_alert(),
            WorkspaceActivity::Idle
        );
        let active_b = controller.workspaces().active();
        assert_eq!(active_b.entity_ruc, "20987654321");
        assert_eq!(active_b.guardrails.unposted_entries, 0);

        assert!(controller.workspaces_mut().switch_to(0));
        assert!(controller.active_review().is_none());
        let active_a = controller.workspaces().active();
        assert_eq!(active_a.activity, WorkspaceActivity::Idle);
        assert_eq!(active_a.guardrails.unposted_entries, 0);
    }

    #[test]
    fn unbalanced_proposal_is_rejected_even_when_accepted_and_approved() {
        let ws = FiscalWorkspace::new("20123456789", "Andino SAC", "2026-10");
        let mut controller = DrenyraHostController::new(ws);
        controller
            .stage_proposal(sample_proposal("je-bad", RiskTier::R2Material, false))
            .expect("proposal matches workspace");

        {
            let review = controller.active_review_mut().expect("staged review");
            review.accept_all();
            assert!(review.record_human_approval("CPA-PERU-001"));
        }

        assert_eq!(
            controller.build_post_request(),
            Err(ControllerError::ReviewNotReadyToPost)
        );
    }

    #[test]
    fn escaped_fields_produce_valid_single_line_ndjson() {
        let ws = FiscalWorkspace::new("20123456789", "Andino SAC", "2026-10");
        let mut controller = DrenyraHostController::new(ws);
        controller
            .stage_proposal(sample_proposal(
                "je-\"quoted\"\\line\n1",
                RiskTier::R2Material,
                true,
            ))
            .expect("proposal matches workspace");
        {
            let review = controller.active_review_mut().expect("staged review");
            review.accept_all();
            assert!(review.record_human_approval("CPA\\\"01\nPERU"));
        }

        let frame = controller
            .build_post_request()
            .expect("escaped frame should be valid single-line NDJSON");
        assert!(!frame.contains('\n') && !frame.contains('\r'));
        assert_eq!(validate_json_container(&frame), Ok(()));
    }

    #[test]
    fn stage_proposal_rejects_proposal_scoped_to_another_entity_or_period() {
        let ws = FiscalWorkspace::new("20123456789", "Andino SAC", "2026-10");
        let mut controller = DrenyraHostController::new(ws);

        let mut foreign_ruc = sample_proposal("je-x", RiskTier::R1Draft, true);
        foreign_ruc.entity_ruc = "20987654321".into();
        assert_eq!(
            controller.stage_proposal(foreign_ruc),
            Err(ControllerError::ProposalScopeMismatch)
        );

        let mut foreign_period = sample_proposal("je-y", RiskTier::R1Draft, true);
        foreign_period.fiscal_period = "2026-11".into();
        assert_eq!(
            controller.stage_proposal(foreign_period),
            Err(ControllerError::ProposalScopeMismatch)
        );

        assert!(controller.active_review().is_none());
        let active = controller.workspaces().active();
        assert_eq!(active.activity, WorkspaceActivity::Idle);
        assert_eq!(active.guardrails.unposted_entries, 0);
        assert_eq!(
            ControllerError::ProposalScopeMismatch.to_string(),
            "journal entry proposal RUC or fiscal period does not match the active workspace"
        );
    }

    #[test]
    fn stage_proposal_refuses_to_overwrite_an_unposted_review() {
        let ws = FiscalWorkspace::new("20123456789", "Andino SAC", "2026-10");
        let mut controller = DrenyraHostController::new(ws);
        controller
            .stage_proposal(sample_proposal("je-001", RiskTier::R1Draft, true))
            .expect("first proposal stages");

        assert_eq!(
            controller.stage_proposal(sample_proposal("je-002", RiskTier::R1Draft, true)),
            Err(ControllerError::ReviewAlreadyStaged)
        );
        assert_eq!(
            controller.active_review().map(|r| r.proposal().id.as_str()),
            Some("je-001")
        );
        assert_eq!(
            ControllerError::ReviewAlreadyStaged.to_string(),
            "the active workspace already has an unposted journal entry review"
        );
    }

    #[test]
    fn staged_review_stays_scoped_to_its_workspace_across_switches() {
        let ws_a = FiscalWorkspace::new("20123456789", "Andino SAC", "2026-10");
        let ws_b = FiscalWorkspace::new("20987654321", "Sierra SRL", "2026-11");
        let mut controller = DrenyraHostController::new(ws_a);
        let idx_b = controller.workspaces_mut().add_workspace(ws_b);

        controller
            .stage_proposal(sample_proposal("je-a", RiskTier::R2Material, true))
            .expect("proposal matches workspace A");
        {
            let review = controller.active_review_mut().expect("staged review");
            review.accept_all();
            assert!(review.record_human_approval("CPA-PERU-001"));
        }

        assert!(controller.workspaces_mut().switch_to(idx_b));
        assert!(controller.active_review().is_none());
        assert_eq!(
            controller.build_post_request(),
            Err(ControllerError::NoStagedReview)
        );
        assert_eq!(
            controller.workspaces().background_alert(),
            WorkspaceActivity::AwaitingApproval
        );
        let active_b = controller.workspaces().active();
        assert_eq!(active_b.activity, WorkspaceActivity::Idle);
        assert_eq!(active_b.guardrails.unposted_entries, 0);

        assert!(controller.workspaces_mut().switch_to(0));
        let frame = controller
            .build_post_request()
            .expect("workspace A posts its own approved review");
        assert!(frame.contains(r#""entity_ruc":"20123456789""#));
        assert_eq!(controller.confirm_posted(1), Ok(()));
        let active_a = controller.workspaces().active();
        assert_eq!(active_a.activity, WorkspaceActivity::Idle);
        assert_eq!(active_a.guardrails.unposted_entries, 0);
        assert_eq!(
            controller.workspaces().background_alert(),
            WorkspaceActivity::Idle
        );
    }

    #[test]
    fn build_post_request_rejects_review_when_active_workspace_scope_changed() {
        let ws = FiscalWorkspace::new("20123456789", "Andino SAC", "2026-10");
        let mut controller = DrenyraHostController::new(ws);
        controller
            .stage_proposal(sample_proposal("je-001", RiskTier::R2Material, true))
            .expect("proposal matches workspace");
        {
            let review = controller.active_review_mut().expect("staged review");
            review.accept_all();
            assert!(review.record_human_approval("CPA-PERU-001"));
        }

        controller.workspaces_mut().active_mut().fiscal_period = "2026-11".into();
        assert_eq!(
            controller.build_post_request(),
            Err(ControllerError::ProposalScopeMismatch)
        );
        assert!(controller.active_review().is_some());
        let active = controller.workspaces().active();
        assert_eq!(active.guardrails.unposted_entries, 1);
        assert_eq!(active.activity, WorkspaceActivity::AwaitingApproval);
    }

    #[test]
    fn active_risk_tier_follows_staged_review_until_post_is_confirmed() {
        let ws = FiscalWorkspace::new("20123456789", "Andino SAC", "2026-10");
        let mut controller = DrenyraHostController::new(ws);
        assert_eq!(
            controller.workspaces().active().guardrails.active_risk_tier,
            RiskTier::R0Read
        );

        controller
            .stage_proposal(sample_proposal("je-001", RiskTier::R2Material, true))
            .expect("proposal matches workspace");
        assert_eq!(
            controller.workspaces().active().guardrails.active_risk_tier,
            RiskTier::R2Material
        );

        {
            let review = controller.active_review_mut().expect("staged review");
            review.accept_all();
            assert!(review.record_human_approval("CPA-PERU-001"));
        }
        controller.build_post_request().expect("frame");
        assert_eq!(
            controller.workspaces().active().guardrails.active_risk_tier,
            RiskTier::R2Material,
            "still gated while the post is in flight"
        );
        assert_eq!(controller.confirm_posted(1), Ok(()));
        assert_eq!(
            controller.workspaces().active().guardrails.active_risk_tier,
            RiskTier::R0Read
        );
    }

    #[test]
    fn reconciliation_and_workspace_switching_coordinate_cleanly() {
        let ws1 = FiscalWorkspace::new("20123456789", "Andino SAC", "2026-10");
        let ws2 = FiscalWorkspace::new("20987654321", "Sierra SRL", "2026-11");
        let mut controller = DrenyraHostController::new(ws1);
        let idx2 = controller.workspaces_mut().add_workspace(ws2);
        assert!(controller.workspaces_mut().switch_to(idx2));
        assert_eq!(controller.workspaces().active().entity_ruc, "20987654321");

        let table = ReconciliationTable::new(vec![ReconciliationRow {
            id: "rec-1".into(),
            reference: "BCP-100".into(),
            bank_amount: MoneyCents::from_cents(15_000),
            ledger_amount: MoneyCents::from_cents(12_000),
            status: ReconciliationStatus::Pending,
        }]);
        controller.set_reconciliation(table);
        assert_eq!(
            controller.reconciliation().total_unmatched_delta(),
            Some(MoneyCents::from_cents(3_000))
        );
    }

    #[test]
    fn edit_after_built_frame_blocks_rebuild_until_reapproved() {
        let ws = FiscalWorkspace::new("20123456789", "Andino SAC", "2026-10");
        let mut controller = DrenyraHostController::new(ws);
        controller
            .stage_proposal(sample_proposal("je-001", RiskTier::R3Irreversible, true))
            .expect("stage");
        {
            let review = controller.active_review_mut().expect("staged");
            review.accept_all();
            assert!(review.record_human_approval("CPA-1"));
        }
        let first = controller.build_post_request().expect("approved frame");
        assert!(first.contains(r#""id":1"#));
        {
            let review = controller.active_review_mut().expect("staged");
            assert!(review.reject_line(0, "wrong"));
            assert!(review.accept_line(0));
        }
        assert!(matches!(
            controller.build_post_request(),
            Err(ControllerError::ReviewNotReadyToPost)
        ));
        // Failed rebuild leaves the in-flight request untouched.
        assert_eq!(controller.pending_post_request(), Some(1));
        {
            let review = controller.active_review_mut().expect("staged");
            assert!(review.record_human_approval("CPA-2"));
        }
        let retry = controller.build_post_request().expect("re-approved frame");
        assert!(retry.contains(r#""approved_by":"CPA-2""#));
        assert!(retry.contains(r#""id":2"#));
        assert!(matches!(
            controller.confirm_posted(1),
            Err(ControllerError::UnknownPostRequest)
        ));
        controller.confirm_posted(2).expect("confirm retry");
        assert!(controller.active_review().is_none());
    }
}
