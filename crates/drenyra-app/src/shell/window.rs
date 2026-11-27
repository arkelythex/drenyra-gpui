//! Native GPUI window: renders [`ShellViewModel`] and maps key bindings to [`ShellCommand`]s.
//!
//! This module holds no domain logic. Everything it shows comes from the view model and every
//! key it handles goes through [`ShellState::apply`], both unit-tested without a display. Every
//! color comes from the active theme's [`ThemeTokens`]; this module defines none of its own.

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

use gpui::{
    actions, div, prelude::*, px, rgb, size, App, Application, Bounds, ClickEvent, Context,
    FocusHandle, KeyBinding, Rgba, SharedString, Window, WindowBounds, WindowOptions,
};

use super::commands::{ShellCommand, ShellState, KEY_BINDINGS};
use super::settings::save_if_changed;
use super::theme::{ThemeId, ThemeTokens};
use super::view_model::{
    pane_area, GuardrailChips, PaneTile, ReviewPanel, RowDecision, SettingsPanel, ShellViewModel,
    ThemeOption, WorkspaceChip, FOOTER_HEIGHT, REVIEW_PANEL_WIDTH, TOP_BAR_HEIGHT,
};
use crate::ui::DesktopShellConfig;
use drenyra_workspace::{PaneKind, WorkspaceActivity};

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
        OpenSettings,
        CloseOverlay,
        Confirm,
        Quit
    ]
);

/// Monospace family for amounts and identifiers (installed with the Dreamcoder setup).
const MONO_FONT: &str = "JetBrainsMono Nerd Font";

/// Keys shown in the footer, in display order.
const FOOTER_HINT: &str = "Ctrl+D split · Ctrl+W close · Ctrl+Shift+T undo · Ctrl+Tab next pane · Ctrl+K next workspace · J/K line · A accept · R reject · Ctrl+, settings · Ctrl+Q quit";
/// Footer keys while the Settings overlay is open.
const SETTINGS_HINT: &str =
    "J/K choose theme · Enter apply · Esc close · click a theme to apply it";

/// Opens the native window and runs the GPUI event loop until the window closes.
///
/// `settings_path` is where theme changes are saved; `None` keeps them for this session only.
/// Returns `Err` when the window could not be opened, so `--window` exits non-zero instead of
/// reporting success after the event loop quits.
pub fn run(
    config: DesktopShellConfig,
    state: ShellState,
    settings_path: Option<PathBuf>,
) -> Result<(), String> {
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
                    settings_path,
                    save_error: None,
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
            ShellCommand::OpenSettings => KeyBinding::new(key, OpenSettings, None),
            ShellCommand::CloseOverlay => KeyBinding::new(key, CloseOverlay, None),
            ShellCommand::Confirm => KeyBinding::new(key, Confirm, None),
        })
        .collect()
}

struct ShellWindow {
    state: ShellState,
    focus_handle: FocusHandle,
    settings_path: Option<PathBuf>,
    /// Last failure to save settings, shown in the footer until a save succeeds.
    save_error: Option<String>,
}

impl ShellWindow {
    fn dispatch(&mut self, command: ShellCommand, window: &mut Window, cx: &mut Context<Self>) {
        let area = self.pane_area(window);
        let before = self.state.settings();
        if self.state.apply(command, area) {
            self.persist_if_changed(before);
            cx.notify();
        }
    }

    fn select_theme(&mut self, theme: ThemeId, cx: &mut Context<Self>) {
        let before = self.state.settings();
        if self.state.select_theme(theme) {
            self.persist_if_changed(before);
            cx.notify();
        }
    }

    fn persist_if_changed(&mut self, before: super::settings::Settings) {
        match save_if_changed(before, self.state.settings(), self.settings_path.as_deref()) {
            Ok(true) => self.save_error = None,
            Ok(false) => {}
            Err(message) => self.save_error = Some(message),
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
        let t = vm.theme.tokens();

        let settings_button = div()
            .id("open-settings")
            .cursor_pointer()
            .on_click(cx.listener(|this, _: &ClickEvent, w, cx| {
                this.dispatch(ShellCommand::OpenSettings, w, cx)
            }))
            .child(chip("⚙ Settings", t.text_muted, &t));

        let overlay = vm.settings.as_ref().map(|panel| {
            let options = panel.themes.iter().map(|option| {
                let id = option.id;
                theme_card(option, &t).on_click(
                    cx.listener(move |this, _: &ClickEvent, _, cx| this.select_theme(id, cx)),
                )
            });
            let close = div()
                .id("close-settings")
                .cursor_pointer()
                .on_click(cx.listener(|this, _: &ClickEvent, w, cx| {
                    this.dispatch(ShellCommand::CloseOverlay, w, cx)
                }))
                .child(chip("Esc  Close", t.text_muted, &t));
            settings_overlay(panel, options.collect(), close, &t)
        });

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
            .on_action(cx.listener(|this, _: &OpenSettings, w, cx| {
                this.dispatch(ShellCommand::OpenSettings, w, cx)
            }))
            .on_action(cx.listener(|this, _: &CloseOverlay, w, cx| {
                this.dispatch(ShellCommand::CloseOverlay, w, cx)
            }))
            .on_action(
                cx.listener(|this, _: &Confirm, w, cx| this.dispatch(ShellCommand::Confirm, w, cx)),
            )
            .size_full()
            .relative()
            .overflow_hidden()
            .flex()
            .flex_col()
            .bg(rgb(t.canvas))
            .text_color(rgb(t.text))
            .text_sm()
            .child(top_bar(&vm, settings_button, &t))
            .child(
                div()
                    .flex_1()
                    .flex()
                    .flex_row()
                    .min_h_0()
                    .overflow_hidden()
                    .child(pane_layer(&vm.panes, &t))
                    .children(vm.review.as_ref().map(|panel| review_panel(panel, &t))),
            )
            .child(footer(
                vm.settings.is_some(),
                self.save_error.as_deref(),
                &t,
            ))
            .children(overlay)
    }
}

fn top_bar(vm: &ShellViewModel, settings_button: impl IntoElement, t: &ThemeTokens) -> gpui::Div {
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
        .bg(rgb(t.chrome))
        .border_b_1()
        .border_color(rgb(t.divider))
        .child(
            // Workspace chips give way first; guardrail chips must always stay visible.
            div()
                .flex_1()
                .min_w_0()
                .overflow_hidden()
                .flex()
                .flex_row()
                .gap_2()
                .children(vm.workspaces.iter().map(|ws| workspace_chip(ws, t))),
        )
        .child(
            div()
                .flex_none()
                .flex()
                .flex_row()
                .gap_2()
                .items_center()
                .child(guardrail_chips(&vm.guardrails, vm.demo, t))
                .child(settings_button),
        )
}

fn activity_color(activity: WorkspaceActivity, t: &ThemeTokens) -> Rgba {
    match activity {
        WorkspaceActivity::Idle => rgb(t.text_subtle),
        WorkspaceActivity::Reconciling => rgb(t.info),
        WorkspaceActivity::AwaitingApproval => rgb(t.warning),
    }
}

fn dot(color: Rgba) -> impl IntoElement {
    div().size(px(8.0)).rounded_full().bg(color)
}

/// Rounded label on the strong surface; `color` is the text color token.
fn chip(label: impl Into<SharedString>, color: u32, t: &ThemeTokens) -> gpui::Div {
    div()
        .flex()
        .flex_row()
        .items_center()
        .gap_1()
        .px_2()
        .py_0p5()
        .rounded_md()
        .bg(rgb(t.surface_strong))
        .border_1()
        .border_color(rgb(t.divider))
        .text_color(rgb(color))
        .whitespace_nowrap()
        .child(label.into())
}

fn workspace_chip(ws: &WorkspaceChip, t: &ThemeTokens) -> impl IntoElement {
    let label = format!(
        "{} · {} · {}",
        ws.legal_name, ws.entity_ruc, ws.fiscal_period
    );
    let base = chip(label, if ws.active { t.text } else { t.text_subtle }, t);
    let base = if ws.active {
        base.border_color(rgb(t.accent))
    } else {
        base
    };
    base.child(dot(activity_color(ws.activity, t)))
}

fn guardrail_chips(g: &GuardrailChips, demo: bool, t: &ThemeTokens) -> impl IntoElement {
    let on_off = |on: bool| if on { "on" } else { "off" };
    let unposted_color = if g.unposted_entries > 0 {
        t.warning
    } else {
        t.text_subtle
    };
    div()
        .flex()
        .flex_row()
        .gap_2()
        .items_center()
        .children(demo.then(|| chip("DEMO DATA", t.warning, t)))
        .child(chip(format!("RDA {}", on_off(g.rda_enabled)), t.success, t))
        .child(chip(
            format!("Guardian Angel {}", on_off(g.guardian_angel_active)),
            t.success,
            t,
        ))
        .child(chip(g.risk_tier_label, t.text, t))
        .child(chip(
            format!("Unposted {}", g.unposted_entries),
            unposted_color,
            t,
        ))
        .child(
            chip("Other workspaces", t.text_subtle, t)
                .child(dot(activity_color(g.background_alert, t))),
        )
}

fn pane_layer(panes: &[PaneTile], t: &ThemeTokens) -> impl IntoElement {
    div()
        .flex_1()
        .relative()
        .min_w_0()
        .children(panes.iter().map(|tile| pane_tile(tile, t)))
}

/// One-line role description shown under each pane title.
fn pane_caption(kind: PaneKind) -> &'static str {
    match kind {
        PaneKind::FiscalAgent => "Drenyra Shell agent session",
        PaneKind::LedgerViewer => "Ledger and trial balance",
        PaneKind::EvidenceVault => "Source vouchers: XML, PDF, CDR, bank statements",
        PaneKind::FiscalCli => "Fiscal command line",
    }
}

fn pane_tile(tile: &PaneTile, t: &ThemeTokens) -> impl IntoElement {
    let border = if tile.focused { t.focus } else { t.divider };
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
                .rounded_lg()
                .overflow_hidden()
                .bg(rgb(t.surface))
                .border_1()
                .border_color(rgb(border))
                .child(
                    div()
                        .flex()
                        .flex_row()
                        .items_center()
                        .justify_between()
                        .gap_2()
                        .px_3()
                        .py_2()
                        .bg(rgb(t.surface_raised))
                        .border_b_1()
                        .border_color(rgb(t.divider))
                        .child(
                            div()
                                .flex()
                                .flex_col()
                                .min_w_0()
                                .child(
                                    div()
                                        .text_color(rgb(t.text_heading))
                                        .font_weight(gpui::FontWeight::SEMIBOLD)
                                        .child(tile.title),
                                )
                                .child(
                                    div()
                                        .text_xs()
                                        .text_color(rgb(t.text_subtle))
                                        .truncate()
                                        .child(pane_caption(tile.kind)),
                                ),
                        )
                        .children(tile.focused.then(|| chip("focused", t.accent, t))),
                )
                .child(
                    div()
                        .flex_1()
                        .p_3()
                        .text_xs()
                        .text_color(rgb(t.text_muted))
                        .child("Not connected yet: showing demo data."),
                ),
        )
}

fn review_panel(panel: &ReviewPanel, t: &ThemeTokens) -> impl IntoElement {
    let status = if panel.can_post {
        ("Ready to post", t.success)
    } else if !panel.balanced {
        ("Blocked: entry is not balanced", t.error)
    } else {
        ("Blocked: decide every line and record approval", t.warning)
    };
    div()
        .w(px(REVIEW_PANEL_WIDTH as f32))
        .flex_none()
        .flex()
        .flex_col()
        .gap_2()
        .p_3()
        .bg(rgb(t.surface))
        .border_l_1()
        .border_color(rgb(t.divider))
        .child(
            div()
                .flex()
                .flex_row()
                .justify_between()
                .items_center()
                .child(
                    div()
                        .text_color(rgb(t.text_heading))
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .child("Entry Diff Review"),
                )
                .child(chip(panel.risk_tier_label, t.warning, t)),
        )
        .child(
            div()
                .text_xs()
                .font_family(MONO_FONT)
                .text_color(rgb(t.text_subtle))
                .child(panel.proposal_id.clone()),
        )
        .child(review_header(t))
        .children(panel.rows.iter().map(|row| review_row(row, t)))
        .child(
            div()
                .flex()
                .flex_row()
                .border_t_1()
                .border_color(rgb(t.divider))
                .pt_1()
                .child(div().flex_1().text_color(rgb(t.text_muted)).child("Totals"))
                .child(money_cell(panel.total_debit.clone()))
                .child(money_cell(panel.total_credit.clone())),
        )
        .child(chip(status.0, status.1, t))
        .children(
            panel
                .approved_by
                .as_ref()
                .map(|who| chip(format!("Approved by {who}"), t.success, t)),
        )
}

fn review_header(t: &ThemeTokens) -> impl IntoElement {
    div()
        .flex()
        .flex_row()
        .text_xs()
        .text_color(rgb(t.text_subtle))
        .child(div().w(px(20.0)))
        .child(div().flex_1().child("Account"))
        .child(div().w(px(88.0)).flex().justify_end().child("Debit"))
        .child(div().w(px(88.0)).flex().justify_end().child("Credit"))
}

fn money_cell(text: String) -> impl IntoElement {
    div()
        .w(px(88.0))
        .flex()
        .justify_end()
        .font_family(MONO_FONT)
        .child(text)
}

fn review_row(row: &super::view_model::ReviewRow, t: &ThemeTokens) -> impl IntoElement {
    let (mark, color) = match &row.decision {
        RowDecision::Pending => ("○", t.text_subtle),
        RowDecision::Accepted => ("✓", t.success),
        RowDecision::Rejected { .. } => ("✕", t.error),
    };
    let base = div()
        .flex()
        .flex_row()
        .items_center()
        .px_1()
        .py_0p5()
        .rounded_md()
        .child(div().w(px(20.0)).text_color(rgb(color)).child(mark))
        .child(
            div()
                .flex_1()
                .flex()
                .flex_col()
                .child(div().font_family(MONO_FONT).child(row.account_code.clone()))
                .child(
                    div()
                        .text_xs()
                        .text_color(rgb(if row.cursor {
                            t.on_selection
                        } else {
                            t.text_subtle
                        }))
                        .child(row.description.clone()),
                ),
        )
        .child(money_cell(row.debit.clone()))
        .child(money_cell(row.credit.clone()));
    if row.cursor {
        base.bg(rgb(t.selection)).text_color(rgb(t.on_selection))
    } else {
        base
    }
}

fn footer(settings_open: bool, save_error: Option<&str>, t: &ThemeTokens) -> impl IntoElement {
    let (text, color) = match save_error {
        Some(err) => (err.to_owned(), t.error),
        None if settings_open => (SETTINGS_HINT.to_owned(), t.text_subtle),
        None => (FOOTER_HINT.to_owned(), t.text_subtle),
    };
    div()
        .h(px(FOOTER_HEIGHT as f32))
        .flex_none()
        .flex()
        .items_center()
        .px_3()
        .overflow_hidden()
        .text_xs()
        .text_color(rgb(color))
        .bg(rgb(t.chrome))
        .border_t_1()
        .border_color(rgb(t.divider))
        .child(div().min_w_0().truncate().child(text))
}

/// Modal Settings overlay: section navigation on the left, the Themes section on the right.
fn settings_overlay(
    panel: &SettingsPanel,
    theme_cards: Vec<gpui::Stateful<gpui::Div>>,
    close: impl IntoElement,
    t: &ThemeTokens,
) -> impl IntoElement {
    let nav = div()
        .w(px(180.0))
        .flex_none()
        .flex()
        .flex_col()
        .gap_1()
        .p_3()
        .bg(rgb(t.chrome))
        .border_r_1()
        .border_color(rgb(t.divider))
        .child(
            div()
                .px_2()
                .pb_2()
                .text_color(rgb(t.text_heading))
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .child("Settings"),
        )
        .children(panel.sections.iter().enumerate().map(|(index, section)| {
            let active = index == panel.active_section;
            let item = div()
                .px_2()
                .py_1()
                .rounded_md()
                .text_color(rgb(if active { t.text } else { t.text_subtle }))
                .child(*section);
            if active {
                item.bg(rgb(t.surface_strong))
            } else {
                item
            }
        }));

    let content = div()
        .flex_1()
        .min_w_0()
        .flex()
        .flex_col()
        .gap_3()
        .p_4()
        .child(
            div()
                .flex()
                .flex_row()
                .justify_between()
                .items_center()
                .child(
                    div()
                        .text_lg()
                        .text_color(rgb(t.text_heading))
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .child("Themes"),
                )
                .child(close),
        )
        .child(
            div()
                .text_xs()
                .text_color(rgb(t.text_muted))
                .child("Dreamcoder palettes from the canonical Dreamcoder tokens. The choice is saved and used on the next start."),
        )
        .child(
            div()
                .w_full()
                .flex()
                .flex_row()
                .gap_3()
                .children(theme_cards),
        );

    div()
        .absolute()
        .top_0()
        .left_0()
        .size_full()
        .flex()
        .items_center()
        .justify_center()
        .bg(gpui::rgba(t.scrim))
        .child(
            div()
                .w(px(720.0))
                .max_w_full()
                .flex()
                .flex_row()
                .rounded_lg()
                .overflow_hidden()
                .bg(rgb(t.surface_raised))
                .border_1()
                .border_color(rgb(t.border_ui))
                .shadow_lg()
                .child(nav)
                .child(content),
        )
}

/// A clickable theme card previewing the theme's own canvas, surfaces and status colors.
fn theme_card(option: &ThemeOption, t: &ThemeTokens) -> gpui::Stateful<gpui::Div> {
    let p = option.id.tokens();
    let swatch = |color: u32| div().h(px(14.0)).flex_1().bg(rgb(color));
    let border = if option.cursor { t.focus } else { t.divider };
    div()
        .id(option.id.slug())
        .cursor_pointer()
        .flex_1()
        .min_w_0()
        .flex()
        .flex_col()
        .rounded_lg()
        .overflow_hidden()
        .border_2()
        .border_color(rgb(border))
        .child(
            div()
                .h(px(96.0))
                .flex()
                .flex_col()
                .gap_2()
                .p_3()
                .bg(rgb(p.canvas))
                .child(
                    div()
                        .flex_1()
                        .rounded_md()
                        .p_2()
                        .bg(rgb(p.surface_raised))
                        .border_1()
                        .border_color(rgb(p.divider))
                        .text_xs()
                        .text_color(rgb(p.text))
                        .child("Aa  1212 · 118.00"),
                )
                .child(
                    div()
                        .flex()
                        .flex_row()
                        .rounded_sm()
                        .overflow_hidden()
                        .child(swatch(p.accent))
                        .child(swatch(p.success))
                        .child(swatch(p.warning))
                        .child(swatch(p.error))
                        .child(swatch(p.info)),
                ),
        )
        .child(
            div()
                .flex()
                .h(px(44.0))
                .flex_row()
                .justify_between()
                .items_center()
                .px_3()
                .bg(rgb(t.surface))
                .child(div().text_color(rgb(t.text)).child(option.label))
                .children(option.applied.then(|| chip("Active", t.success, t))),
        )
}
