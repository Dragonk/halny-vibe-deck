//! The egui application: top-level layout, event pump, and glue
//! between the ACP worker, the `~/.vibe` config layer and the UI.
//!
//! Layout mirrors the original Mistral Vibe Nexus app:
//! a slim header (brand + agent status), a resizable sessions/projects
//! sidebar, the chat column, and the input bar.

use std::path::PathBuf;
use std::time::Duration;

use eframe::egui;
use egui::{Align, CentralPanel, Context, Layout, SidePanel, TopBottomPanel, Window};
use vibedeck_acp::AcpHandle;
use vibedeck_core::proto::{AcpCommand, AcpEvent, AcpSessionInfo, AvailableCommand, ModeInfo};
use vibedeck_core::types::{ChatSession, Message, MessageSender, ThinkingBlock};

use crate::i18n::{t, Lang};
use crate::settings::SettingsPane;
use crate::setup::SetupPane;
use crate::state::{AcpStatus, AppSettings, FolderPicker, PendingApproval, PendingQuestion, View};
use crate::stats::StatsPane;
use crate::theme;

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
    pub models: Vec<String>,
    pub modes: Vec<ModeInfo>,
    pub commands: Vec<AvailableCommand>,

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
            models: Vec::new(),
            modes: Vec::new(),
            commands: Vec::new(),
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
        if repaint {
            ctx.request_repaint();
        }
        if self.busy || self.setup_pane.is_working() {
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
                models,
                title,
            } => {
                self.status = AcpStatus::Connected;
                self.busy = false;
                self.modes = modes;
                self.models = models;
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

        // -- header (Nexus-style: brand + agent status + theme toggle) ----------
        TopBottomPanel::top("header")
            .frame(
                egui::Frame::default()
                    .fill(p.bg)
                    .inner_margin(egui::Margin::symmetric(12.0, 6.0))
                    .stroke(egui::Stroke::new(1.0_f32, p.border_subtle)),
            )
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    // brand: pixel flame + name
                    theme::paint_flame(ui, &p, 0.13);
                    ui.add_space(6.0);
                    ui.heading(egui::RichText::new(t.app_name).strong().color(p.text));

                    // status: "● Connected" with tooltip
                    let (status_text, color) = match self.status {
                        AcpStatus::Connected => (t.connected, p.green),
                        AcpStatus::Connecting => (t.connecting, p.yellow),
                        AcpStatus::Disconnected => (t.disconnected, p.muted),
                        AcpStatus::Error => (t.error, p.red),
                    };
                    ui.add_space(10.0);
                    let status_label = ui
                        .label(
                            egui::RichText::new(format!("● {status_text}"))
                                .small()
                                .color(color),
                        )
                        .on_hover_text(if self.acp_error.is_empty() {
                            status_text.to_string()
                        } else {
                            format!("{status_text} — {}", self.acp_error)
                        });
                    let _ = status_label;
                    if self.busy {
                        ui.label(egui::RichText::new("⏳").small());
                    }

                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if ui
                            .add(
                                egui::Button::new(
                                    egui::RichText::new(if self.settings.theme == "light" {
                                        "🌙"
                                    } else {
                                        "☀"
                                    })
                                    .small(),
                                )
                                .fill(p.panel),
                            )
                            .on_hover_text(t.settings_theme)
                            .clicked()
                        {
                            let next = if self.settings.theme == "light" { "dark" } else { "light" };
                            self.set_theme(ctx, next);
                        }
                        if ui
                            .add(egui::Button::new(egui::RichText::new("ℹ").small()).fill(p.panel))
                            .on_hover_text(t.about)
                            .clicked()
                        {
                            self.show_about = true;
                        }
                        if let Some((msg, at)) = &self.toast {
                            if at.elapsed() < Duration::from_secs(3) {
                                ui.label(
                                    egui::RichText::new(msg)
                                        .small()
                                        .color(p.green),
                                );
                            }
                        }
                    });
                });
            });

        match self.view {
            View::Chat => {
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
                TopBottomPanel::bottom("input")
                    .frame(
                        egui::Frame::default()
                            .fill(p.bg)
                            .inner_margin(egui::Margin::symmetric(12.0, 8.0))
                            .stroke(egui::Stroke::new(1.0_f32, p.border_subtle)),
                    )
                    .show(ctx, |ui| crate::input::show(self, ui));
                CentralPanel::default()
                    .frame(egui::Frame::default().fill(p.bg))
                    .show(ctx, |ui| crate::chat::show(self, ui));
            }
            View::Settings => {
                CentralPanel::default()
                    .frame(egui::Frame::default().fill(p.bg))
                    .show(ctx, |ui| crate::settings::show(self, ui));
            }
            View::Stats => {
                CentralPanel::default()
                    .frame(egui::Frame::default().fill(p.bg))
                    .show(ctx, |ui| crate::stats::show(self, ui));
            }
            View::Setup => {
                CentralPanel::default()
                    .frame(egui::Frame::default().fill(p.bg))
                    .show(ctx, |ui| crate::setup::show(self, ui));
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

        // In-app folder picker.
        if self.folder_picker.open {
            let mut open = true;
            Window::new(t.pick_folder)
                .open(&mut open)
                .default_size([520.0, 420.0])
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
}
