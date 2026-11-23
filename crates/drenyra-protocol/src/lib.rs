//! Zero-float fiscal protocol contracts and JSON-RPC 2.0 stdio framing for Drenyra GPUI.

use std::fmt;

/// Protocol validation and framing errors.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProtocolError {
    EmptyParams,
    MultilineParams,
    InvalidParamsContainer,
}

impl fmt::Display for ProtocolError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyParams => f.write_str("params_json must not be empty"),
            Self::MultilineParams => f.write_str("params_json must be single-line for NDJSON framing"),
            Self::InvalidParamsContainer => {
                f.write_str("params_json must be a JSON object ({...}) or array ([...])")
            }
        }
    }
}

impl std::error::Error for ProtocolError {}

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
    /// exactly one side (debit XOR credit), and total debits equal total credits without overflow.
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

/// Offline-verifiable execution receipt summary emitted by `drenyra-ai` after human approval.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SignedReceiptSummary {
    pub receipt_id: String,
    pub proposal_id: String,
    pub content_sha256: String,
    pub approved_by: String,
    pub total_amount: MoneyCents,
}

/// Minimal JSON-RPC 2.0 request frame for `drenyra-shell --mode rpc` and `drenyra-ai mcp serve`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JsonRpcRequest {
    pub id: u64,
    pub method: String,
    pub params_json: String,
}

impl JsonRpcRequest {
    /// Serializes the request as a single-line RFC 8259 JSON-RPC 2.0 NDJSON frame.
    pub fn to_ndjson(&self) -> Result<String, ProtocolError> {
        let trimmed = self.params_json.trim();
        if trimmed.is_empty() {
            return Err(ProtocolError::EmptyParams);
        }
        if trimmed.contains('\n') || trimmed.contains('\r') {
            return Err(ProtocolError::MultilineParams);
        }
        validate_json_container(trimmed)?;

        let escaped_method = escape_json_str(&self.method);
        Ok(format!(
            r#"{{"jsonrpc":"2.0","id":{},"method":"{}","params":{}}}"#,
            self.id, escaped_method, trimmed
        ))
    }
}

const MAX_JSON_DEPTH: usize = 512;

/// Validates that `input` is a single complete RFC 8259 JSON object (`{...}`) or array (`[...]`).
pub fn validate_json_container(input: &str) -> Result<(), ProtocolError> {
    let mut validator = JsonValidator::new(input.as_bytes());
    validator.skip_ws();
    match validator.peek() {
        Some(b'{') => validator.parse_object(0)?,
        Some(b'[') => validator.parse_array(0)?,
        _ => return Err(ProtocolError::InvalidParamsContainer),
    }
    validator.skip_ws();
    if validator.pos != validator.bytes.len() {
        return Err(ProtocolError::InvalidParamsContainer);
    }
    Ok(())
}

struct JsonValidator<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl<'a> JsonValidator<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, pos: 0 }
    }

    fn peek(&self) -> Option<u8> {
        self.bytes.get(self.pos).copied()
    }

    fn next(&mut self) -> Option<u8> {
        let byte = self.bytes.get(self.pos).copied()?;
        self.pos += 1;
        Some(byte)
    }

    fn skip_ws(&mut self) {
        while matches!(self.peek(), Some(b' ' | b'\t' | b'\n' | b'\r')) {
            self.pos += 1;
        }
    }

    fn expect_bytes(&mut self, expected: &[u8]) -> Result<(), ProtocolError> {
        if self.bytes.get(self.pos..self.pos + expected.len()) == Some(expected) {
            self.pos += expected.len();
            Ok(())
        } else {
            Err(ProtocolError::InvalidParamsContainer)
        }
    }

    fn parse_value(&mut self, depth: usize) -> Result<(), ProtocolError> {
        if depth >= MAX_JSON_DEPTH {
            return Err(ProtocolError::InvalidParamsContainer);
        }
        self.skip_ws();
        match self.peek() {
            Some(b'{') => self.parse_object(depth + 1),
            Some(b'[') => self.parse_array(depth + 1),
            Some(b'"') => self.parse_string(),
            Some(b't') => self.expect_bytes(b"true"),
            Some(b'f') => self.expect_bytes(b"false"),
            Some(b'n') => self.expect_bytes(b"null"),
            Some(b'-' | b'0'..=b'9') => self.parse_number(),
            _ => Err(ProtocolError::InvalidParamsContainer),
        }
    }

    fn parse_object(&mut self, depth: usize) -> Result<(), ProtocolError> {
        if depth >= MAX_JSON_DEPTH {
            return Err(ProtocolError::InvalidParamsContainer);
        }
        if self.next() != Some(b'{') {
            return Err(ProtocolError::InvalidParamsContainer);
        }
        self.skip_ws();
        if self.peek() == Some(b'}') {
            self.pos += 1;
            return Ok(());
        }
        loop {
            self.skip_ws();
            self.parse_string()?;
            self.skip_ws();
            if self.next() != Some(b':') {
                return Err(ProtocolError::InvalidParamsContainer);
            }
            self.parse_value(depth)?;
            self.skip_ws();
            match self.next() {
                Some(b',') => continue,
                Some(b'}') => return Ok(()),
                _ => return Err(ProtocolError::InvalidParamsContainer),
            }
        }
    }

    fn parse_array(&mut self, depth: usize) -> Result<(), ProtocolError> {
        if depth >= MAX_JSON_DEPTH {
            return Err(ProtocolError::InvalidParamsContainer);
        }
        if self.next() != Some(b'[') {
            return Err(ProtocolError::InvalidParamsContainer);
        }
        self.skip_ws();
        if self.peek() == Some(b']') {
            self.pos += 1;
            return Ok(());
        }
        loop {
            self.parse_value(depth)?;
            self.skip_ws();
            match self.next() {
                Some(b',') => continue,
                Some(b']') => return Ok(()),
                _ => return Err(ProtocolError::InvalidParamsContainer),
            }
        }
    }

    fn parse_string(&mut self) -> Result<(), ProtocolError> {
        if self.next() != Some(b'"') {
            return Err(ProtocolError::InvalidParamsContainer);
        }
        loop {
            match self.next() {
                None => return Err(ProtocolError::InvalidParamsContainer),
                Some(b'"') => return Ok(()),
                Some(b'\\') => match self.next() {
                    Some(b'"' | b'\\' | b'/' | b'b' | b'f' | b'n' | b'r' | b't') => {}
                    Some(b'u') => {
                        for _ in 0..4 {
                            match self.next() {
                                Some(b) if b.is_ascii_hexdigit() => {}
                                _ => return Err(ProtocolError::InvalidParamsContainer),
                            }
                        }
                    }
                    _ => return Err(ProtocolError::InvalidParamsContainer),
                },
                Some(b) if b < 0x20 => return Err(ProtocolError::InvalidParamsContainer),
                Some(_) => {}
            }
        }
    }

    fn parse_number(&mut self) -> Result<(), ProtocolError> {
        if self.peek() == Some(b'-') {
            self.pos += 1;
        }
        match self.next() {
            Some(b'0') => {}
            Some(b'1'..=b'9') => {
                while matches!(self.peek(), Some(b'0'..=b'9')) {
                    self.pos += 1;
                }
            }
            _ => return Err(ProtocolError::InvalidParamsContainer),
        }
        if self.peek() == Some(b'.') {
            self.pos += 1;
            if !matches!(self.peek(), Some(b'0'..=b'9')) {
                return Err(ProtocolError::InvalidParamsContainer);
            }
            while matches!(self.peek(), Some(b'0'..=b'9')) {
                self.pos += 1;
            }
        }
        if matches!(self.peek(), Some(b'e' | b'E')) {
            self.pos += 1;
            if matches!(self.peek(), Some(b'+' | b'-')) {
                self.pos += 1;
            }
            if !matches!(self.peek(), Some(b'0'..=b'9')) {
                return Err(ProtocolError::InvalidParamsContainer);
            }
            while matches!(self.peek(), Some(b'0'..=b'9')) {
                self.pos += 1;
            }
        }
        Ok(())
    }
}

/// Escapes a Rust string into a valid RFC 8259 JSON string body (all control chars < 0x20 escaped).
pub fn escape_json_str(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for ch in input.chars() {
        match ch {
            '"' => out.push_str(r#"\""#),
            '\\' => out.push_str(r#"\\"#),
            '\u{08}' => out.push_str(r#"\b"#),
            '\u{0C}' => out.push_str(r#"\f"#),
            '\n' => out.push_str(r#"\n"#),
            '\r' => out.push_str(r#"\r"#),
            '\t' => out.push_str(r#"\t"#),
            c if (c as u32) < 0x20 => {
                use std::fmt::Write as _;
                let _ = write!(out, "\\u{:04x}", c as u32);
            }
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
        assert_eq!(
            MoneyCents::from_cents(i128::MAX).checked_add(MoneyCents::from_cents(1)),
            None
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
    fn journal_entry_proposal_enforces_double_entry_and_boundary_invariants() {
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

        let single_line = JournalEntryProposal {
            lines: vec![balanced.lines[0].clone()],
            ..balanced.clone()
        };
        assert!(!single_line.is_balanced());

        let degenerate = JournalEntryProposal {
            lines: vec![
                JournalLine {
                    account_code: "1011".into(),
                    debit: MoneyCents::from_cents(100),
                    credit: MoneyCents::from_cents(100),
                    description: "Both sides non-zero".into(),
                },
                JournalLine {
                    account_code: "7011".into(),
                    debit: MoneyCents::ZERO,
                    credit: MoneyCents::ZERO,
                    description: "Zero line".into(),
                },
            ],
            ..balanced.clone()
        };
        assert!(!degenerate.is_balanced());

        let overflow = JournalEntryProposal {
            lines: vec![
                JournalLine {
                    account_code: "1011".into(),
                    debit: MoneyCents::from_cents(i128::MAX),
                    credit: MoneyCents::ZERO,
                    description: "Max debit".into(),
                },
                JournalLine {
                    account_code: "1012".into(),
                    debit: MoneyCents::from_cents(1),
                    credit: MoneyCents::ZERO,
                    description: "Overflow debit".into(),
                },
            ],
            ..balanced
        };
        assert!(!overflow.is_balanced());
    }

    #[test]
    fn json_rpc_request_serializes_valid_ndjson_escapes_controls_and_rejects_bad_params() {
        let req = JsonRpcRequest {
            id: 42,
            method: "tools/call".into(),
            params_json: r#"{"name":"ledger_validate"}"#.into(),
        };
        assert_eq!(
            req.to_ndjson().expect("valid frame"),
            r#"{"jsonrpc":"2.0","id":42,"method":"tools/call","params":{"name":"ledger_validate"}}"#
        );

        let control_req = JsonRpcRequest {
            id: 7,
            method: "a\"b\\c\n\u{0000}\u{001f}".into(),
            params_json: "[]".into(),
        };
        assert_eq!(
            control_req.to_ndjson().expect("escaped control chars"),
            r#"{"jsonrpc":"2.0","id":7,"method":"a\"b\\c\n\u0000\u001f","params":[]}"#
        );

        assert_eq!(
            JsonRpcRequest {
                id: 1,
                method: "ping".into(),
                params_json: "   ".into(),
            }
            .to_ndjson(),
            Err(ProtocolError::EmptyParams)
        );
        assert_eq!(
            JsonRpcRequest {
                id: 1,
                method: "ping".into(),
                params_json: "{\n\"a\":1\n}".into(),
            }
            .to_ndjson(),
            Err(ProtocolError::MultilineParams)
        );
        assert_eq!(
            JsonRpcRequest {
                id: 1,
                method: "ping".into(),
                params_json: "not-json".into(),
            }
            .to_ndjson(),
            Err(ProtocolError::InvalidParamsContainer)
        );
    }

    #[test]
    fn json_validator_rejects_malformed_containers_injections_and_control_bytes() {
        let invalid_cases = [
            // Unbalanced braces/brackets
            r#"{"a":1"#,
            r#"{"a":1}}"#,
            r#"[1,2"#,
            r#"[1,2]]"#,
            r#"{"a":[1,2}"#,
            // Trailing commas
            r#"{"a":1,}"#,
            r#"[1,2,]"#,
            r#"{"a":[1,]}"#,
            // Unclosed strings / bad escapes
            r#"{"a":"unclosed}"#,
            r#"{"a":"bad\x41"}"#,
            // Sibling-injection / multiple top-level objects / trailing garbage
            r#"{"a":1},"injected":2"#,
            r#"{"a":1}{"b":2}"#,
            r#"[1][2]"#,
            r#"{"a":1} trailing"#,
            // Invalid numbers and non-container top-level values
            r#"{"a":01}"#,
            r#"{"a":1.}"#,
            r#"{"a":.5}"#,
            r#"{"a":1e}"#,
            r#"{"a":+1}"#,
            r#""string""#,
            r#"123"#,
            r#"true"#,
            r#"null"#,
        ];

        for bad in invalid_cases {
            assert_eq!(
                validate_json_container(bad),
                Err(ProtocolError::InvalidParamsContainer),
                "expected validate_json_container to reject {bad:?}"
            );
            assert_eq!(
                JsonRpcRequest {
                    id: 1,
                    method: "test".into(),
                    params_json: bad.into(),
                }
                .to_ndjson(),
                Err(ProtocolError::InvalidParamsContainer),
                "expected to_ndjson to reject {bad:?}"
            );
        }

        // All 32 ASCII control bytes (< 0x20) unescaped inside JSON strings must be rejected.
        for byte in 0u8..0x20 {
            let ch = byte as char;
            let payload = format!(r#"{{"k":"{ch}"}}"#);
            assert_eq!(
                validate_json_container(&payload),
                Err(ProtocolError::InvalidParamsContainer),
                "expected validate_json_container to reject control byte 0x{byte:02x}"
            );
            let expected_ndjson_err = if ch == '\n' || ch == '\r' {
                ProtocolError::MultilineParams
            } else {
                ProtocolError::InvalidParamsContainer
            };
            assert_eq!(
                JsonRpcRequest {
                    id: 1,
                    method: "test".into(),
                    params_json: payload,
                }
                .to_ndjson(),
                Err(expected_ndjson_err),
                "expected to_ndjson to reject control byte 0x{byte:02x}"
            );

            // And escape_json_str must produce a valid escaped string for every control byte < 0x20.
            let escaped = escape_json_str(&ch.to_string());
            let wrapped = format!(r#"{{"k":"{escaped}"}}"#);
            assert_eq!(
                validate_json_container(&wrapped),
                Ok(()),
                "expected escaped control byte 0x{byte:02x} ({wrapped}) to validate"
            );
        }

        // Valid containers must pass.
        for valid in [
            r#"{}"#,
            r#"[]"#,
            r#"{"a":1,"b":[-0,0,12.34e-5,true,false,null],"c":["x\"y\\z\/\b\f\n\r\t\u0041"]}"#,
            r#"  [ { "nested" : [ 1 , 2 , 3 ] } ]  "#,
        ] {
            assert_eq!(
                validate_json_container(valid),
                Ok(()),
                "expected validate_json_container to accept {valid:?}"
            );
        }
    }
}
