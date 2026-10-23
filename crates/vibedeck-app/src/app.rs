//! The egui application: top-level layout, event pump, and glue
//! between the ACP worker, the `~/.vibe` config layer and the UI.
//!
//! Layout mirrors the original Mistral Vibe Nexus app 1:1:
//! a frameless window with a slim header acting as the titlebar
//! (brand + agent status + ─ □ ✕), an always-present sidebar
//! (collapsible to an icon rail), the chat column with an optional
//! right workspace dock, and the input bar.
//!
//! **Unofficial community project — not affiliated with Mistral AI.**

use std::path::PathBuf;
use std::time::Duration;

use eframe::egui;
use egui::{
    Align, CentralPanel, Context, Id, Layout, Sense, SidePanel, TopBottomPanel, ViewportCommand,
    Window,
};
use vibedeck_acp::AcpHandle;
use vibedeck_core::proto::{
    AcpCommand, AcpEvent, AcpSessionInfo, AvailableCommand, ConfigOption, ModeInfo,
};
use vibedeck_core::types::{ChatSession, Message, MessageSender, ThinkingBlock};

use crate::i18n::{t, Lang};
use crate::settings::SettingsPane;
use crate::setup::SetupPane;
use crate::state::{AcpStatus, AppSettings, FolderPicker, PendingApproval, PendingQuestion, UsageStats, View};
use crate::stats::StatsPane;
use crate::theme;

/// Register the bundled Noto Sans Symbols 2 as a fallback font so the
/// interface glyphs (✕ □ ▣ ➤ ⏹ ✔ ☀ ◧ ▤ ▾ …) render on Windows instead of
/// tofu squares — egui's Ubuntu font has no coverage for them.
fn install_symbols_font(ctx: &egui::Context) {
    let mut fonts = egui::FontDefinitions::default();
    fonts.font_data.insert(
        "noto-symbols".into(),
        egui::FontData::from_static(include_bytes!("../assets/NotoSansSymbols2-Regular.ttf")),
    );
    for family in [egui::FontFamily::Proportional, egui::FontFamily::Monospace] {
        fonts.families.entry(family).or_default().push("noto-symbols".into());
    }
    ctx.set_fonts(fonts);
}

pub struct VibedeckApp {
    pub settings: AppSettings,
    pub lang: Lang,
    pub view: View,
    pub acp: AcpHandle,
    pub status: AcpStatus,
    pub protocol_version: Option<u32>,
    pub acp_error: String,

    pub sessions: Vec<ChatSession>,
    pub current: Option<usize>,
    pub backend_sessions: Vec<AcpSessionInfo>,
    /// Session config options (model / mode / thinking / …) advertised by
    /// `session/new` → `configOptions` (vibe-acp ≥ 3.0).
    pub config_options: Vec<ConfigOption>,
    pub modes: Vec<ModeInfo>,
    pub commands: Vec<AvailableCommand>,

    /// Live usage counters for the active session (`usage_update` events).
    pub usage: UsageStats,
    /// Right workspace dock (files tree + git).
    pub dock: crate::dock::Dock,

    /// A prompt waiting for `session/new` / `session/load` to finish.
    pub pending_prompt: Option<String>,
    pub busy: bool,

    pub approval: Option<PendingApproval>,
    pub question: Option<PendingQuestion>,
    pub input: String,
    pub file_index: Vec<(String, bool)>,
    /// Whether `file_index` matches the current working directory.
    /// (Lazily built on the first `@` mention — keeps startup fast.)
    pub file_index_fresh: bool,
    pub folder_picker: FolderPicker,

    pub settings_pane: SettingsPane,
    pub stats_pane: StatsPane,
    pub setup_pane: SetupPane,

    pub debug_log: Vec<String>,
    pub show_about: bool,
    pub toast: Option<(String, std::time::Instant)>,
}

impl VibedeckApp {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let settings = AppSettings::load();
        let lang = Lang::from_code(&settings.language);
        // Symbols font (─ □ ✕ ➤ ⏹ ✔ …) — egui's Ubuntu font lacks them and
        // Windows shows tofu squares. Registered as a fallback so text keeps
        // using the Nexus-style Ubuntu everywhere else.
        install_symbols_font(&cc.egui_ctx);
        // Apply the saved theme (Nexus dark by default).
        theme::apply(&cc.egui_ctx, &settings.theme);

        let acp = AcpHandle::spawn();
        let mut app = Self {
            view: View::Setup,
            status: AcpStatus::Disconnected,
            protocol_version: None,
            acp_error: String::new(),
            sessions: Vec::new(),
            current: None,
            backend_sessions: Vec::new(),
            config_options: Vec::new(),
            modes: Vec::new(),
            commands: Vec::new(),
            usage: UsageStats::default(),
            dock: crate::dock::Dock::default(),
            pending_prompt: None,
            busy: false,
            approval: None,
            question: None,
            input: String::new(),
            file_index: Vec::new(),
            file_index_fresh: false,
            folder_picker: FolderPicker::default(),
            settings_pane: SettingsPane::default(),
            stats_pane: StatsPane::default(),
            setup_pane: SetupPane::default(),
            debug_log: Vec::new(),
            show_about: false,
            toast: None,
            lang,
            acp,
            settings,
        };

        // Fast start: if we already know the vibe-acp path from the settings,
        // connect immediately without any blocking dependency scan. The UI
        // is usable right away; the handshake completes asynchronously.
        if !app.settings.acp_path.is_empty() {
            app.start_agent(&app.settings.acp_path.clone());
            app.view = View::Chat;
        } else {
            // Otherwise probe dependencies in the background; when the
            // result arrives (see `poll_setup`), auto-connect if found.
            app.setup_pane.refresh_status_async();
        }
        app
    }

    pub fn tr(&self) -> &'static crate::i18n::Str {
        t(self.lang)
    }

    pub fn palette(&self) -> theme::Palette {
        theme::Palette::for_theme(&self.settings.theme)
    }

    /// Re-apply the theme after a settings change.
    pub fn set_theme(&mut self, ctx: &Context, theme: &str) {
        self.settings.theme = theme.to_string();
        self.settings.save();
        theme::apply(ctx, theme);
    }

    pub fn start_agent(&mut self, acp_path: &str) {
        self.settings.acp_path = acp_path.to_string();
        self.settings.save();
        self.status = AcpStatus::Connecting;
        self.acp.send(AcpCommand::Start {
            acp_path: acp_path.to_string(),
        });
    }

    /// Mark the `@`-mention file index as outdated (rebuilt lazily).
    pub fn mark_file_index_stale(&mut self) {
        self.file_index_fresh = false;
    }

    /// Reset the workspace dock when the working directory changes.
    pub fn reset_dock(&mut self) {
        self.dock.expanded.clear();
        self.dock.preview = None;
        self.dock.git.branch.clear();
        self.dock.git.entries.clear();
        self.dock.git.diff_file = None;
        self.dock.git.diff_text.clear();
    }

    /// Ensure the @-mention index matches the current working directory.
    /// Only called from the input-bar `@` popup — never at startup.
    pub fn ensure_file_index(&mut self) {
        if !self.file_index_fresh {
            let cwd = self.current_workdir();
            self.file_index = crate::picker::index_files(&cwd);
            self.file_index_fresh = true;
        }
    }

    pub fn current_workdir(&self) -> PathBuf {
        if !self.settings.working_directory.is_empty() {
            PathBuf::from(&self.settings.working_directory)
        } else {
            dirs::home_dir().unwrap_or_else(|| PathBuf::from("."))
        }
    }

    /// Current value of a config option (model / mode / thinking…).
    pub fn option_value(&self, id: &str) -> Option<&ConfigOption> {
        self.config_options.iter().find(|o| o.id == id)
    }

    /// Select a config option locally + on the agent.
    pub fn set_config_option(&mut self, id: &str, value: String) {
        if let Some(opt) = self.config_options.iter_mut().find(|o| o.id == id) {
            opt.current_value = value.clone();
        }
        if id == "model" {
            self.settings.preferred_model = Some(value.clone());
            self.settings.save();
        }
        if let Some(session) = self.current_session() {
            if let Some(acp_session_id) = session.acp_session_id.clone() {
                self.acp.send(AcpCommand::SetConfigOption {
                    session_id: acp_session_id,
                    key: id.to_string(),
                    value,
                });
            }
        }
    }

    pub fn current_session(&self) -> Option<&ChatSession> {
        self.current.and_then(|i| self.sessions.get(i))
    }

    pub fn current_session_mut(&mut self) -> Option<&mut ChatSession> {
        match self.current {
            Some(i) => self.sessions.get_mut(i),
            None => None,
        }
    }

    pub fn new_local_session(&mut self) {
        let cwd = self
            .current_session()
            .map(|s| s.working_directory.clone())
            .unwrap_or_else(|| self.settings.working_directory.clone());
        let mut session = ChatSession::new("New session", &cwd);
        session.working_directory = cwd;
        self.sessions.insert(0, session);
        self.current = Some(0);
        self.mark_file_index_stale();
        self.usage = UsageStats::default();
        self.reset_dock();
        self.view = View::Chat;
    }

    pub fn start_backend_session(&mut self) {
        let cwd = self.current_workdir().to_string_lossy().to_string();
        self.settings.remember_folder(&cwd);
        self.settings.save();
        self.busy = true;
        self.status = if self.status == AcpStatus::Connected {
            AcpStatus::Connected
        } else {
            AcpStatus::Connecting
        };
        self.acp.send(AcpCommand::NewSession {
            cwd,
            model: self.settings.preferred_model.clone(),
        });
    }

    pub fn send_message(&mut self, text: String) {
        let text = text.trim().to_string();
        if text.is_empty() {
            return;
        }
        // Make sure a local session exists.
        if self.current.is_none() {
            self.new_local_session();
            self.start_backend_session();
        }
        if let Some(session) = self.current_session_mut() {
            session.messages.push(Message::new_user(&text));
            session.last_active_ms = vibedeck_core::types::now_ms();
        }
        let Some(session) = self.current_session() else {
            return;
        };
        match session.acp_session_id.clone() {
            Some(acp_session_id) => {
                self.busy = true;
                self.acp.send(AcpCommand::Prompt {
                    session_id: acp_session_id,
                    text,
                });
            }
            None => {
                // Session handshake still in flight — queue the prompt.
                self.busy = true;
                self.pending_prompt = Some(text);
                self.start_backend_session();
            }
        }
    }

    pub fn notify(&mut self, msg: impl Into<String>) {
        self.toast = Some((msg.into(), std::time::Instant::now()));
    }

    // -- event pump -----------------------------------------------------------

    pub fn drain_events(&mut self, ctx: &Context) {
        let mut repaint = false;
        while let Ok(event) = self.acp.events().try_recv() {
            repaint = true;
            self.apply_event(event);
        }
        if self.dock.poll_git() {
            repaint = true;
        }
        if repaint {
            ctx.request_repaint();
        }
        if self.busy || self.setup_pane.is_working() || self.dock.git.working() {
            // Keep the UI alive while the agent works / installer streams.
            ctx.request_repaint_after(Duration::from_millis(120));
        }
    }

    fn session_by_acp_id_mut(&mut self, acp_session_id: &str) -> Option<&mut ChatSession> {
        self.sessions
            .iter_mut()
            .find(|s| s.acp_session_id.as_deref() == Some(acp_session_id))
    }

    fn apply_event(&mut self, event: AcpEvent) {
        if self.usage.apply(&event) {
            return; // live usage counters updated
        }
        match event {
            AcpEvent::Initialized { protocol_version } => {
                self.protocol_version = Some(protocol_version);
                self.status = AcpStatus::Connected;
                self.acp_error.clear();
            }
            AcpEvent::Disconnected { .. } => {
                self.status = AcpStatus::Disconnected;
                self.busy = false;
                self.finalize_open_messages();
            }
            AcpEvent::SessionsList(sessions) => self.backend_sessions = sessions,
            AcpEvent::SessionReady {
                session_id,
                modes,
                config_options,
                title,
            } => {
                self.status = AcpStatus::Connected;
                self.busy = false;
                self.modes = modes;
                self.config_options = config_options;
                if let Some(session) = self.current_session_mut() {
                    session.acp_session_id = Some(session_id);
                    if let Some(title) = title {
                        if !title.is_empty() {
                            session.name = title;
                        }
                    }
                }
                if let Some(text) = self.pending_prompt.take() {
                    self.send_message(text);
                }
            }
            AcpEvent::UsageUpdate { .. } => { /* handled above */ }
            AcpEvent::SessionError { context, message } => {
                self.busy = false;
                self.status = AcpStatus::Error;
                self.acp_error = format!("{context}: {message}");
                self.finalize_open_messages();
                self.push_system_message(&format!("⚠ {context}: {message}"));
            }
            AcpEvent::MessageChunk { session_id, text } => {
                if let Some(session) = self.session_by_acp_id_mut(&session_id) {
                    let msg = ensure_ai_message(session);
                    msg.content.push_str(&text);
                }
            }
            AcpEvent::ThinkingChunk { session_id, text } => {
                if let Some(session) = self.session_by_acp_id_mut(&session_id) {
                    let msg = ensure_ai_message(session);
                    match msg.thinking.last_mut() {
                        Some(block) if !block.is_complete => block.content.push_str(&text),
                        _ => msg.thinking.push(ThinkingBlock {
                            id: "thinking".into(),
                            content: text,
                            is_complete: false,
                        }),
                    }
                }
            }
            AcpEvent::ThinkingComplete { session_id } => {
                if let Some(session) = self.session_by_acp_id_mut(&session_id) {
                    if let Some(msg) = session.messages.last_mut() {
                        for block in &mut msg.thinking {
                            block.is_complete = true;
                        }
                    }
                }
            }
            AcpEvent::ToolCallStarted {
                session_id,
                tool_call,
            } => {
                if let Some(session) = self.session_by_acp_id_mut(&session_id) {
                    ensure_ai_message(session).tool_calls.push(tool_call);
                }
            }
            AcpEvent::ToolCallUpdated {
                session_id,
                tool_call_id,
                status,
                output,
            } => {
                if let Some(session) = self.session_by_acp_id_mut(&session_id) {
                    let status_clone = status.clone();
                    for msg in session.messages.iter_mut().rev() {
                        for tc in &mut msg.tool_calls {
                            if tc.id == tool_call_id {
                                tc.status = status_clone;
                                if let Some(output) = &output {
                                    tc.output = Some(output.clone());
                                }
                            }
                        }
                    }
                }
            }
            AcpEvent::ToolApprovalRequest {
                request_id,
                method,
                params_json,
                tool_name,
            } => {
                self.approval = Some(PendingApproval {
                    request_id,
                    method,
                    params_json,
                    tool_name,
                });
            }
            AcpEvent::AskUserQuestion {
                request_id,
                question,
                choices,
            } => {
                self.question = Some(PendingQuestion {
                    request_id,
                    question,
                    choices,
                    answer: String::new(),
                });
            }
            AcpEvent::PlanUpdate {
                session_id,
                entries_json,
            } => {
                self.debug_log
                    .push(format!("plan [{session_id}]: {entries_json}"));
            }
            AcpEvent::AvailableCommands {
                session_id,
                commands,
            } => {
                let _ = session_id;
                self.commands = commands;
            }
            AcpEvent::PromptCompleted { .. } => {
                self.busy = false;
                self.finalize_open_messages();
                // The agent may have touched files — refresh the git dock.
                let cwd = self.current_workdir();
                self.dock.refresh_status(&cwd);
            }
            AcpEvent::DebugLog(line) => {
                self.debug_log.push(line);
                if self.debug_log.len() > 500 {
                    self.debug_log.remove(0);
                }
            }
        }
    }

    fn finalize_open_messages(&mut self) {
        if let Some(session) = self.current_session_mut() {
            for msg in &mut session.messages {
                if msg.sender == MessageSender::Ai {
                    msg.is_complete = true;
                }
            }
        }
    }

    fn push_system_message(&mut self, text: &str) {
        if let Some(session) = self.current_session_mut() {
            session.messages.push(Message::new_system(text));
        }
    }

    // -- layout -----------------------------------------------------------------

    pub fn ui(&mut self, ctx: &Context) {
        self.drain_events(ctx);
        self.poll_setup();

        let p = self.palette();
        let t = self.tr();

        // -- frameless window: edge resize (PhotoCraft-style zones) ------
        crate::frameless::resize_zones(ctx);

        // -- header (titlebar: brand + status + window buttons) ---------
        TopBottomPanel::top("header")
            .frame(
                egui::Frame::default()
                    .fill(p.surface)
                    .inner_margin(egui::Margin::symmetric(8.0, 5.0))
                    .stroke(egui::Stroke::new(1.0_f32, p.border_subtle)),
            )
            .show(ctx, |ui| {
                ui.horizontal_centered(|ui| {
                    ui.set_min_height(28.0);
                    // brand: pixel flame + name
                    theme::paint_flame(ui, &p, 0.13);
                    ui.add_space(6.0);
                    ui.label(
                        egui::RichText::new(t.app_name)
                            .size(13.0)
                            .strong()
                            .color(p.text),
                    );

                    // status: "● Connected" with tooltip
                    let (status_text, color) = match self.status {
                        AcpStatus::Connected => (t.connected, p.green),
                        AcpStatus::Connecting => (t.connecting, p.accent),
                        AcpStatus::Disconnected => (t.disconnected, p.muted),
                        AcpStatus::Error => (t.error, p.red),
                    };
                    ui.add_space(10.0);
                    ui.label(
                        egui::RichText::new(format!("● {status_text}"))
                            .size(11.0)
                            .color(color),
                    )
                    .on_hover_text(if self.acp_error.is_empty() {
                        status_text.to_string()
                    } else {
                        format!("{status_text} — {}", self.acp_error)
                    });
                    if self.busy {
                        ui.add(egui::Spinner::new().size(14.0));
                    }

                    // right side: dock / theme / about / window buttons
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        crate::frameless::window_buttons(ctx, ui, &p);
                        ui.separator();
                        if ui
                            .add(
                                egui::Button::new(egui::RichText::new("i").small())
                                    .fill(egui::Color32::TRANSPARENT),
                            )
                            .on_hover_text(t.about)
                            .clicked()
                        {
                            self.show_about = true;
                        }
                        if ui
                            .add(
                                egui::Button::new(egui::RichText::new(if self.settings.theme == "light" {
                                    "🌙"
                                } else {
                                    "☀"
                                })
                                .small())
                                .fill(egui::Color32::TRANSPARENT),
                            )
                            .on_hover_text(t.settings_theme)
                            .clicked()
                        {
                            let next = if self.settings.theme == "light" { "dark" } else { "light" };
                            self.set_theme(ctx, next);
                        }
                        // workspace dock toggle (chat only)
                        if self.view == View::Chat {
                            let dock_glyph = if self.dock.open { "◧" } else { "▤" };
                            let dock_fill = if self.dock.open {
                                p.panel_hover
                            } else {
                                egui::Color32::TRANSPARENT
                            };
                            if ui
                                .add(
                                    egui::Button::new(egui::RichText::new(dock_glyph).small())
                                        .fill(dock_fill),
                                )
                                .on_hover_text(t.dock_files)
                                .clicked()
                            {
                                self.dock.open = !self.dock.open;
                                if self.dock.open && self.dock.tab == crate::dock::DockTab::Git {
                                    let cwd = self.current_workdir();
                                    self.dock.refresh_status(&cwd);
                                }
                            }
                        }
                        if let Some((msg, at)) = &self.toast {
                            if at.elapsed() < Duration::from_secs(3) {
                                ui.label(egui::RichText::new(msg).small().color(p.green));
                            }
                        }
                    });
                });

                // The header content doubles as the titlebar drag region
                // (Nexus-style frameless window; the right-side buttons stay
                // clickable because they were allocated earlier and win hits).
                let mut drag = ui.max_rect();
                drag.max.x -= 210.0; // window buttons + toggles
                let resp = ui.interact(drag, Id::new("titlebar-drag"), Sense::drag());
                if resp.dragged() {
                    // OS move with the press hand-off (see frameless.rs) —
                    // a plain StartDrag leaves egui thinking the button is
                    // still down and the next move/resize dies.
                    crate::frameless::start_os_move(ctx);
                } else if resp.double_clicked() {
                    let maximized = ctx.input(|i| i.viewport().maximized.unwrap_or(false));
                    ctx.send_viewport_cmd(ViewportCommand::Maximized(!maximized));
                }
            });

        // -- sidebar (always visible; collapsible rail) --------------------
        if self.settings.sidebar_collapsed {
            SidePanel::left("sidebar-rail")
                .exact_width(46.0)
                .frame(
                    egui::Frame::default()
                        .fill(p.surface)
                        .inner_margin(egui::Margin::same(4.0))
                        .stroke(egui::Stroke::new(1.0_f32, p.border_subtle)),
                )
                .show(ctx, |ui| crate::sessions::show_rail(self, ui));
        } else {
            SidePanel::left("sessions")
                .resizable(true)
                .default_width(220.0)
                .width_range(120.0..=400.0)
                .frame(
                    egui::Frame::default()
                        .fill(p.surface)
                        .inner_margin(egui::Margin::same(8.0))
                        .stroke(egui::Stroke::new(1.0_f32, p.border_subtle)),
                )
                .show(ctx, |ui| crate::sessions::show(self, ui));
        }

        // -- main column ----------------------------------------------------
        match self.view {
            View::Chat => {
                crate::dock::panel(self, ctx);
                TopBottomPanel::bottom("input")
                    .frame(
                        egui::Frame::default()
                            .fill(p.bg)
                            .inner_margin(egui::Margin::symmetric(12.0, 8.0)),
                    )
                    .show(ctx, |ui| crate::input::show(self, ui));
                CentralPanel::default()
                    .frame(egui::Frame::default().fill(p.bg))
                    .show(ctx, |ui| crate::chat::show(self, ui));
            }
            View::Settings | View::Stats | View::Setup => {
                CentralPanel::default()
                    .frame(egui::Frame::default().fill(p.bg))
                    .show(ctx, |ui| {
                        // Non-chat views render in the main column, next to
                        // the sidebar — with an explicit close button.
                        ui.horizontal(|ui| {
                            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                if ui
                                    .add(
                                        egui::Button::new(egui::RichText::new(format!("✕ {}", t.close)).small())
                                            .fill(p.panel),
                                    )
                                    .clicked()
                                {
                                    self.view = View::Chat;
                                }
                            });
                        });
                        ui.separator();
                        match self.view {
                            View::Settings => crate::settings::show(self, ui),
                            View::Stats => crate::stats::show(self, ui),
                            View::Setup => crate::setup::show(self, ui),
                            _ => {}
                        }
                    });
            }
        }

        self.ui_windows(ctx);
    }

    /// Poll the background dependency probe; auto-connect on first result.
    fn poll_setup(&mut self) {
        let fresh = self.setup_pane.poll_status();
        if !fresh {
            return;
        }
        if self.setup_pane.autodetect {
            // Settings → auto-detect: fill in the path only.
            self.setup_pane.autodetect = false;
            if let Some(p) = self.setup_pane.status.vibe_acp_path.clone() {
                self.settings.acp_path = p.to_string_lossy().to_string();
                self.settings.save();
                self.notify(self.tr().saved);
            }
        } else if self.settings.acp_path.is_empty() {
            // Startup probe: connect right away when vibe-acp is available.
            if let Some(path) = self.setup_pane.status.vibe_acp_path.clone() {
                let path_str = path.to_string_lossy().to_string();
                self.start_agent(&path_str);
                self.view = View::Chat;
            }
        }
    }

    fn ui_windows(&mut self, ctx: &Context) {
        let t = self.tr();
        let p = self.palette();

        // About window.
        if self.show_about {
            let mut show_about = true;
            Window::new(t.about)
                .open(&mut show_about)
                .resizable(false)
                .show(ctx, |ui| crate::about::show(self, ui));
            self.show_about = show_about;
        }

        // Tool approval modal — visible while `approval` is set.
        if let Some(appr) = self.approval.as_mut() {
            let mut decision: Option<&'static str> = None;
            Window::new(egui::RichText::new(format!("🔒 {}", t.approve)).strong())
                .resizable(false)
                .collapsible(false)
                .show(ctx, |ui| {
                    ui.label(t.tool_calls);
                    ui.label(
                        egui::RichText::new(
                            appr.tool_name
                                .clone()
                                .unwrap_or_else(|| appr.method.clone()),
                        )
                        .strong(),
                    );
                    ui.add_space(6.0);
                    egui::ScrollArea::vertical()
                        .max_height(220.0)
                        .show(ui, |ui| {
                            ui.style_mut().override_text_style = Some(egui::TextStyle::Monospace);
                            ui.label(&appr.params_json);
                        });
                    ui.add_space(6.0);
                    ui.horizontal(|ui| {
                        if ui
                            .add(
                                egui::Button::new(
                                    egui::RichText::new(format!("✅ {}", t.approve))
                                        .strong()
                                        .color(egui::Color32::WHITE),
                                )
                                .fill(p.accent),
                            )
                            .clicked()
                        {
                            decision = Some("proceed_once");
                        }
                        if ui.button(format!("❌ {}", t.reject)).clicked() {
                            decision = Some("reject");
                        }
                    });
                });
            if let Some(option_id) = decision {
                self.acp.send(AcpCommand::Approve {
                    request_id: appr.request_id.clone(),
                    option_id: option_id.into(),
                });
                self.approval = None;
            }
        }

        // Agent question modal — visible while `question` is set.
        if let Some(quest) = self.question.as_mut() {
            let mut submitted: Option<String> = None;
            Window::new(egui::RichText::new(format!("❓ {}", t.agent_question)).strong())
                .resizable(false)
                .collapsible(false)
                .show(ctx, |ui| {
                    ui.label(&quest.question);
                    ui.add_space(6.0);
                    if !quest.choices.is_empty() {
                        let choices = quest.choices.clone();
                        for choice in choices {
                            if ui.button(&choice).clicked() {
                                submitted = Some(choice);
                            }
                        }
                    } else {
                        ui.text_edit_singleline(&mut quest.answer);
                        if ui.button(t.answer).clicked() {
                            submitted = Some(quest.answer.clone());
                        }
                    }
                });
            if let Some(answer) = submitted {
                self.acp.send(AcpCommand::AnswerQuestion {
                    request_id: quest.request_id.clone(),
                    answer,
                });
                self.question = None;
            }
        }

        // In-app folder picker (fixed size so the confirm row never
        // falls off-screen).
        if self.folder_picker.open {
            let mut open = true;
            Window::new(t.pick_folder)
                .open(&mut open)
                .fixed_size([520.0, 440.0])
                .resizable(false)
                .show(ctx, |ui| crate::picker::show_folder_picker(self, ui));
            self.folder_picker.open = open && self.folder_picker.open;
        }
    }
}

/// Find or create the streaming AI message of a session.
pub fn ensure_ai_message(session: &mut ChatSession) -> &mut Message {
    let needs_new = !matches!(
        session.messages.last(),
        Some(msg) if msg.sender == MessageSender::Ai && !msg.is_complete
    );
    if needs_new {
        session.messages.push(Message::new_ai());
    }
    session.messages.last_mut().expect("just pushed")
}

impl eframe::App for VibedeckApp {
    fn update(&mut self, ctx: &Context, _frame: &mut eframe::Frame) {
        self.ui(ctx);
    }

    /// Ends the press the OS kept during a window move/resize
    /// (see `frameless::release_after_os_resize`).
    fn raw_input_hook(&mut self, ctx: &Context, raw: &mut egui::RawInput) {
        crate::frameless::release_after_os_resize(ctx, raw);
    }
}
