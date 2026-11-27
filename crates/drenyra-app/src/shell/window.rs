//! Native GPUI window: renders [`ShellViewModel`] and maps key bindings to [`ShellCommand`]s.
//!
//! This module holds no domain logic. Everything it shows comes from the view model and every
//! key it handles goes through [`ShellState::apply`], both unit-tested without a display. Every
//! color comes from the active theme's [`ThemeTokens`]; this module defines none of its own.

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::rc::Rc;
use std::time::Duration;

use gpui::{
    actions, div, prelude::*, px, rgb, size, AnyElement, App, Application, Bounds, ClickEvent,
    Context, FocusHandle, KeyBinding, KeyDownEvent, Rgba, SharedString, Window, WindowBounds,
    WindowOptions,
};

use drenyra_rpc::agent::{AgentSession, AgentUpdate};

use super::agent_pane::{agent_launch, AgentStatus, EntryRole};

use super::commands::{typed_text, ShellCommand, ShellState, ENTRY_READY_TO_POST, KEY_BINDINGS};
use super::settings::save_if_changed;
use super::theme::{ThemeId, ThemeTokens};
use super::view_model::{
    pane_area_with, AgentPaneView, ApprovalDialogView, CliPaneView, EvidencePaneView,
    GuardrailChips, LedgerPaneView, LedgerPostedEntry, LedgerStagedEntry, PaletteView, PaneTile,
    ReviewPanel, RowDecision, SettingsPanel, ShellViewModel, SidebarView, StatusTone, ThemeOption,
    WorkspaceChip, FOOTER_HEIGHT, REVIEW_PANEL_WIDTH, SIDEBAR_WIDTH, TOP_BAR_HEIGHT,
};
use crate::ui::DesktopShellConfig;
use drenyra_workspace::{PaneKind, WorkspaceActivity, WorkspaceKey};

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
        FocusComposer,
        StopAgent,
        OpenApproval,
        OpenPalette,
        ToggleSidebar,
        RestartAgent,
        Quit
    ]
);

/// Monospace family for amounts and identifiers (installed with the Dreamcoder setup).
const MONO_FONT: &str = "JetBrainsMono Nerd Font";

/// Keys shown in the footer, in display order.
const FOOTER_HINT: &str = "Ctrl+B sidebar · Ctrl+Shift+P commands · I ask the agent · Ctrl+P approve · Ctrl+D split · Ctrl+W close · Ctrl+Shift+T undo · Ctrl+Tab next pane · Ctrl+K next workspace · J/K line · A accept · R reject · Ctrl+, settings · Ctrl+Q quit";
/// Footer keys while typing a prompt.
const COMPOSER_HINT: &str =
    "Type your question · Enter send · Esc stop typing · Ctrl+C stop the agent";
/// How often the window drains agent output.
const AGENT_POLL_INTERVAL: Duration = Duration::from_millis(40);
/// Key context for single-key bindings; absent while typing so letters reach the composer.
const SHELL_CONTEXT: &str = "DrenyraShell";
const COMPOSER_CONTEXT: &str = "DrenyraComposer";

/// Footer keys while the command palette is open.
const PALETTE_HINT: &str = "Type to filter · Up/Down choose · Enter run · Esc close";

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
                cx.spawn(async move |this, cx| loop {
                    cx.background_executor().timer(AGENT_POLL_INTERVAL).await;
                    if this
                        .update(cx, |shell: &mut ShellWindow, cx| shell.poll_agents(cx))
                        .is_err()
                    {
                        break;
                    }
                })
                .detach();
                ShellWindow {
                    state,
                    focus_handle,
                    settings_path,
                    save_error: None,
                    sessions: BTreeMap::new(),
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
/// has exactly one action type. Keys without a modifier only apply in [`SHELL_CONTEXT`], so they
/// type into the composer instead of firing commands while a prompt is being written.
fn key_bindings() -> Vec<KeyBinding> {
    KEY_BINDINGS
        .iter()
        .map(|&(key, command)| {
            let context = (!key.contains('-')).then_some(SHELL_CONTEXT);
            (key, command, context)
        })
        .map(|(key, command, context)| match command {
            ShellCommand::SplitFocused => KeyBinding::new(key, SplitFocused, context),
            ShellCommand::CloseFocused => KeyBinding::new(key, CloseFocused, context),
            ShellCommand::UndoClose => KeyBinding::new(key, UndoClose, context),
            ShellCommand::FocusNextPane => KeyBinding::new(key, FocusNextPane, context),
            ShellCommand::NextWorkspace => KeyBinding::new(key, NextWorkspace, context),
            ShellCommand::ReviewCursorDown => KeyBinding::new(key, ReviewCursorDown, context),
            ShellCommand::ReviewCursorUp => KeyBinding::new(key, ReviewCursorUp, context),
            ShellCommand::AcceptLine => KeyBinding::new(key, AcceptLine, context),
            ShellCommand::RejectLine => KeyBinding::new(key, RejectLine, context),
            ShellCommand::OpenSettings => KeyBinding::new(key, OpenSettings, context),
            ShellCommand::CloseOverlay => KeyBinding::new(key, CloseOverlay, context),
            ShellCommand::Confirm => KeyBinding::new(key, Confirm, context),
            ShellCommand::FocusComposer => KeyBinding::new(key, FocusComposer, context),
            ShellCommand::StopAgent => KeyBinding::new(key, StopAgent, context),
            ShellCommand::OpenApproval => KeyBinding::new(key, OpenApproval, context),
            ShellCommand::OpenPalette => KeyBinding::new(key, OpenPalette, context),
            ShellCommand::ToggleSidebar => KeyBinding::new(key, ToggleSidebar, context),
            ShellCommand::RestartAgent => KeyBinding::new(key, RestartAgent, context),
        })
        .collect()
}

struct ShellWindow {
    state: ShellState,
    focus_handle: FocusHandle,
    settings_path: Option<PathBuf>,
    /// Last failure to save settings, shown in the footer until a save succeeds.
    save_error: Option<String>,
    /// Running agent processes by company and period. Dropping one closes the agent gracefully.
    sessions: BTreeMap<WorkspaceKey, AgentSession>,
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

    /// Starts the active workspace's agent on first view, sends queued commands, and applies
    /// whatever every agent wrote since the last poll.
    fn poll_agents(&mut self, cx: &mut Context<Self>) {
        let mut changed = self.ensure_active_agent();
        for (key, session) in &self.sessions {
            let pane = self.state.agent_pane_mut(key);
            for frame in pane.take_outbox() {
                if let Err(err) = session.send(&frame) {
                    changed |= pane.apply(AgentUpdate::Failed(err.to_string()));
                    break;
                }
            }
            for update in session.drain() {
                changed |= pane.apply(update);
            }
        }
        if changed {
            cx.notify();
        }
    }

    fn ensure_active_agent(&mut self) -> bool {
        let key = self.state.active_workspace_key();
        if self.state.agent_pane(&key).status() != &AgentStatus::NotStarted {
            return false;
        }
        // A restarted pane may still own its previous session. Its Drop can wait for the agent's
        // grace period, so it runs off the UI thread.
        if let Some(previous) = self.sessions.remove(&key) {
            std::thread::spawn(move || drop(previous));
        }
        let pane = self.state.agent_pane_mut(&key);
        match agent_launch(crate::find_in_path) {
            Ok(config) => match AgentSession::spawn(&config) {
                Ok(session) => {
                    pane.mark_started();
                    self.sessions.insert(key, session);
                }
                Err(err) => pane.mark_unavailable(format!("Could not start the agent: {err}")),
            },
            Err(message) => pane.mark_unavailable(message),
        }
        true
    }

    /// Routes typing to the active text field (composer or approval dialog).
    fn on_key_down(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        if !self.state.text_input_active() {
            return;
        }
        let keystroke = &event.keystroke;
        let mods = keystroke.modifiers;
        let changed = match keystroke.key.as_str() {
            "enter" if mods.shift => self.state.type_text("\n"),
            "enter" => {
                self.dispatch(ShellCommand::Confirm, window, cx);
                false
            }
            "escape" => {
                self.dispatch(ShellCommand::CloseOverlay, window, cx);
                false
            }
            "backspace" => self.state.backspace(),
            "up" => {
                self.dispatch(ShellCommand::ReviewCursorUp, window, cx);
                false
            }
            "down" => {
                self.dispatch(ShellCommand::ReviewCursorDown, window, cx);
                false
            }
            _ => match typed_text(
                mods.control,
                mods.alt,
                mods.platform,
                keystroke.key_char.as_deref(),
            ) {
                Some(text) => self.state.type_text(text),
                None => return,
            },
        };
        cx.stop_propagation();
        if changed {
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
        pane_area_with(
            f32::from(viewport.width).max(0.0) as u32,
            f32::from(viewport.height).max(0.0) as u32,
            self.state.controller().active_review().is_some(),
            self.state.sidebar_open(),
        )
    }
}

impl Render for ShellWindow {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let vm = self.state.view(self.pane_area(window));
        let t = vm.theme.tokens();

        let commands_button = div()
            .id("open-palette")
            .cursor_pointer()
            .on_click(cx.listener(|this, _: &ClickEvent, w, cx| {
                this.dispatch(ShellCommand::OpenPalette, w, cx)
            }))
            .child(chip("⌘ Commands", t.text_muted, &t));
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

        let sidebar = vm.sidebar.as_ref().map(|sidebar| {
            let companies: Vec<AnyElement> = sidebar
                .companies
                .iter()
                .map(|company| {
                    let index = company.index;
                    sidebar_company(company, &t)
                        .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                            if this.state.switch_workspace(index) {
                                cx.notify();
                            }
                        }))
                        .into_any_element()
                })
                .collect();
            sidebar_panel(sidebar, companies, &t)
        });

        let panes: Vec<AnyElement> = vm
            .panes
            .iter()
            .map(|tile| {
                let body = match tile.kind {
                    PaneKind::FiscalAgent => vm.agent.as_ref().map(|agent| {
                        let composer = div()
                            .id(("composer", tile.id))
                            .cursor_text()
                            .on_click(cx.listener(|this, _: &ClickEvent, w, cx| {
                                this.dispatch(ShellCommand::FocusComposer, w, cx)
                            }))
                            .child(composer_box(agent, tile.focused, &t));
                        agent_body(agent, composer, &t).into_any_element()
                    }),
                    PaneKind::LedgerViewer => vm
                        .ledger
                        .as_ref()
                        .map(|ledger| ledger_body(ledger, &t).into_any_element()),
                    PaneKind::EvidenceVault => vm
                        .evidence
                        .as_ref()
                        .map(|evidence| evidence_body(evidence, &t).into_any_element()),
                    PaneKind::FiscalCli => vm
                        .cli
                        .as_ref()
                        .map(|cli| cli_body(cli, &t).into_any_element()),
                }
                .unwrap_or_else(|| placeholder_body(&t).into_any_element());
                pane_tile(
                    tile,
                    agent_header(tile.kind, vm.agent.as_ref(), &t),
                    body,
                    &t,
                )
                .into_any_element()
            })
            .collect();

        div()
            .key_context(if self.state.text_input_active() {
                COMPOSER_CONTEXT
            } else {
                SHELL_CONTEXT
            })
            .track_focus(&self.focus_handle)
            .on_key_down(cx.listener(Self::on_key_down))
            .on_action(cx.listener(|this, _: &FocusComposer, w, cx| {
                this.dispatch(ShellCommand::FocusComposer, w, cx)
            }))
            .on_action(cx.listener(|this, _: &StopAgent, w, cx| {
                this.dispatch(ShellCommand::StopAgent, w, cx)
            }))
            .on_action(cx.listener(|this, _: &OpenApproval, w, cx| {
                this.dispatch(ShellCommand::OpenApproval, w, cx)
            }))
            .on_action(cx.listener(|this, _: &OpenPalette, w, cx| {
                this.dispatch(ShellCommand::OpenPalette, w, cx)
            }))
            .on_action(cx.listener(|this, _: &RestartAgent, w, cx| {
                this.dispatch(ShellCommand::RestartAgent, w, cx)
            }))
            .on_action(cx.listener(|this, _: &ToggleSidebar, w, cx| {
                this.dispatch(ShellCommand::ToggleSidebar, w, cx)
            }))
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
            .child(top_bar(&vm, commands_button, settings_button, &t))
            .child(
                div()
                    .flex_1()
                    .flex()
                    .flex_row()
                    .min_h_0()
                    .overflow_hidden()
                    .children(sidebar)
                    .child(div().flex_1().relative().min_w_0().children(panes))
                    .children(vm.review.as_ref().map(|panel| {
                        let approve = div()
                            .id("open-approval")
                            .cursor_pointer()
                            .on_click(cx.listener(|this, _: &ClickEvent, w, cx| {
                                this.dispatch(ShellCommand::OpenApproval, w, cx)
                            }))
                            .child(chip("Approve…  Ctrl+P", t.accent, &t));
                        review_panel(panel, approve, &t)
                    })),
            )
            .child(footer(
                vm.settings.is_some(),
                vm.palette.is_some(),
                vm.approval.as_ref(),
                vm.agent.as_ref().is_some_and(|a| a.composer_active),
                self.save_error.as_deref(),
                &t,
            ))
            .children(overlay)
            .children(
                vm.approval
                    .as_ref()
                    .map(|dialog| approval_dialog(dialog, &t)),
            )
            .children(
                vm.palette
                    .as_ref()
                    .map(|palette| command_palette(palette, &t)),
            )
    }
}

fn top_bar(
    vm: &ShellViewModel,
    commands_button: impl IntoElement,
    settings_button: impl IntoElement,
    t: &ThemeTokens,
) -> gpui::Div {
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
                .child(commands_button)
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

/// One-line role description shown under each pane title.
fn pane_caption(kind: PaneKind) -> &'static str {
    match kind {
        PaneKind::FiscalAgent => "Drenyra Shell agent session",
        PaneKind::LedgerViewer => "Entries staged or posted in this session",
        PaneKind::EvidenceVault => "Session evidence log (unsigned)",
        PaneKind::FiscalCli => "Environment check and command reference",
    }
}

fn tone_color(tone: StatusTone, t: &ThemeTokens) -> u32 {
    match tone {
        StatusTone::Muted => t.text_subtle,
        StatusTone::Busy => t.info,
        StatusTone::Ok => t.success,
        StatusTone::Warning => t.warning,
        StatusTone::Error => t.error,
    }
}

/// Right side of a pane header: the agent status (and model) for agent panes.
fn agent_header(
    kind: PaneKind,
    agent: Option<&AgentPaneView>,
    t: &ThemeTokens,
) -> Option<gpui::Div> {
    let agent = agent.filter(|_| kind == PaneKind::FiscalAgent)?;
    Some(
        div()
            .flex()
            .flex_row()
            .gap_2()
            .items_center()
            .min_w_0()
            .children(agent.model.as_ref().map(|model| {
                div()
                    .text_xs()
                    .font_family(MONO_FONT)
                    .text_color(rgb(t.text_subtle))
                    .truncate()
                    .child(model.clone())
            }))
            .child(chip(
                agent.status_label.clone(),
                tone_color(agent.status_tone, t),
                t,
            )),
    )
}

fn pane_tile(
    tile: &PaneTile,
    header_extra: Option<gpui::Div>,
    body: AnyElement,
    t: &ThemeTokens,
) -> impl IntoElement {
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
                        .flex_none()
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
                        .children(header_extra),
                )
                .child(body),
        )
}

/// Body used only when the view model carries no data for a pane (never from `ShellState`).
fn placeholder_body(t: &ThemeTokens) -> impl IntoElement {
    div()
        .flex_1()
        .p_3()
        .text_xs()
        .text_color(rgb(t.text_muted))
        .child("Nothing to show in this pane.")
}

/// Scrollless column shared by the Ledger, Evidence and CLI bodies: content clipped at the
/// bottom, the pane's limits pinned as a footnote.
fn pane_column(limits: &'static str, t: &ThemeTokens) -> (gpui::Div, gpui::Div) {
    let content = div()
        .flex_1()
        .min_h_0()
        .overflow_hidden()
        .flex()
        .flex_col()
        .gap_2()
        .p_3();
    let footnote = div()
        .flex_none()
        .px_3()
        .py_2()
        .border_t_1()
        .border_color(rgb(t.divider))
        .text_xs()
        .text_color(rgb(t.text_subtle))
        .child(limits);
    (content, footnote)
}

fn pane_section_heading(text: impl Into<SharedString>, t: &ThemeTokens) -> impl IntoElement {
    div()
        .text_xs()
        .font_weight(gpui::FontWeight::SEMIBOLD)
        .text_color(rgb(t.text_subtle))
        .child(text.into())
}

fn wrap_column(content: gpui::Div, footnote: gpui::Div) -> impl IntoElement {
    div()
        .flex_1()
        .min_h_0()
        .flex()
        .flex_col()
        .child(content)
        .child(footnote)
}

/// Ledger pane: the active workspace's staged entry and the entries posted in this session.
fn ledger_body(ledger: &LedgerPaneView, t: &ThemeTokens) -> impl IntoElement {
    let (content, footnote) = pane_column(ledger.limits, t);
    let content = content
        .child(
            div()
                .text_xs()
                .font_family(MONO_FONT)
                .text_color(rgb(t.text_muted))
                .child(format!(
                    "RUC {} · {}",
                    ledger.entity_ruc, ledger.fiscal_period
                )),
        )
        .children(
            ledger
                .empty_message
                .as_ref()
                .map(|message| div().text_color(rgb(t.text_subtle)).child(message.clone())),
        )
        .children(
            ledger
                .staged
                .as_ref()
                .map(|staged| ledger_staged(staged, t)),
        )
        .children(
            (!ledger.posted.is_empty()).then(|| pane_section_heading("POSTED IN THIS SESSION", t)),
        )
        .children(ledger.posted.iter().map(|entry| ledger_posted(entry, t)));
    wrap_column(content, footnote)
}

fn ledger_staged(staged: &LedgerStagedEntry, t: &ThemeTokens) -> impl IntoElement {
    let status_color = if !staged.balanced {
        t.error
    } else if staged.status == ENTRY_READY_TO_POST {
        t.success
    } else {
        t.warning
    };
    div()
        .flex()
        .flex_col()
        .gap_1()
        .child(
            div()
                .flex()
                .flex_row()
                .justify_between()
                .items_center()
                .gap_2()
                .child(pane_section_heading("STAGED, NOT POSTED", t))
                .child(chip(staged.status, status_color, t)),
        )
        .child(
            div()
                .flex()
                .flex_row()
                .gap_2()
                .text_xs()
                .child(
                    div()
                        .font_family(MONO_FONT)
                        .text_color(rgb(t.text))
                        .child(staged.proposal_id.clone()),
                )
                .child(
                    div()
                        .text_color(rgb(t.text_subtle))
                        .child(staged.risk_tier_label),
                ),
        )
        .child(review_header(t))
        .children(staged.lines.iter().map(|line| {
            div()
                .flex()
                .flex_row()
                .items_center()
                .px_1()
                .child(div().w(px(20.0)))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .flex()
                        .flex_col()
                        .child(
                            div()
                                .font_family(MONO_FONT)
                                .child(line.account_code.clone()),
                        )
                        .child(
                            div()
                                .text_xs()
                                .text_color(rgb(t.text_subtle))
                                .truncate()
                                .child(line.description.clone()),
                        ),
                )
                .child(money_cell(line.debit.clone()))
                .child(money_cell(line.credit.clone()))
        }))
        .child(
            div()
                .flex()
                .flex_row()
                .px_1()
                .border_t_1()
                .border_color(rgb(t.divider))
                .pt_1()
                .child(div().flex_1().text_color(rgb(t.text_muted)).child("Totals"))
                .child(money_cell(staged.total_debit.clone()))
                .child(money_cell(staged.total_credit.clone())),
        )
}

fn ledger_posted(entry: &LedgerPostedEntry, t: &ThemeTokens) -> impl IntoElement {
    div()
        .flex()
        .flex_col()
        .gap_0p5()
        .p_2()
        .rounded_md()
        .bg(rgb(t.surface_raised))
        .child(
            div()
                .flex()
                .flex_row()
                .justify_between()
                .gap_2()
                .child(
                    div()
                        .min_w_0()
                        .truncate()
                        .font_family(MONO_FONT)
                        .text_color(rgb(t.text))
                        .child(entry.proposal_id.clone()),
                )
                .child(chip("Posted", t.success, t)),
        )
        .child(div().text_xs().text_color(rgb(t.text_muted)).child(format!(
            "{} · approved by {} · request {}",
            entry.risk_tier_label, entry.approved_by, entry.request_id
        )))
        .child(
            div()
                .flex()
                .flex_row()
                .child(
                    div()
                        .flex_1()
                        .text_xs()
                        .text_color(rgb(t.text_subtle))
                        .child("Totals"),
                )
                .child(money_cell(entry.total_debit.clone()))
                .child(money_cell(entry.total_credit.clone())),
        )
}

/// Evidence pane: the active workspace's session log, newest first.
fn evidence_body(evidence: &EvidencePaneView, t: &ThemeTokens) -> impl IntoElement {
    let (content, footnote) = pane_column(evidence.limits, t);
    let content = content
        .child(
            div()
                .text_xs()
                .font_family(MONO_FONT)
                .text_color(rgb(t.text_muted))
                .child(format!(
                    "RUC {} · {}",
                    evidence.entity_ruc, evidence.fiscal_period
                )),
        )
        .children(
            evidence
                .empty_message
                .as_ref()
                .map(|message| div().text_color(rgb(t.text_subtle)).child(message.clone())),
        )
        .children(evidence.entries.iter().map(|entry| {
            div()
                .font_family(MONO_FONT)
                .text_xs()
                .text_color(rgb(t.text))
                .child(entry.clone())
        }));
    wrap_column(content, footnote)
}

/// Fiscal CLI pane: the captured environment check and the flag reference; runs nothing.
fn cli_body(cli: &CliPaneView, t: &ThemeTokens) -> impl IntoElement {
    let (content, footnote) = pane_column(cli.limits, t);
    let content = content
        .child(pane_section_heading("drenyra-gpui --check-env", t))
        .children(cli.environment.is_empty().then(|| {
            div()
                .text_color(rgb(t.text_subtle))
                .child("No environment check was captured for this window.")
        }))
        .children(cli.environment.iter().map(|field| {
            div()
                .flex()
                .flex_row()
                .gap_2()
                .text_xs()
                .font_family(MONO_FONT)
                .child(
                    div()
                        .w(px(150.0))
                        .flex_none()
                        .text_color(rgb(t.text_muted))
                        .child(field.name),
                )
                .child(div().text_color(rgb(t.text)).child(field.value.clone()))
        }))
        .child(pane_section_heading("FLAGS", t))
        .children(cli.flags.iter().map(|flag| {
            div()
                .flex()
                .flex_col()
                .text_xs()
                .child(
                    div()
                        .font_family(MONO_FONT)
                        .text_color(rgb(t.accent))
                        .child(format!("drenyra-gpui {}", flag.usage)),
                )
                .child(div().text_color(rgb(t.text_muted)).child(flag.description))
        }));
    wrap_column(content, footnote)
}

/// Transcript (newest at the bottom; older entries are clipped at the top) and the composer.
fn agent_body(
    agent: &AgentPaneView,
    composer: impl IntoElement,
    t: &ThemeTokens,
) -> impl IntoElement {
    let empty = agent.transcript.is_empty() && agent.streaming_text.is_empty();
    div()
        .flex_1()
        .min_h_0()
        .flex()
        .flex_col()
        .child(
            div()
                .flex_1()
                .min_h_0()
                .overflow_hidden()
                .flex()
                .flex_col()
                .justify_end()
                .gap_2()
                .p_3()
                .children(empty.then(|| {
                    div()
                        .text_color(rgb(t.text_subtle))
                        .child("Ask the Fiscal Agent about this company. Press I or click below to start typing.")
                }))
                .children(agent.transcript.iter().map(|entry| transcript_entry(entry.role, &entry.text, t)))
                .children(agent.restart_hint.as_ref().map(|hint| {
                    div().text_color(rgb(t.warning)).child(hint.clone())
                }))
                .children((!agent.streaming_text.is_empty()).then(|| {
                    transcript_entry(EntryRole::Agent, &format!("{}▍", agent.streaming_text), t)
                })),
        )
        .child(composer)
}

fn transcript_entry(role: EntryRole, text: &str, t: &ThemeTokens) -> impl IntoElement {
    let (label, label_color, body_color) = match role {
        EntryRole::Operator => ("You", t.accent, t.text),
        EntryRole::Agent => ("Agent", t.success, t.text),
        EntryRole::Tool => ("Tool", t.info, t.text_muted),
        EntryRole::Notice => ("Note", t.text_subtle, t.text_muted),
        EntryRole::Error => ("Error", t.error, t.error),
    };
    div()
        .flex()
        .flex_col()
        .gap_0p5()
        .child(
            div()
                .text_xs()
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .text_color(rgb(label_color))
                .child(label),
        )
        .child(div().text_color(rgb(body_color)).child(text.to_owned()))
}

fn composer_box(agent: &AgentPaneView, pane_focused: bool, t: &ThemeTokens) -> impl IntoElement {
    let (text, color) = if agent.composer_active {
        (format!("{}▏", agent.composer), t.text)
    } else if !agent.composer.is_empty() {
        (agent.composer.clone(), t.text_muted)
    } else if !agent.can_send {
        ("The agent is not available.".to_owned(), t.text_subtle)
    } else if pane_focused {
        ("Press I to ask the Fiscal Agent…".to_owned(), t.text_subtle)
    } else {
        (
            "Focus this pane (Ctrl+Tab) to ask the Fiscal Agent…".to_owned(),
            t.text_subtle,
        )
    };
    let border = if agent.composer_active {
        t.focus
    } else {
        t.border_ui
    };
    div()
        .flex_none()
        .m_2()
        .px_3()
        .py_2()
        .rounded_md()
        .bg(rgb(t.surface_raised))
        .border_1()
        .border_color(rgb(border))
        .text_color(rgb(color))
        .child(text)
}

fn review_panel(
    panel: &ReviewPanel,
    approve: impl IntoElement,
    t: &ThemeTokens,
) -> impl IntoElement {
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
        .children(panel.approved_by.is_none().then_some(approve))
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

fn footer(
    settings_open: bool,
    palette_open: bool,
    approval: Option<&ApprovalDialogView>,
    composing: bool,
    save_error: Option<&str>,
    t: &ThemeTokens,
) -> impl IntoElement {
    let hint = if settings_open {
        SETTINGS_HINT
    } else if palette_open {
        PALETTE_HINT
    } else if let Some(dialog) = approval {
        dialog.footer_hint()
    } else if composing {
        COMPOSER_HINT
    } else {
        FOOTER_HINT
    };
    let (text, color) = match save_error {
        Some(err) => (err.to_owned(), t.error),
        None => (hint.to_owned(), t.text_subtle),
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

/// Modal human-approval dialog: entry summary, approver field and the current blocker.
fn approval_dialog(dialog: &ApprovalDialogView, t: &ThemeTokens) -> impl IntoElement {
    let summary_row = |label: &'static str, value: String| {
        div()
            .flex()
            .flex_row()
            .justify_between()
            .child(div().text_color(rgb(t.text_muted)).child(label))
            .child(div().font_family(MONO_FONT).child(value))
    };
    let requirement = if dialog.required {
        "This risk tier requires a recorded human approval before posting."
    } else {
        "This risk tier does not require approval; recording one is optional."
    };
    let message = dialog
        .error
        .clone()
        .or_else(|| dialog.blocker.map(str::to_owned));
    let current_approver = dialog.current_approver.as_ref().map(|current| {
        div()
            .text_xs()
            .text_color(rgb(t.warning))
            .child(format!(
                "Approved by {current}. Confirming replaces that approval; both stay in the evidence log."
            ))
    });
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
                .w(px(460.0))
                .max_w_full()
                .flex()
                .flex_col()
                .gap_3()
                .p_4()
                .rounded_lg()
                .bg(rgb(t.surface_raised))
                .border_1()
                .border_color(rgb(t.border_ui))
                .shadow_lg()
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
                                .child(dialog.title()),
                        )
                        .child(chip(dialog.risk_tier_label, t.warning, t)),
                )
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap_1()
                        .p_3()
                        .rounded_md()
                        .bg(rgb(t.surface))
                        .child(summary_row("Entry", dialog.proposal_id.clone()))
                        .child(summary_row("Debit", dialog.total_debit.clone()))
                        .child(summary_row("Credit", dialog.total_credit.clone())),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(rgb(t.text_muted))
                        .child(requirement),
                )
                .children(current_approver)
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap_1()
                        .child(
                            div()
                                .text_xs()
                                .text_color(rgb(t.text_subtle))
                                .child("Approver ID"),
                        )
                        .child(
                            div()
                                .px_3()
                                .py_2()
                                .rounded_md()
                                .bg(rgb(t.surface))
                                .border_1()
                                .border_color(rgb(t.focus))
                                .font_family(MONO_FONT)
                                .child(format!("{}▏", dialog.approver)),
                        ),
                )
                .children(message.map(|text| {
                    div()
                        .text_xs()
                        .text_color(rgb(if dialog.ready && dialog.error.is_none() {
                            t.text_subtle
                        } else {
                            t.warning
                        }))
                        .child(text)
                }))
                .child(div().text_xs().text_color(rgb(t.text_subtle)).child(
                    "The ID is recorded as typed. Editing any line afterwards clears the approval.",
                )),
        )
}

/// Command palette: query field and the matching entries with their key hints.
fn command_palette(palette: &PaletteView, t: &ThemeTokens) -> impl IntoElement {
    let entries = palette.entries.iter().take(14).map(|entry| {
        let row = div()
            .flex()
            .flex_row()
            .justify_between()
            .items_center()
            .gap_3()
            .px_3()
            .py_1p5()
            .rounded_md()
            .child(div().min_w_0().truncate().child(entry.label.clone()))
            .children((!entry.keys.is_empty()).then(|| {
                div()
                    .flex_none()
                    .text_xs()
                    .font_family(MONO_FONT)
                    .text_color(rgb(t.text_subtle))
                    .child(entry.keys.clone())
            }));
        if entry.selected {
            row.bg(rgb(t.selection)).text_color(rgb(t.on_selection))
        } else {
            row
        }
    });
    let query = if palette.query.is_empty() {
        div()
            .text_color(rgb(t.text_subtle))
            .child("Type a command, a company or a theme…▏")
    } else {
        div().child(format!("{}▏", palette.query))
    };
    div()
        .absolute()
        .top_0()
        .left_0()
        .size_full()
        .flex()
        .flex_col()
        .items_center()
        .pt(px(96.0))
        .bg(gpui::rgba(t.scrim))
        .child(
            div()
                .w(px(560.0))
                .max_w_full()
                .flex()
                .flex_col()
                .gap_1()
                .p_2()
                .rounded_lg()
                .bg(rgb(t.surface_raised))
                .border_1()
                .border_color(rgb(t.border_ui))
                .shadow_lg()
                .child(
                    div()
                        .px_3()
                        .py_2()
                        .mb_1()
                        .rounded_md()
                        .bg(rgb(t.surface))
                        .border_1()
                        .border_color(rgb(t.focus))
                        .child(query),
                )
                .children(entries)
                .children(palette.entries.is_empty().then(|| {
                    div()
                        .px_3()
                        .py_2()
                        .text_color(rgb(t.text_subtle))
                        .child("No matching commands.")
                })),
        )
}

/// Left sidebar: companies with their staged entries, plus the documents section.
fn sidebar_panel(
    sidebar: &SidebarView,
    companies: Vec<AnyElement>,
    t: &ThemeTokens,
) -> impl IntoElement {
    let heading = |text: &'static str| {
        div()
            .px_2()
            .pt_2()
            .text_xs()
            .font_weight(gpui::FontWeight::SEMIBOLD)
            .text_color(rgb(t.text_subtle))
            .child(text)
    };
    let unposted: usize = sidebar.companies.iter().map(|c| c.unposted_entries).sum();
    div()
        .w(px(SIDEBAR_WIDTH as f32))
        .flex_none()
        .flex()
        .flex_col()
        .gap_1()
        .p_2()
        .overflow_hidden()
        .bg(rgb(t.chrome))
        .border_r_1()
        .border_color(rgb(t.divider))
        .child(heading("COMPANIES"))
        .children(companies)
        .child(heading("UNPOSTED ENTRIES"))
        .child(
            div()
                .px_2()
                .text_xs()
                .text_color(rgb(if unposted > 0 { t.warning } else { t.text_subtle }))
                .child(match unposted {
                    0 => "Nothing waiting to post.".to_owned(),
                    1 => "1 entry waiting across companies.".to_owned(),
                    n => format!("{n} entries waiting across companies."),
                }),
        )
        .child(heading("DOCUMENTS & LEDGERS"))
        .child(
            div()
                .px_2()
                .text_xs()
                .text_color(rgb(t.text_subtle))
                .child("No document source connected yet. Vouchers (XML, PDF, CDR) and ledgers appear here once Drenyra Shell serves them."),
        )
}

/// One clickable company card in the sidebar.
fn sidebar_company(
    company: &super::view_model::SidebarCompany,
    t: &ThemeTokens,
) -> gpui::Stateful<gpui::Div> {
    let card = div()
        .id(("company", company.index))
        .cursor_pointer()
        .flex()
        .flex_col()
        .gap_0p5()
        .p_2()
        .rounded_md()
        .border_1()
        .border_color(rgb(if company.active { t.accent } else { t.chrome }))
        .child(
            div()
                .flex()
                .flex_row()
                .justify_between()
                .items_center()
                .gap_2()
                .child(
                    div()
                        .min_w_0()
                        .truncate()
                        .text_color(rgb(if company.active {
                            t.text_heading
                        } else {
                            t.text
                        }))
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .child(company.legal_name.clone()),
                )
                .child(dot(activity_color(company.activity, t))),
        )
        .child(
            div()
                .text_xs()
                .font_family(MONO_FONT)
                .text_color(rgb(t.text_subtle))
                .child(format!(
                    "RUC {} · {}",
                    company.entity_ruc, company.fiscal_period
                )),
        )
        .children(company.staged.as_ref().map(|entry| {
            div()
                .flex()
                .flex_row()
                .justify_between()
                .gap_2()
                .text_xs()
                .child(
                    div()
                        .min_w_0()
                        .truncate()
                        .font_family(MONO_FONT)
                        .text_color(rgb(t.text_muted))
                        .child(entry.proposal_id.clone()),
                )
                .child(
                    div()
                        .flex_none()
                        .text_color(rgb(t.warning))
                        .child(entry.status),
                )
        }));
    if company.active {
        card.bg(rgb(t.surface_strong))
    } else {
        card
    }
}
