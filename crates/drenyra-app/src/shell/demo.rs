//! Deterministic demo fixture shared by `--demo-session` and the native window.

use drenyra_protocol::{JournalEntryProposal, JournalLine, MoneyCents, RiskTier};
use drenyra_workspace::FiscalWorkspace;

use crate::controller::{Clock, DrenyraHostController};

/// Primary demo entity: RUC, legal name, fiscal period.
pub const DEMO_ENTITY: (&str, &str, &str) = ("20123456789", "Andino SAC", "2026-10");

/// Balanced R2 sale with 18% IGV for the primary demo entity.
pub fn demo_proposal() -> JournalEntryProposal {
    let (ruc, _, period) = DEMO_ENTITY;
    JournalEntryProposal {
        id: "je-demo-2026-10-001".into(),
        entity_ruc: ruc.into(),
        fiscal_period: period.into(),
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
    }
}

/// Primary demo workspace without any staged review.
pub fn demo_workspace() -> FiscalWorkspace {
    let (ruc, name, period) = DEMO_ENTITY;
    FiscalWorkspace::new(ruc, name, period)
}

/// Controller with Andino SAC (staged R2 proposal) and Sierra SRL (idle); evidence times come
/// from the system clock.
pub fn demo_controller() -> DrenyraHostController {
    demo_controller_with_clock(Clock::System)
}

/// Like [`demo_controller`], stamping evidence events (the staging included) with `clock`.
pub fn demo_controller_with_clock(clock: Clock) -> DrenyraHostController {
    let mut controller = DrenyraHostController::with_clock(demo_workspace(), clock);
    controller
        .workspaces_mut()
        .add_workspace(FiscalWorkspace::new("20987654321", "Sierra SRL", "2026-11"))
        .expect("demo workspaces are distinct");
    controller
        .stage_proposal(demo_proposal())
        .expect("demo proposal matches the demo workspace scope");
    controller
}
