//! Zero-float fiscal protocol contracts and JSON-RPC 2.0 stdio framing for Drenyra GPUI.

use std::fmt;

/// Exact monetary value in signed integer cents (BigInt-cent invariant; never float).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct MoneyCents(pub i128);

impl MoneyCents {
    pub const ZERO: Self = Self(0);

    pub const fn from_cents(cents: i128) -> Self {
        Self(cents)
    }

    pub const fn checked_add(self, other: Self) -> Option<Self> {
        match self.0.checked_add(other.0) {
            Some(sum) => Some(Self(sum)),
            None => None,
        }
    }

    pub fn format_decimal(self) -> String {
        let sign = if self.0 < 0 { "-" } else { "" };
        let abs = self.0.unsigned_abs();
        let whole = abs / 100;
        let frac = abs % 100;
        format!("{sign}{whole}.{frac:02}")
    }
}

impl fmt::Display for MoneyCents {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.format_decimal())
    }
}

/// Risk tier governing human-in-the-loop authority (R0 read-only -> R3 irreversible dual approval).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RiskTier {
    R0Read,
    R1Draft,
    R2Material,
    R3Irreversible,
}

impl RiskTier {
    pub const fn requires_human_approval(self) -> bool {
        matches!(self, Self::R2Material | Self::R3Irreversible)
    }
}

/// One side of a proposed double-entry journal line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JournalLine {
    pub account_code: String,
    pub debit: MoneyCents,
    pub credit: MoneyCents,
    pub description: String,
}

/// Proposed accounting journal entry reviewed in Drenyra GPUI before posting.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JournalEntryProposal {
    pub id: String,
    pub entity_ruc: String,
    pub fiscal_period: String,
    pub risk_tier: RiskTier,
    pub lines: Vec<JournalLine>,
}

impl JournalEntryProposal {
    /// Returns true iff the entry has at least two lines, every line is strictly positive on
    /// exactly one side (debit XOR credit), and total debits equal total credits.
    pub fn is_balanced(&self) -> bool {
        if self.lines.len() < 2 {
            return false;
        }
        let mut total_debit = MoneyCents::ZERO;
        let mut total_credit = MoneyCents::ZERO;

        for line in &self.lines {
            let debit_pos = line.debit.0 > 0;
            let credit_pos = line.credit.0 > 0;
            let debit_neg = line.debit.0 < 0;
            let credit_neg = line.credit.0 < 0;

            if debit_neg || credit_neg || debit_pos == credit_pos {
                return false;
            }
            match total_debit.checked_add(line.debit) {
                Some(sum) => total_debit = sum,
                None => return false,
            }
            match total_credit.checked_add(line.credit) {
                Some(sum) => total_credit = sum,
                None => return false,
            }
        }

        total_debit.0 > 0 && total_debit == total_credit
    }
}

/// Minimal JSON-RPC 2.0 request frame for `drenyra-shell --mode rpc` and `drenyra-ai mcp serve`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JsonRpcRequest {
    pub id: u64,
    pub method: String,
    pub params_json: String,
}

impl JsonRpcRequest {
    pub fn to_ndjson(&self) -> String {
        let escaped_method = escape_json_str(&self.method);
        format!(
            r#"{{"jsonrpc":"2.0","id":{},"method":"{}","params":{}}}"#,
            self.id, escaped_method, self.params_json
        )
    }
}

fn escape_json_str(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for ch in input.chars() {
        match ch {
            '"' => out.push_str(r#"\""#),
            '\\' => out.push_str(r#"\\"#),
            '\n' => out.push_str(r#"\n"#),
            '\r' => out.push_str(r#"\r"#),
            '\t' => out.push_str(r#"\t"#),
            c => out.push(c),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn money_cents_formats_positive_negative_and_zero_without_floats() {
        assert_eq!(MoneyCents::ZERO.format_decimal(), "0.00");
        assert_eq!(MoneyCents::from_cents(125_050).format_decimal(), "1250.50");
        assert_eq!(MoneyCents::from_cents(-5).format_decimal(), "-0.05");
        assert_eq!(
            MoneyCents::from_cents(100).checked_add(MoneyCents::from_cents(250)),
            Some(MoneyCents::from_cents(350))
        );
    }

    #[test]
    fn risk_tier_enforces_human_approval_for_r2_and_r3() {
        assert!(!RiskTier::R0Read.requires_human_approval());
        assert!(!RiskTier::R1Draft.requires_human_approval());
        assert!(RiskTier::R2Material.requires_human_approval());
        assert!(RiskTier::R3Irreversible.requires_human_approval());
    }

    #[test]
    fn journal_entry_proposal_enforces_double_entry_balance_and_positive_lines() {
        let balanced = JournalEntryProposal {
            id: "je-001".into(),
            entity_ruc: "20123456789".into(),
            fiscal_period: "2026-10".into(),
            risk_tier: RiskTier::R2Material,
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
                    credit: MoneyCents::from_cents(1_800),
                    description: "IGV 18%".into(),
                },
            ],
        };
        assert!(balanced.is_balanced());

        let mut unbalanced = balanced.clone();
        unbalanced.lines[2].credit = MoneyCents::from_cents(1_799);
        assert!(!unbalanced.is_balanced());

        let mut negative = balanced.clone();
        negative.lines[0].debit = MoneyCents::from_cents(-11_800);
        assert!(!negative.is_balanced());
    }

    #[test]
    fn json_rpc_request_serializes_as_valid_ndjson_line() {
        let req = JsonRpcRequest {
            id: 42,
            method: "tools/call".into(),
            params_json: r#"{"name":"ledger_validate"}"#.into(),
        };
        assert_eq!(
            req.to_ndjson(),
            r#"{"jsonrpc":"2.0","id":42,"method":"tools/call","params":{"name":"ledger_validate"}}"#
        );
    }
}
