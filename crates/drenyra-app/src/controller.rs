//! Unified host controller coordinating workspaces, accounting reviews, reconciliation, and RPC frames.

use std::collections::BTreeMap;
use std::fmt;
use std::time::{SystemTime, UNIX_EPOCH};

use drenyra_protocol::{
    escape_json_str, JournalEntryProposal, JsonRpcRequest, ProtocolError, RiskTier,
};
use drenyra_ui::{EntryDiffReview, ReconciliationTable, ReplaceApprovalError};
use drenyra_workspace::{FiscalWorkspace, WorkspaceActivity, WorkspaceKey, WorkspaceManager};

/// What happened to a staged journal entry, as recorded in the session evidence log.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EvidenceKind {
    /// The proposal was staged for human review.
    Staged {
        proposal_id: String,
        risk_tier: RiskTier,
    },
    /// A human approval was recorded where none existed.
    Approved {
        proposal_id: String,
        approver: String,
    },
    /// An existing approval was explicitly replaced by a different approver.
    ApprovalReplaced {
        proposal_id: String,
        previous: String,
        approver: String,
    },
    /// A line decision changed, which cleared the recorded approval.
    ApprovalCleared {
        proposal_id: String,
        previous: String,
    },
    /// A `ledger_post` frame was built (one event per frame, retries included).
    PostRequested {
        proposal_id: String,
        request_id: u64,
    },
    /// The transport confirmed the `ledger_post` request.
    Posted {
        proposal_id: String,
        request_id: u64,
    },
}

/// One entry of the session evidence log.
///
/// `seq` is global to the controller (shared by every workspace), starts at 1 and increases by
/// one per event, so events from different workspaces can be ordered against each other.
/// `recorded_at_ms` is when the event was recorded, in Unix milliseconds (UTC), read once from
/// the controller's [`Clock`]. Order events by `seq`: the system wall clock can step backwards.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EvidenceEvent {
    pub seq: u64,
    pub recorded_at_ms: u64,
    pub kind: EvidenceKind,
}

/// Where the controller reads the time it stamps on evidence events (Unix milliseconds, UTC).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Clock {
    /// The operating system's wall clock (not monotonic). A time before 1970 reads as 0 and one
    /// beyond `u64::MAX` milliseconds as `u64::MAX`.
    #[default]
    System,
    /// Deterministic clock: the first reading is `next_ms`, each reading then advances it by
    /// `step_ms`, saturating at `u64::MAX`. Build it with [`Clock::fixed`].
    Fixed { next_ms: u64, step_ms: u64 },
}

impl Clock {
    /// A deterministic clock that reads `start_ms` first and then advances by `step_ms` per read.
    pub const fn fixed(start_ms: u64, step_ms: u64) -> Self {
        Self::Fixed {
            next_ms: start_ms,
            step_ms,
        }
    }

    /// Current time in Unix milliseconds (UTC); a fixed clock advances by its step.
    pub fn now_ms(&mut self) -> u64 {
        match self {
            Self::System => SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_or(0, |elapsed| {
                    u64::try_from(elapsed.as_millis()).unwrap_or(u64::MAX)
                }),
            Self::Fixed { next_ms, step_ms } => {
                let now = *next_ms;
                *next_ms = now.saturating_add(*step_ms);
                now
            }
        }
    }
}

/// A journal entry whose `ledger_post` the transport confirmed in this session, exactly as the
/// confirmed frame carried it (proposal and approver captured when the frame was built).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PostedEntry {
    pub proposal: JournalEntryProposal,
    /// `None` for tiers that need no human approval.
    pub approved_by: Option<String>,
    pub request_id: u64,
}

/// An in-flight `ledger_post`: its request id and exactly what the frame carried, so the posted
/// record matches what was sent even if the review changes before the transport confirms.
#[derive(Debug, Clone, PartialEq, Eq)]
struct PendingPost {
    request_id: u64,
    proposal: JournalEntryProposal,
    approved_by: Option<String>,
}

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
///
/// Every review mutation that matters for audit (staging, approving, replacing or clearing an
/// approval, building and confirming a post) goes through this controller and appends to an
/// append-only evidence log keyed by the owning workspace's RUC and period; each event carries
/// a sequence number and the UTC time read from the controller's [`Clock`]. The log and the
/// posted-entry list are **in memory and session-only**: nothing is persisted, signed or sent
/// anywhere, so they are an operator aid, not a fiscal receipt (signed receipts belong to
/// `drenyra-ai`).
#[derive(Debug, Clone)]
pub struct DrenyraHostController {
    workspaces: WorkspaceManager,
    reviews: BTreeMap<usize, EntryDiffReview>,
    /// In-flight `ledger_post` per workspace index (request id plus what its frame carried),
    /// awaiting transport confirmation.
    pending_posts: BTreeMap<usize, PendingPost>,
    reconciliation: ReconciliationTable,
    next_request_id: u64,
    /// Session evidence log per workspace identity (append-only, unsigned, not persisted).
    evidence: BTreeMap<WorkspaceKey, Vec<EvidenceEvent>>,
    next_evidence_seq: u64,
    /// Time source for evidence events; read only when an event is appended.
    clock: Clock,
    /// Entries posted in this session per workspace identity (not persisted).
    posted: BTreeMap<WorkspaceKey, Vec<PostedEntry>>,
}

impl DrenyraHostController {
    /// Controller whose evidence events are stamped by the system clock.
    pub fn new(initial_workspace: FiscalWorkspace) -> Self {
        Self::with_clock(initial_workspace, Clock::System)
    }

    /// Controller whose evidence events are stamped by `clock` (for example [`Clock::fixed`] in
    /// tests). The clock is set once, before any event, and never changes afterwards.
    pub fn with_clock(initial_workspace: FiscalWorkspace, clock: Clock) -> Self {
        Self {
            workspaces: WorkspaceManager::new(initial_workspace),
            reviews: BTreeMap::new(),
            pending_posts: BTreeMap::new(),
            reconciliation: ReconciliationTable::default(),
            next_request_id: 1,
            evidence: BTreeMap::new(),
            next_evidence_seq: 1,
            clock,
            posted: BTreeMap::new(),
        }
    }

    /// Session evidence log of the workspace `key`, oldest first; empty when nothing happened.
    /// In memory only: it is neither persisted nor signed.
    pub fn evidence(&self, key: &WorkspaceKey) -> &[EvidenceEvent] {
        self.evidence.get(key).map_or(&[], Vec::as_slice)
    }

    /// Entries of the workspace `key` whose post was confirmed in this session, oldest first.
    /// In memory only: it is neither persisted nor signed.
    pub fn posted(&self, key: &WorkspaceKey) -> &[PostedEntry] {
        self.posted.get(key).map_or(&[], Vec::as_slice)
    }

    /// Identity of the workspace that owns `proposal`: its RUC and period, which matched the
    /// active workspace when it was staged.
    fn owner_key(proposal: &JournalEntryProposal) -> WorkspaceKey {
        WorkspaceKey::new(&proposal.entity_ruc, &proposal.fiscal_period)
    }

    fn log(&mut self, owner: WorkspaceKey, kind: EvidenceKind) {
        let seq = self.next_evidence_seq;
        self.next_evidence_seq = seq.saturating_add(1);
        let recorded_at_ms = self.clock.now_ms();
        self.evidence.entry(owner).or_default().push(EvidenceEvent {
            seq,
            recorded_at_ms,
            kind,
        });
    }

    /// Records the first human approval of the active workspace's staged review and logs it.
    /// Returns `false` and logs nothing when there is no staged review, the ID is invalid, or an
    /// approval already exists (replacing one needs [`Self::replace_active_approval`]).
    pub fn approve_active(&mut self, approver: &str) -> bool {
        let Some(review) = self.active_review_mut_inner() else {
            return false;
        };
        if !review.record_human_approval(approver) {
            return false;
        }
        let owner = Self::owner_key(review.proposal());
        let kind = EvidenceKind::Approved {
            proposal_id: review.proposal().id.clone(),
            approver: review.approved_by().unwrap_or_default().to_owned(),
        };
        self.log(owner, kind);
        true
    }

    /// Replaces the active review's approver (see [`EntryDiffReview::replace_human_approval`])
    /// and logs both IDs. Without a staged review it fails with
    /// [`ReplaceApprovalError::NothingToReplace`]. On error nothing changes and nothing is logged.
    pub fn replace_active_approval(
        &mut self,
        expected_previous: &str,
        approver: &str,
    ) -> Result<(), ReplaceApprovalError> {
        let review = self
            .active_review_mut_inner()
            .ok_or(ReplaceApprovalError::NothingToReplace)?;
        review.replace_human_approval(expected_previous, approver)?;
        let owner = Self::owner_key(review.proposal());
        let kind = EvidenceKind::ApprovalReplaced {
            proposal_id: review.proposal().id.clone(),
            previous: expected_previous.to_owned(),
            approver: review.approved_by().unwrap_or_default().to_owned(),
        };
        self.log(owner, kind);
        Ok(())
    }

    /// Accepts line `idx` of the active review; logs [`EvidenceKind::ApprovalCleared`] when the
    /// change cleared an approval. Returns `false` without a staged review or for an unknown line.
    pub fn accept_active_line(&mut self, idx: usize) -> bool {
        self.decide_active_line(|review| review.accept_line(idx))
    }

    /// Rejects line `idx` of the active review with `comment`; logs
    /// [`EvidenceKind::ApprovalCleared`] when the change cleared an approval. Returns `false`
    /// without a staged review, for an unknown line or for a blank comment.
    pub fn reject_active_line(&mut self, idx: usize, comment: &str) -> bool {
        self.decide_active_line(|review| review.reject_line(idx, comment))
    }

    fn decide_active_line(&mut self, decide: impl FnOnce(&mut EntryDiffReview) -> bool) -> bool {
        let Some(review) = self.active_review_mut_inner() else {
            return false;
        };
        let previous = review.approved_by().map(str::to_owned);
        if !decide(review) {
            return false;
        }
        if let (Some(previous), None) = (previous, review.approved_by()) {
            let owner = Self::owner_key(review.proposal());
            let kind = EvidenceKind::ApprovalCleared {
                proposal_id: review.proposal().id.clone(),
                previous,
            };
            self.log(owner, kind);
        }
        true
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
        let staged = EvidenceKind::Staged {
            proposal_id: proposal.id.clone(),
            risk_tier,
        };
        self.log(Self::owner_key(&proposal), staged);
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

    /// Direct mutable access for tests only: production code mutates reviews through the
    /// controller methods above so every audit-relevant change reaches the evidence log.
    #[cfg(test)]
    pub fn active_review_mut(&mut self) -> Option<&mut EntryDiffReview> {
        self.active_review_mut_inner()
    }

    fn active_review_mut_inner(&mut self) -> Option<&mut EntryDiffReview> {
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
        let owner = Self::owner_key(proposal);
        let requested = EvidenceKind::PostRequested {
            proposal_id: proposal.id.clone(),
            request_id: request.id,
        };
        let pending = PendingPost {
            request_id: request.id,
            proposal: proposal.clone(),
            approved_by: review.approved_by().map(str::to_owned),
        };

        self.pending_posts
            .insert(self.workspaces.active_index(), pending);
        self.next_request_id = self.next_request_id.saturating_add(1);
        self.log(owner, requested);
        Ok(frame)
    }

    /// Request id of the active workspace's in-flight `ledger_post`, if a frame was built and
    /// not yet confirmed.
    pub fn pending_post_request(&self) -> Option<u64> {
        self.pending_posts
            .get(&self.workspaces.active_index())
            .map(|pending| pending.request_id)
    }

    /// Records that the transport confirmed the `ledger_post` request `request_id`, clearing the
    /// review and guardrails of the workspace that built it, even if another workspace is active.
    ///
    /// The posted record ([`PostedEntry`] and the [`EvidenceKind::Posted`] event) is what the
    /// confirmed frame sent: the proposal and approver captured by [`Self::build_post_request`]
    /// for `request_id`, even if the approval was replaced, or cleared by a line change, after
    /// the frame was built.
    ///
    /// Fails with [`ControllerError::UnknownPostRequest`] for ids that are not in flight, such as
    /// one superseded by a retry or already confirmed.
    pub fn confirm_posted(&mut self, request_id: u64) -> Result<(), ControllerError> {
        let index = self
            .pending_posts
            .iter()
            .find_map(|(&index, pending)| (pending.request_id == request_id).then_some(index))
            .ok_or(ControllerError::UnknownPostRequest)?;

        if let Some(sent) = self.pending_posts.remove(&index) {
            self.reviews.remove(&index);
            let owner = Self::owner_key(&sent.proposal);
            let posted = EvidenceKind::Posted {
                proposal_id: sent.proposal.id.clone(),
                request_id,
            };
            self.log(owner.clone(), posted);
            self.posted.entry(owner).or_default().push(PostedEntry {
                proposal: sent.proposal,
                approved_by: sent.approved_by,
                request_id,
            });
        }
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
        let idx_b = controller
            .workspaces_mut()
            .add_workspace(ws_b)
            .expect("distinct workspace");
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
            // Approver IDs cannot carry newlines or other hidden characters (drenyra-ui guard);
            // quotes and backslashes are visible and still need escaping in the frame.
            assert!(!review.record_human_approval("CPA\\\"01\nPERU"));
            assert!(review.record_human_approval("CPA\\\"01 PERU"));
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
        let idx_b = controller
            .workspaces_mut()
            .add_workspace(ws_b)
            .expect("distinct workspace");

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
        let idx2 = controller
            .workspaces_mut()
            .add_workspace(ws2)
            .expect("distinct workspace");
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

    fn two_workspaces() -> (DrenyraHostController, WorkspaceKey, WorkspaceKey, usize) {
        let ws_a = FiscalWorkspace::new("20123456789", "Andino SAC", "2026-10");
        let ws_b = FiscalWorkspace::new("20987654321", "Sierra SRL", "2026-11");
        let (key_a, key_b) = (ws_a.key(), ws_b.key());
        let mut controller = DrenyraHostController::new(ws_a);
        let idx_b = controller
            .workspaces_mut()
            .add_workspace(ws_b)
            .expect("distinct workspace");
        (controller, key_a, key_b, idx_b)
    }

    fn kinds(controller: &DrenyraHostController, key: &WorkspaceKey) -> Vec<EvidenceKind> {
        controller
            .evidence(key)
            .iter()
            .map(|event| event.kind.clone())
            .collect()
    }

    #[test]
    fn evidence_log_records_the_approval_lifecycle_on_the_owning_workspace() {
        let (mut controller, key_a, key_b, idx_b) = two_workspaces();
        assert!(controller.evidence(&key_a).is_empty());
        let proposal = sample_proposal("je-001", RiskTier::R2Material, true);
        controller.stage_proposal(proposal.clone()).expect("stage");
        for line in 0..3 {
            assert!(controller.accept_active_line(line));
        }
        assert!(controller.approve_active("  CPA-1 "));
        assert_eq!(controller.replace_active_approval("CPA-1", "CPA-2"), Ok(()));
        assert_eq!(
            controller
                .active_review()
                .and_then(EntryDiffReview::approved_by),
            Some("CPA-2")
        );
        assert!(controller.reject_active_line(0, "wrong account"));
        assert!(controller.accept_active_line(0));
        assert!(controller.approve_active("CPA-3"));
        controller.build_post_request().expect("frame");
        assert!(controller.workspaces_mut().switch_to(idx_b));
        assert_eq!(controller.confirm_posted(1), Ok(()));

        let id = || "je-001".to_owned();
        assert_eq!(
            kinds(&controller, &key_a),
            vec![
                EvidenceKind::Staged {
                    proposal_id: id(),
                    risk_tier: RiskTier::R2Material
                },
                EvidenceKind::Approved {
                    proposal_id: id(),
                    approver: "CPA-1".into()
                },
                EvidenceKind::ApprovalReplaced {
                    proposal_id: id(),
                    previous: "CPA-1".into(),
                    approver: "CPA-2".into()
                },
                EvidenceKind::ApprovalCleared {
                    proposal_id: id(),
                    previous: "CPA-2".into()
                },
                EvidenceKind::Approved {
                    proposal_id: id(),
                    approver: "CPA-3".into()
                },
                EvidenceKind::PostRequested {
                    proposal_id: id(),
                    request_id: 1
                },
                EvidenceKind::Posted {
                    proposal_id: id(),
                    request_id: 1
                },
            ]
        );
        let seqs: Vec<u64> = controller.evidence(&key_a).iter().map(|e| e.seq).collect();
        assert_eq!(seqs, vec![1, 2, 3, 4, 5, 6, 7]);
        assert!(
            controller.evidence(&key_b).is_empty(),
            "the active workspace at confirm time gets nothing"
        );
        assert_eq!(
            controller.posted(&key_a),
            &[PostedEntry {
                proposal,
                approved_by: Some("CPA-3".into()),
                request_id: 1,
            }]
        );
        assert!(controller.posted(&key_b).is_empty());
    }

    #[test]
    fn sequence_numbers_are_global_and_strictly_increasing_across_workspaces() {
        let (mut controller, key_a, key_b, idx_b) = two_workspaces();
        controller
            .stage_proposal(sample_proposal("je-a", RiskTier::R1Draft, true))
            .expect("stage A");
        assert!(controller.workspaces_mut().switch_to(idx_b));
        let mut sierra = sample_proposal("je-b", RiskTier::R1Draft, true);
        (sierra.entity_ruc, sierra.fiscal_period) = ("20987654321".into(), "2026-11".into());
        controller.stage_proposal(sierra).expect("stage B");
        for line in 0..3 {
            assert!(controller.accept_active_line(line));
        }
        controller.build_post_request().expect("first frame");
        controller.build_post_request().expect("retry frame");
        assert_eq!(controller.confirm_posted(2), Ok(()));

        let seq_a: Vec<u64> = controller.evidence(&key_a).iter().map(|e| e.seq).collect();
        let seq_b: Vec<u64> = controller.evidence(&key_b).iter().map(|e| e.seq).collect();
        assert_eq!(seq_a, vec![1]);
        assert_eq!(seq_b, vec![2, 3, 4, 5]);
        assert_eq!(
            kinds(&controller, &key_b)[1..],
            [
                EvidenceKind::PostRequested {
                    proposal_id: "je-b".into(),
                    request_id: 1
                },
                EvidenceKind::PostRequested {
                    proposal_id: "je-b".into(),
                    request_id: 2
                },
                EvidenceKind::Posted {
                    proposal_id: "je-b".into(),
                    request_id: 2
                },
            ]
        );
        assert_eq!(
            controller.posted(&key_b)[0].approved_by,
            None,
            "R1 drafts post without an approver"
        );
        assert_eq!(controller.posted(&key_b)[0].request_id, 2);
    }

    #[test]
    fn refused_operations_append_no_evidence() {
        let (mut controller, key_a, key_b, _) = two_workspaces();
        // No staged review.
        assert!(!controller.approve_active("CPA-1"));
        assert_eq!(
            controller.replace_active_approval("CPA-1", "CPA-2"),
            Err(ReplaceApprovalError::NothingToReplace)
        );
        assert!(!controller.accept_active_line(0));
        assert!(!controller.reject_active_line(0, "x"));
        assert!(controller.build_post_request().is_err());
        assert!(controller.confirm_posted(1).is_err());
        let mut foreign = sample_proposal("je-x", RiskTier::R2Material, true);
        foreign.entity_ruc = "20987654321".into();
        assert!(controller.stage_proposal(foreign).is_err());
        assert!(controller.evidence(&key_a).is_empty());

        controller
            .stage_proposal(sample_proposal("je-001", RiskTier::R2Material, true))
            .expect("stage");
        assert!(controller
            .stage_proposal(sample_proposal("je-002", RiskTier::R2Material, true))
            .is_err());
        assert!(controller.build_post_request().is_err(), "lines pending");
        for line in 0..3 {
            assert!(controller.accept_active_line(line));
        }
        assert!(!controller.approve_active("\u{200B}"));
        assert!(!controller.approve_active("   "));
        assert_eq!(
            controller.replace_active_approval("CPA-1", "CPA-2"),
            Err(ReplaceApprovalError::NothingToReplace)
        );
        assert!(controller.approve_active("CPA-1"));
        assert!(
            !controller.approve_active("CPA-2"),
            "a second approval needs an explicit replace"
        );
        assert_eq!(
            controller.replace_active_approval("CPA-9", "CPA-2"),
            Err(ReplaceApprovalError::StaleApproval {
                current: "CPA-1".into()
            })
        );
        assert_eq!(
            controller.replace_active_approval("CPA-1", " CPA-1 "),
            Err(ReplaceApprovalError::SameApprover)
        );
        assert_eq!(
            controller.replace_active_approval("CPA-1", "CPA\u{2066}2"),
            Err(ReplaceApprovalError::InvalidApprover)
        );
        assert!(controller.accept_active_line(0), "same decision");
        assert!(!controller.reject_active_line(0, "  "));
        assert!(!controller.accept_active_line(99));
        assert!(controller.confirm_posted(7).is_err());

        assert_eq!(
            kinds(&controller, &key_a),
            vec![
                EvidenceKind::Staged {
                    proposal_id: "je-001".into(),
                    risk_tier: RiskTier::R2Material
                },
                EvidenceKind::Approved {
                    proposal_id: "je-001".into(),
                    approver: "CPA-1".into()
                },
            ]
        );
        assert_eq!(
            controller
                .active_review()
                .and_then(EntryDiffReview::approved_by),
            Some("CPA-1")
        );
        assert!(controller.evidence(&key_b).is_empty());
        assert!(controller.posted(&key_a).is_empty());
    }

    #[test]
    fn fixed_clock_returns_start_then_advances_by_step_and_saturates() {
        let mut clock = Clock::fixed(1_000, 250);
        assert_eq!(
            [clock.now_ms(), clock.now_ms(), clock.now_ms()],
            [1_000, 1_250, 1_500]
        );
        let mut frozen = Clock::fixed(42, 0);
        assert_eq!([frozen.now_ms(), frozen.now_ms()], [42, 42]);
        let mut edge = Clock::fixed(u64::MAX - 1, 5);
        assert_eq!([edge.now_ms(), edge.now_ms()], [u64::MAX - 1, u64::MAX]);
        assert_eq!(edge.now_ms(), u64::MAX, "saturates instead of wrapping");
        assert_eq!(Clock::default(), Clock::System);
    }

    #[test]
    fn evidence_events_carry_the_clock_time_and_refusals_never_read_the_clock() {
        let ws = FiscalWorkspace::new("20123456789", "Andino SAC", "2026-10");
        let key = ws.key();
        let mut controller = DrenyraHostController::with_clock(ws, Clock::fixed(1_000, 250));
        controller
            .stage_proposal(sample_proposal("je-001", RiskTier::R2Material, true))
            .expect("stage");
        // Refused (blank ID, unknown line): no clock reading. Accepting lines clears no
        // approval, so it logs nothing either.
        assert!(!controller.approve_active("   "));
        assert!(!controller.accept_active_line(99));
        for line in 0..3 {
            assert!(controller.accept_active_line(line));
        }
        assert!(controller.approve_active("CPA-1"));
        controller.build_post_request().expect("frame");
        assert_eq!(controller.confirm_posted(1), Ok(()));

        let id = || "je-001".to_owned();
        assert_eq!(
            controller.evidence(&key),
            &[
                EvidenceEvent {
                    seq: 1,
                    recorded_at_ms: 1_000,
                    kind: EvidenceKind::Staged {
                        proposal_id: id(),
                        risk_tier: RiskTier::R2Material
                    }
                },
                EvidenceEvent {
                    seq: 2,
                    recorded_at_ms: 1_250,
                    kind: EvidenceKind::Approved {
                        proposal_id: id(),
                        approver: "CPA-1".into()
                    }
                },
                EvidenceEvent {
                    seq: 3,
                    recorded_at_ms: 1_500,
                    kind: EvidenceKind::PostRequested {
                        proposal_id: id(),
                        request_id: 1
                    }
                },
                EvidenceEvent {
                    seq: 4,
                    recorded_at_ms: 1_750,
                    kind: EvidenceKind::Posted {
                        proposal_id: id(),
                        request_id: 1
                    }
                },
            ]
        );
    }

    #[test]
    fn new_stamps_events_with_the_system_clock() {
        // The wall clock may step backwards (NTP, a manual change) between readings, so the
        // stamp is only required to be non-zero and near the readings taken around it. Ten
        // minutes tolerates any realistic step while still rejecting a fixed or zero clock.
        const SLACK_MS: u128 = 10 * 60 * 1_000;
        let system_ms = || {
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|elapsed| elapsed.as_millis())
                .expect("system clock after 1970")
        };
        let ws = FiscalWorkspace::new("20123456789", "Andino SAC", "2026-10");
        let key = ws.key();
        let mut controller = DrenyraHostController::new(ws);
        let before = system_ms();
        controller
            .stage_proposal(sample_proposal("je-001", RiskTier::R2Material, true))
            .expect("stage");
        let after = system_ms();
        let recorded = u128::from(controller.evidence(&key)[0].recorded_at_ms);
        assert_ne!(recorded, 0, "stamped with a zero clock");
        let window = before.min(after).saturating_sub(SLACK_MS)..=before.max(after) + SLACK_MS;
        assert!(
            window.contains(&recorded),
            "{recorded} outside {window:?} (readings {before} and {after})"
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
