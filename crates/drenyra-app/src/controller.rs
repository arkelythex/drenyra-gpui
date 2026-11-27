//! Unified host controller coordinating workspaces, accounting reviews, reconciliation, and RPC frames.

use std::collections::BTreeMap;
use std::fmt;

use drenyra_protocol::{escape_json_str, JournalEntryProposal, JsonRpcRequest, ProtocolError};
use drenyra_ui::{EntryDiffReview, ReconciliationTable};
use drenyra_workspace::{FiscalWorkspace, WorkspaceActivity, WorkspaceManager};

/// Errors produced by [`DrenyraHostController`] when gating or framing host operations.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ControllerError {
    NoStagedReview,
    ReviewNotReadyToPost,
    ReviewAlreadyStaged,
    ProposalScopeMismatch,
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
            | Self::ProposalScopeMismatch => None,
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
    reconciliation: ReconciliationTable,
    next_request_id: u64,
}

impl DrenyraHostController {
    pub fn new(initial_workspace: FiscalWorkspace) -> Self {
        Self {
            workspaces: WorkspaceManager::new(initial_workspace),
            reviews: BTreeMap::new(),
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
        let active = self.workspaces.active();
        if proposal.entity_ruc != active.entity_ruc
            || proposal.fiscal_period != active.fiscal_period
        {
            return Err(ControllerError::ProposalScopeMismatch);
        }
        if self.reviews.contains_key(&index) {
            return Err(ControllerError::ReviewAlreadyStaged);
        }

        self.reviews.insert(index, EntryDiffReview::new(proposal));
        let active = self.workspaces.active_mut();
        active.activity = WorkspaceActivity::AwaitingApproval;
        active.guardrails.unposted_entries = 1;
        Ok(())
    }

    /// Staged review of the active workspace, if any.
    pub fn active_review(&self) -> Option<&EntryDiffReview> {
        self.reviews.get(&self.workspaces.active_index())
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

    /// Builds the `ledger_post` NDJSON frame for the active workspace's approved review and
    /// clears it. On error, the review and workspace state are left untouched.
    pub fn build_post_request(&mut self) -> Result<String, ControllerError> {
        let review = self
            .active_review()
            .ok_or(ControllerError::NoStagedReview)?;
        if !review.can_post() {
            return Err(ControllerError::ReviewNotReadyToPost);
        }

        let proposal = review.proposal();
        let params_json = format!(
            r#"{{"name":"ledger_post","arguments":{{"proposal_id":"{}","entity_ruc":"{}","fiscal_period":"{}","approved_by":"{}"}}}}"#,
            escape_json_str(&proposal.id),
            escape_json_str(&proposal.entity_ruc),
            escape_json_str(&proposal.fiscal_period),
            escape_json_str(review.approved_by().unwrap_or("")),
        );

        let request = JsonRpcRequest {
            id: self.next_request_id,
            method: "tools/call".into(),
            params_json,
        };
        let frame = request.to_ndjson()?;

        self.next_request_id = self.next_request_id.saturating_add(1);
        self.reviews.remove(&self.workspaces.active_index());
        let active = self.workspaces.active_mut();
        active.guardrails.unposted_entries = 0;
        active.activity = WorkspaceActivity::Idle;

        Ok(frame)
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

        // Workspace state resets after post.
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
        let active_a = controller.workspaces().active();
        assert_eq!(active_a.activity, WorkspaceActivity::Idle);
        assert_eq!(active_a.guardrails.unposted_entries, 0);
        assert_eq!(
            controller.workspaces().background_alert(),
            WorkspaceActivity::Idle
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
}
