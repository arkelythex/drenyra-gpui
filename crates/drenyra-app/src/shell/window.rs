//! Native GPUI window: renders [`ShellViewModel`] and maps key bindings to [`ShellCommand`]s.
//!
//! This module holds no domain logic. Everything it shows comes from the view model and every
//! key it handles goes through [`ShellState::apply`], both unit-tested without a display.

use std::cell::RefCell;
use std::rc::Rc;

use gpui::{
    actions, div, prelude::*, px, rgb, size, App, Application, Bounds, Context, FocusHandle,
    KeyBinding, Rgba, SharedString, Window, WindowBounds, WindowOptions,
};

use super::commands::{ShellCommand, ShellState, KEY_BINDINGS};
use super::view_model::{
    pane_area, GuardrailChips, PaneTile, ReviewPanel, RowDecision, ShellViewModel, WorkspaceChip,
    FOOTER_HEIGHT, REVIEW_PANEL_WIDTH, TOP_BAR_HEIGHT,
};
use crate::ui::DesktopShellConfig;
use drenyra_workspace::WorkspaceActivity;

actions!(
    drenyra,
    [
        SplitFocused,
        CloseFocused,
        UndoClose,
        FocusNextPane,
        NextWorkspace,
        ReviewCursorDown,
        ReviewCursorUp,
        AcceptLine,
        RejectLine,
        Quit
    ]
);

const BG: u32 = 0x0f1216;
const SURFACE: u32 = 0x171b21;
const BORDER: u32 = 0x2a313b;
const TEXT: u32 = 0xe6e9ee;
const MUTED: u32 = 0x8b95a3;
const ACCENT: u32 = 0x4fa3ff;
const AMBER: u32 = 0xf0b44c;
const GREEN: u32 = 0x4cc38a;
const RED: u32 = 0xef6b6b;

/// Keys shown in the footer, in display order.
const FOOTER_HINT: &str = "Ctrl+D split · Ctrl+W close · Ctrl+Shift+T undo · Ctrl+Tab next pane · Ctrl+K next workspace · J/K line · A accept · R reject · Ctrl+Q quit";

/// Opens the native window and runs the GPUI event loop until the window closes.
///
/// Returns `Err` when the window could not be opened, so `--window` exits non-zero instead of
/// reporting success after the event loop quits.
pub fn run(config: DesktopShellConfig, state: ShellState) -> Result<(), String> {
    let open_error: Rc<RefCell<Option<String>>> = Rc::new(RefCell::new(None));
    let open_error_slot = Rc::clone(&open_error);
    Application::new().run(move |cx: &mut App| {
        cx.bind_keys(key_bindings());
        cx.bind_keys([KeyBinding::new("ctrl-q", Quit, None)]);
        cx.on_action(|_: &Quit, cx| cx.quit());
        cx.on_window_closed(|cx| {
            if cx.windows().is_empty() {
                cx.quit();
            }
        })
        .detach();

        let bounds = Bounds::centered(
            None,
            size(
                px(config.window_width as f32),
                px(config.window_height as f32),
            ),
            cx,
        );
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            app_id: Some("drenyra-gpui".into()),
            ..Default::default()
        };
        let opened = cx.open_window(options, |window, cx| {
            window.set_window_title(&config.app_title);
            cx.new(|cx| {
                let focus_handle = cx.focus_handle();
                focus_handle.focus(window);
                ShellWindow {
                    state,
                    focus_handle,
                }
            })
        });
        if let Err(err) = opened {
            *open_error_slot.borrow_mut() = Some(format!("failed to open native window: {err}"));
            cx.quit();
            return;
        }
        cx.activate(true);
    });
    let error = open_error.borrow_mut().take();
    error.map_or(Ok(()), Err)
}

/// GPUI key bindings derived from [`KEY_BINDINGS`]; the match is exhaustive, so every command
/// has exactly one action type.
fn key_bindings() -> Vec<KeyBinding> {
    KEY_BINDINGS
        .iter()
        .map(|&(key, command)| match command {
            ShellCommand::SplitFocused => KeyBinding::new(key, SplitFocused, None),
            ShellCommand::CloseFocused => KeyBinding::new(key, CloseFocused, None),
            ShellCommand::UndoClose => KeyBinding::new(key, UndoClose, None),
            ShellCommand::FocusNextPane => KeyBinding::new(key, FocusNextPane, None),
            ShellCommand::NextWorkspace => KeyBinding::new(key, NextWorkspace, None),
            ShellCommand::ReviewCursorDown => KeyBinding::new(key, ReviewCursorDown, None),
            ShellCommand::ReviewCursorUp => KeyBinding::new(key, ReviewCursorUp, None),
            ShellCommand::AcceptLine => KeyBinding::new(key, AcceptLine, None),
            ShellCommand::RejectLine => KeyBinding::new(key, RejectLine, None),
        })
        .collect()
}

struct ShellWindow {
    state: ShellState,
    focus_handle: FocusHandle,
}

impl ShellWindow {
    fn dispatch(&mut self, command: ShellCommand, window: &mut Window, cx: &mut Context<Self>) {
        let area = self.pane_area(window);
        if self.state.apply(command, area) {
            cx.notify();
        }
    }

    fn pane_area(&self, window: &Window) -> super::view_model::Rect {
        let viewport = window.viewport_size();
        pane_area(
            f32::from(viewport.width).max(0.0) as u32,
            f32::from(viewport.height).max(0.0) as u32,
            self.state.controller().active_review().is_some(),
        )
    }
}

impl Render for ShellWindow {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let vm = self.state.view(self.pane_area(window));

        div()
            .track_focus(&self.focus_handle)
            .on_action(cx.listener(|this, _: &SplitFocused, w, cx| {
                this.dispatch(ShellCommand::SplitFocused, w, cx)
            }))
            .on_action(cx.listener(|this, _: &CloseFocused, w, cx| {
                this.dispatch(ShellCommand::CloseFocused, w, cx)
            }))
            .on_action(cx.listener(|this, _: &UndoClose, w, cx| {
                this.dispatch(ShellCommand::UndoClose, w, cx)
            }))
            .on_action(cx.listener(|this, _: &FocusNextPane, w, cx| {
                this.dispatch(ShellCommand::FocusNextPane, w, cx)
            }))
            .on_action(cx.listener(|this, _: &NextWorkspace, w, cx| {
                this.dispatch(ShellCommand::NextWorkspace, w, cx)
            }))
            .on_action(cx.listener(|this, _: &ReviewCursorDown, w, cx| {
                this.dispatch(ShellCommand::ReviewCursorDown, w, cx)
            }))
            .on_action(cx.listener(|this, _: &ReviewCursorUp, w, cx| {
                this.dispatch(ShellCommand::ReviewCursorUp, w, cx)
            }))
            .on_action(cx.listener(|this, _: &AcceptLine, w, cx| {
                this.dispatch(ShellCommand::AcceptLine, w, cx)
            }))
            .on_action(cx.listener(|this, _: &RejectLine, w, cx| {
                this.dispatch(ShellCommand::RejectLine, w, cx)
            }))
            .size_full()
            .overflow_hidden()
            .flex()
            .flex_col()
            .bg(rgb(BG))
            .text_color(rgb(TEXT))
            .text_sm()
            .child(top_bar(&vm))
            .child(
                div()
                    .flex_1()
                    .flex()
                    .flex_row()
                    .min_h_0()
                    .overflow_hidden()
                    .child(pane_layer(&vm.panes))
                    .children(vm.review.as_ref().map(review_panel)),
            )
            .child(footer())
    }
}

fn top_bar(vm: &ShellViewModel) -> impl IntoElement {
    div()
        .h(px(TOP_BAR_HEIGHT as f32))
        .flex_none()
        .flex()
        .flex_row()
        .items_center()
        .justify_between()
        .px_3()
        .gap_2()
        .overflow_hidden()
        .bg(rgb(SURFACE))
        .border_b_1()
        .border_color(rgb(BORDER))
        .child(
            // Workspace chips give way first; guardrail chips must always stay visible.
            div()
                .flex_1()
                .min_w_0()
                .overflow_hidden()
                .flex()
                .flex_row()
                .gap_2()
                .children(vm.workspaces.iter().map(workspace_chip)),
        )
        .child(
            div()
                .flex_none()
                .child(guardrail_chips(&vm.guardrails, vm.demo)),
        )
}

fn activity_color(activity: WorkspaceActivity) -> Rgba {
    match activity {
        WorkspaceActivity::Idle => rgb(MUTED),
        WorkspaceActivity::Reconciling => rgb(ACCENT),
        WorkspaceActivity::AwaitingApproval => rgb(AMBER),
    }
}

fn dot(color: Rgba) -> impl IntoElement {
    div().size(px(8.0)).rounded_full().bg(color)
}

fn chip(label: impl Into<SharedString>, color: Rgba) -> gpui::Div {
    div()
        .flex()
        .flex_row()
        .items_center()
        .gap_1()
        .px_2()
        .py_0p5()
        .rounded_md()
        .border_1()
        .border_color(rgb(BORDER))
        .text_color(color)
        .whitespace_nowrap()
        .child(label.into())
}

fn workspace_chip(ws: &WorkspaceChip) -> impl IntoElement {
    let label = format!(
        "{} · {} · {}",
        ws.legal_name, ws.entity_ruc, ws.fiscal_period
    );
    let base = chip(label, if ws.active { rgb(TEXT) } else { rgb(MUTED) });
    let base = if ws.active {
        base.border_color(rgb(ACCENT))
    } else {
        base
    };
    base.child(dot(activity_color(ws.activity)))
}

fn guardrail_chips(g: &GuardrailChips, demo: bool) -> impl IntoElement {
    let on_off = |on: bool| if on { "on" } else { "off" };
    let unposted_color = if g.unposted_entries > 0 {
        rgb(AMBER)
    } else {
        rgb(MUTED)
    };
    div()
        .flex()
        .flex_row()
        .gap_2()
        .items_center()
        .children(demo.then(|| chip("DEMO DATA", rgb(AMBER))))
        .child(chip(format!("RDA {}", on_off(g.rda_enabled)), rgb(GREEN)))
        .child(chip(
            format!("Guardian Angel {}", on_off(g.guardian_angel_active)),
            rgb(GREEN),
        ))
        .child(chip(g.risk_tier_label, rgb(TEXT)))
        .child(chip(
            format!("Unposted {}", g.unposted_entries),
            unposted_color,
        ))
        .child(chip("Other workspaces", rgb(MUTED)).child(dot(activity_color(g.background_alert))))
}

fn pane_layer(panes: &[PaneTile]) -> impl IntoElement {
    div()
        .flex_1()
        .relative()
        .min_w_0()
        .children(panes.iter().map(pane_tile))
}

fn pane_tile(tile: &PaneTile) -> impl IntoElement {
    let border = if tile.focused { ACCENT } else { BORDER };
    div()
        .absolute()
        .left(px(tile.rect.x as f32))
        .top(px(tile.rect.y as f32))
        .w(px(tile.rect.width as f32))
        .h(px(tile.rect.height as f32))
        .p_1()
        .child(
            div()
                .size_full()
                .flex()
                .flex_col()
                .gap_1()
                .p_3()
                .rounded_md()
                .bg(rgb(SURFACE))
                .border_1()
                .border_color(rgb(border))
                .child(div().text_color(rgb(TEXT)).child(tile.title))
                .child(
                    div()
                        .text_xs()
                        .text_color(rgb(MUTED))
                        .child("Drenyra Shell RPC not connected (demo data)"),
                ),
        )
}

fn review_panel(panel: &ReviewPanel) -> impl IntoElement {
    let status = if panel.can_post {
        ("Ready to post", GREEN)
    } else if !panel.balanced {
        ("Blocked: entry is not balanced", RED)
    } else {
        ("Blocked: decide every line and record approval", AMBER)
    };
    div()
        .w(px(REVIEW_PANEL_WIDTH as f32))
        .flex_none()
        .flex()
        .flex_col()
        .gap_2()
        .p_3()
        .bg(rgb(SURFACE))
        .border_l_1()
        .border_color(rgb(BORDER))
        .child(
            div()
                .flex()
                .flex_row()
                .justify_between()
                .child(div().text_color(rgb(TEXT)).child("Entry Diff Review"))
                .child(chip(panel.risk_tier_label, rgb(AMBER))),
        )
        .child(
            div()
                .text_xs()
                .text_color(rgb(MUTED))
                .child(panel.proposal_id.clone()),
        )
        .child(review_header())
        .children(panel.rows.iter().map(review_row))
        .child(
            div()
                .flex()
                .flex_row()
                .border_t_1()
                .border_color(rgb(BORDER))
                .pt_1()
                .child(div().flex_1().text_color(rgb(MUTED)).child("Totals"))
                .child(money_cell(panel.total_debit.clone()))
                .child(money_cell(panel.total_credit.clone())),
        )
        .child(chip(status.0, rgb(status.1)))
        .children(
            panel
                .approved_by
                .as_ref()
                .map(|who| chip(format!("Approved by {who}"), rgb(GREEN))),
        )
}

fn review_header() -> impl IntoElement {
    div()
        .flex()
        .flex_row()
        .text_xs()
        .text_color(rgb(MUTED))
        .child(div().w(px(20.0)))
        .child(div().flex_1().child("Account"))
        .child(money_cell("Debit".to_owned()))
        .child(money_cell("Credit".to_owned()))
}

fn money_cell(text: String) -> impl IntoElement {
    div().w(px(88.0)).flex().justify_end().child(text)
}

fn review_row(row: &super::view_model::ReviewRow) -> impl IntoElement {
    let (mark, color) = match &row.decision {
        RowDecision::Pending => ("○", MUTED),
        RowDecision::Accepted => ("✓", GREEN),
        RowDecision::Rejected { .. } => ("✕", RED),
    };
    let base = div()
        .flex()
        .flex_row()
        .items_center()
        .px_1()
        .rounded_sm()
        .child(div().w(px(20.0)).text_color(rgb(color)).child(mark))
        .child(
            div()
                .flex_1()
                .flex()
                .flex_col()
                .child(row.account_code.clone())
                .child(
                    div()
                        .text_xs()
                        .text_color(rgb(MUTED))
                        .child(row.description.clone()),
                ),
        )
        .child(money_cell(row.debit.clone()))
        .child(money_cell(row.credit.clone()));
    if row.cursor {
        base.bg(rgb(BORDER))
    } else {
        base
    }
}

fn footer() -> impl IntoElement {
    div()
        .h(px(FOOTER_HEIGHT as f32))
        .flex_none()
        .flex()
        .items_center()
        .px_3()
        .overflow_hidden()
        .text_xs()
        .text_color(rgb(MUTED))
        .bg(rgb(SURFACE))
        .border_t_1()
        .border_color(rgb(BORDER))
        .child(div().min_w_0().truncate().child(FOOTER_HINT))
}
