//! Accounting Generative UI (`drenyra_ui`) domain surfaces for Drenyra GPUI.

use drenyra_protocol::{JournalEntryProposal, MoneyCents};

/// Per-line human review decision for a proposed double-entry journal line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LineDecision {
    Pending,
    Accepted,
    Rejected { comment: String },
}

/// Interactive line-by-line review state over a [`JournalEntryProposal`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EntryDiffReview {
    proposal: JournalEntryProposal,
    decisions: Vec<LineDecision>,
    approved_by: Option<String>,
}

impl EntryDiffReview {
    /// Creates a new review with every line initialized to [`LineDecision::Pending`].
    pub fn new(proposal: JournalEntryProposal) -> Self {
        let line_count = proposal.lines.len();
        Self {
            proposal,
            decisions: vec![LineDecision::Pending; line_count],
            approved_by: None,
        }
    }

    pub fn proposal(&self) -> &JournalEntryProposal {
        &self.proposal
    }

    pub fn decisions(&self) -> &[LineDecision] {
        &self.decisions
    }

    /// Marks the line at `idx` as [`LineDecision::Accepted`]. Returns `false` if out of bounds.
    /// Changing a decision clears any recorded human approval (see [`Self::approved_by`]).
    pub fn accept_line(&mut self, idx: usize) -> bool {
        self.set_decision(idx, LineDecision::Accepted)
    }

    /// Marks the line at `idx` as [`LineDecision::Rejected`] with a trimmed non-empty comment.
    /// Returns `false` if `idx` is out of bounds or `comment` is empty/whitespace-only.
    /// Changing a decision clears any recorded human approval (see [`Self::approved_by`]).
    pub fn reject_line(&mut self, idx: usize, comment: impl Into<String>) -> bool {
        let raw = comment.into();
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            return false;
        }
        self.set_decision(
            idx,
            LineDecision::Rejected {
                comment: trimmed.to_string(),
            },
        )
    }

    /// Marks every line in the proposal as [`LineDecision::Accepted`].
    /// Clears any recorded human approval if at least one line changed.
    pub fn accept_all(&mut self) {
        for idx in 0..self.decisions.len() {
            self.set_decision(idx, LineDecision::Accepted);
        }
    }

    /// Single write path for line decisions. A human approval covers the exact set of
    /// decisions it was given for, so any real change invalidates it; repeating the
    /// current decision is a no-op and keeps the approval.
    fn set_decision(&mut self, idx: usize, decision: LineDecision) -> bool {
        let Some(slot) = self.decisions.get_mut(idx) else {
            return false;
        };
        if *slot != decision {
            *slot = decision;
            self.approved_by = None;
        }
        true
    }

    /// Returns the number of lines that are no longer [`LineDecision::Pending`].
    pub fn decided_count(&self) -> usize {
        self.decisions
            .iter()
            .filter(|decision| !matches!(decision, LineDecision::Pending))
            .count()
    }

    /// Returns `true` when every line has a non-pending decision.
    pub fn all_decided(&self) -> bool {
        self.decisions
            .iter()
            .all(|decision| !matches!(decision, LineDecision::Pending))
    }

    /// Records explicit human approval for posting. Rejects empty or whitespace-only approver IDs.
    pub fn record_human_approval(&mut self, approver: impl Into<String>) -> bool {
        let raw = approver.into();
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            return false;
        }
        self.approved_by = Some(trimmed.to_string());
        true
    }

    /// The recorded human approver, if any. It is cleared whenever a line decision changes,
    /// so it always refers to the decisions currently on screen.
    pub fn approved_by(&self) -> Option<&str> {
        self.approved_by.as_deref()
    }

    /// Returns `true` iff the underlying proposal is balanced, every line is accepted,
    /// and a human approver is recorded whenever `risk_tier.requires_human_approval()` holds.
    pub fn can_post(&self) -> bool {
        if !self.proposal.is_balanced() {
            return false;
        }
        let all_accepted = self.decisions.len() == self.proposal.lines.len()
            && self
                .decisions
                .iter()
                .all(|decision| matches!(decision, LineDecision::Accepted));
        if !all_accepted {
            return false;
        }
        if self.proposal.risk_tier.requires_human_approval() && self.approved_by.is_none() {
            return false;
        }
        true
    }
}

/// Reconciliation state of a bank statement vs. general ledger row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReconciliationStatus {
    Matched,
    Pending,
    Discrepancy,
}

/// Single bank-vs-ledger comparison row using exact integer cents (`MoneyCents`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReconciliationRow {
    pub id: String,
    pub reference: String,
    pub bank_amount: MoneyCents,
    pub ledger_amount: MoneyCents,
    pub status: ReconciliationStatus,
}

impl ReconciliationRow {
    /// Exact signed difference `bank_amount - ledger_amount`, or `None` on `i128` overflow.
    pub fn delta(&self) -> Option<MoneyCents> {
        self.bank_amount
            .0
            .checked_sub(self.ledger_amount.0)
            .map(MoneyCents)
    }
}

/// Bank-vs-ledger reconciliation table surface.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ReconciliationTable {
    rows: Vec<ReconciliationRow>,
}

impl ReconciliationTable {
    pub fn new(rows: Vec<ReconciliationRow>) -> Self {
        Self { rows }
    }

    pub fn rows(&self) -> &[ReconciliationRow] {
        &self.rows
    }

    pub fn filter_by_status(&self, status: ReconciliationStatus) -> Vec<&ReconciliationRow> {
        self.rows
            .iter()
            .filter(|row| row.status == status)
            .collect()
    }

    /// Checked sum of `bank_amount - ledger_amount` across all non-`Matched` rows.
    /// Returns `None` if any row delta or the cumulative sum overflows `i128`.
    pub fn total_unmatched_delta(&self) -> Option<MoneyCents> {
        let mut total = MoneyCents::ZERO;
        for row in &self.rows {
            if row.status == ReconciliationStatus::Matched {
                continue;
            }
            let row_delta = row.delta()?;
            total = total.checked_add(row_delta)?;
        }
        Some(total)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use drenyra_protocol::{JournalLine, RiskTier};

    fn sample_proposal(risk_tier: RiskTier, balanced: bool) -> JournalEntryProposal {
        let credit_tax = if balanced { 1_800 } else { 1_799 };
        JournalEntryProposal {
            id: "je-2026-001".into(),
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
                    credit: MoneyCents::from_cents(credit_tax),
                    description: "IGV 18%".into(),
                },
            ],
        }
    }

    #[test]
    fn r1_draft_posts_when_balanced_and_all_lines_accepted_without_human_approver() {
        let mut review = EntryDiffReview::new(sample_proposal(RiskTier::R1Draft, true));
        assert_eq!(review.proposal().id, "je-2026-001");
        assert_eq!(review.decided_count(), 0);
        assert!(!review.all_decided());
        assert!(!review.can_post());

        assert!(review.accept_line(0));
        assert!(review.accept_line(1));
        assert!(!review.accept_line(99));
        assert_eq!(review.decided_count(), 2);
        assert!(!review.all_decided());
        assert!(!review.can_post());

        assert!(review.accept_line(2));
        assert_eq!(review.decided_count(), 3);
        assert!(review.all_decided());
        assert_eq!(review.approved_by(), None);
        assert!(review.can_post());
    }

    #[test]
    fn r2_and_r3_require_non_empty_human_approval_before_posting() {
        for tier in [RiskTier::R2Material, RiskTier::R3Irreversible] {
            let mut review = EntryDiffReview::new(sample_proposal(tier, true));
            review.accept_all();
            assert!(review.all_decided());
            assert!(
                !review.can_post(),
                "tier {tier:?} must require human approval"
            );

            assert!(!review.record_human_approval(""));
            assert!(!review.record_human_approval("   \t\n "));
            assert_eq!(review.approved_by(), None);
            assert!(!review.can_post());

            assert!(review.record_human_approval("  CPA-123  "));
            assert_eq!(review.approved_by(), Some("CPA-123"));
            assert!(review.can_post());
        }
    }

    #[test]
    fn changing_a_decision_after_approval_clears_the_approval() {
        let mut review = EntryDiffReview::new(sample_proposal(RiskTier::R2Material, true));
        review.accept_all();
        assert!(review.record_human_approval("CPA-123"));
        assert!(review.can_post());

        assert!(review.reject_line(1, "Wrong revenue account"));
        assert_eq!(review.approved_by(), None);
        assert!(review.accept_line(1));
        assert_eq!(review.approved_by(), None);
        assert!(
            !review.can_post(),
            "re-accepting must not revive a stale approval"
        );

        // Approval given before every line was decided does not survive the later decisions.
        let mut early = EntryDiffReview::new(sample_proposal(RiskTier::R3Irreversible, true));
        assert!(early.record_human_approval("CPA-123"));
        early.accept_all();
        assert_eq!(early.approved_by(), None);
        assert!(!early.can_post());
    }

    #[test]
    fn repeating_the_same_decision_keeps_the_approval() {
        let mut review = EntryDiffReview::new(sample_proposal(RiskTier::R2Material, true));
        review.accept_all();
        assert!(review.record_human_approval("CPA-123"));

        assert!(review.accept_line(0));
        review.accept_all();
        assert!(!review.accept_line(99));
        assert!(!review.reject_line(0, "   "));
        assert_eq!(review.approved_by(), Some("CPA-123"));
        assert!(review.can_post());
    }

    #[test]
    fn rejecting_line_blocks_posting_and_tracks_decided_count() {
        let mut review = EntryDiffReview::new(sample_proposal(RiskTier::R1Draft, true));
        assert!(!review.reject_line(0, ""));
        assert!(!review.reject_line(0, "   "));
        assert!(!review.reject_line(99, "Out of bounds"));
        assert_eq!(review.decided_count(), 0);
        assert_eq!(review.decisions()[0], LineDecision::Pending);

        assert!(review.reject_line(1, "  Wrong revenue account  "));
        assert_eq!(review.decided_count(), 1);
        assert_eq!(
            review.decisions()[1],
            LineDecision::Rejected {
                comment: "Wrong revenue account".into()
            }
        );

        assert!(review.accept_line(0));
        assert!(review.accept_line(2));
        assert!(review.all_decided());
        assert_eq!(review.decided_count(), 3);
        assert!(!review.can_post());

        // Re-accepting the previously rejected line allows posting.
        assert!(review.accept_line(1));
        assert!(review.can_post());
    }

    #[test]
    fn unbalanced_proposal_never_posts_even_with_all_accepted_and_approval() {
        let mut review = EntryDiffReview::new(sample_proposal(RiskTier::R2Material, false));
        review.accept_all();
        assert!(review.record_human_approval("CPA-123"));
        assert!(review.all_decided());
        assert!(!review.can_post());
    }

    #[test]
    fn reconciliation_table_filters_and_computes_exact_deltas() {
        let rows = vec![
            ReconciliationRow {
                id: "rec-1".into(),
                reference: "BCP-001".into(),
                bank_amount: MoneyCents::from_cents(50_000),
                ledger_amount: MoneyCents::from_cents(50_000),
                status: ReconciliationStatus::Matched,
            },
            ReconciliationRow {
                id: "rec-2".into(),
                reference: "BCP-002".into(),
                bank_amount: MoneyCents::from_cents(12_500),
                ledger_amount: MoneyCents::from_cents(10_000),
                status: ReconciliationStatus::Pending,
            },
            ReconciliationRow {
                id: "rec-3".into(),
                reference: "BCP-003".into(),
                bank_amount: MoneyCents::from_cents(8_000),
                ledger_amount: MoneyCents::from_cents(9_500),
                status: ReconciliationStatus::Discrepancy,
            },
        ];

        assert_eq!(rows[0].delta(), Some(MoneyCents::ZERO));
        assert_eq!(rows[1].delta(), Some(MoneyCents::from_cents(2_500)));
        assert_eq!(rows[2].delta(), Some(MoneyCents::from_cents(-1_500)));

        let table = ReconciliationTable::new(rows);
        assert_eq!(table.rows().len(), 3);
        assert_eq!(
            table
                .filter_by_status(ReconciliationStatus::Matched)
                .into_iter()
                .map(|r| r.id.as_str())
                .collect::<Vec<_>>(),
            vec!["rec-1"]
        );
        assert_eq!(
            table
                .filter_by_status(ReconciliationStatus::Pending)
                .into_iter()
                .map(|r| r.id.as_str())
                .collect::<Vec<_>>(),
            vec!["rec-2"]
        );
        assert_eq!(
            table
                .filter_by_status(ReconciliationStatus::Discrepancy)
                .into_iter()
                .map(|r| r.id.as_str())
                .collect::<Vec<_>>(),
            vec!["rec-3"]
        );
        // Unmatched delta = +2500 + (-1500) = +1000
        assert_eq!(
            table.total_unmatched_delta(),
            Some(MoneyCents::from_cents(1_000))
        );

        let all_matched = ReconciliationTable::new(vec![ReconciliationRow {
            id: "rec-ok".into(),
            reference: "BCP-999".into(),
            bank_amount: MoneyCents::from_cents(100),
            ledger_amount: MoneyCents::from_cents(100),
            status: ReconciliationStatus::Matched,
        }]);
        assert_eq!(all_matched.total_unmatched_delta(), Some(MoneyCents::ZERO));
    }

    #[test]
    fn reconciliation_delta_and_total_return_none_on_overflow() {
        let row_sub_overflow = ReconciliationRow {
            id: "ovf-1".into(),
            reference: "OVF-SUB".into(),
            bank_amount: MoneyCents::from_cents(i128::MAX),
            ledger_amount: MoneyCents::from_cents(-1),
            status: ReconciliationStatus::Discrepancy,
        };
        assert_eq!(row_sub_overflow.delta(), None);

        let table_sub_overflow = ReconciliationTable::new(vec![row_sub_overflow]);
        assert_eq!(table_sub_overflow.total_unmatched_delta(), None);

        let table_add_overflow = ReconciliationTable::new(vec![
            ReconciliationRow {
                id: "ovf-2a".into(),
                reference: "OVF-A".into(),
                bank_amount: MoneyCents::from_cents(i128::MAX),
                ledger_amount: MoneyCents::ZERO,
                status: ReconciliationStatus::Pending,
            },
            ReconciliationRow {
                id: "ovf-2b".into(),
                reference: "OVF-B".into(),
                bank_amount: MoneyCents::from_cents(1),
                ledger_amount: MoneyCents::ZERO,
                status: ReconciliationStatus::Discrepancy,
            },
        ]);
        assert_eq!(table_add_overflow.total_unmatched_delta(), None);
    }
}
