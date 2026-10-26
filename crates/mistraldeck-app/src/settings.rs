//! Settings view: editor for `~/.vibe/config.toml` (latest CLI schema,
//! round-trip safe), agent/prompt/skill management and app settings.
//!
//! Editing model: a *working copy* of the config lives in the pane and
//! the text fields bind directly to it. The top-level **Save** button
//! applies the working copy into the round-trip [`VibeConfigFile`]
//! (preserving every unknown key) and writes it to disk.

use eframe::egui::{self, Ui};
use toml::Value;
use mistraldeck_vibe::{
    AgentFile, McpServer, ModelConfig, PromptFile, Provider, SkillFile, VibeConfigFile,
};

use crate::app::MistraldeckApp;
use crate::i18n::{Lang, Str};
use crate::state::View;

const PERMISSIONS: [&str; 3] = ["allow", "ask", "deny"];
const TRANSPORTS: [&str; 3] = ["http", "streamable-http", "stdio"];

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SettingsTab {
    #[default]
    General,
    Acp,
    Providers,
    Models,
    Mcp,
    Tools,
    Skills,
    Agents,
    Prompts,
    Raw,
}

// -- working copies -----------------------------------------------------------

#[derive(Clone, Debug)]
pub struct ProviderWorking {
    pub p: Provider,
    pub headers_buf: String,
}

impl From<Provider> for ProviderWorking {
    fn from(p: Provider) -> Self {
        Self {
            headers_buf: pairs_to_lines(&p.extra_headers),
            p,
        }
    }
}

impl ProviderWorking {
    fn synced(&self) -> Provider {
        let mut p = self.p.clone();
        p.extra_headers = lines_to_pairs(&self.headers_buf);
        p
    }
}

#[derive(Clone, Debug)]
pub struct McpWorking {
    pub s: McpServer,
    pub args_buf: String,
    pub env_buf: String,
    pub headers_buf: String,
    pub st_buf: String,
    pub tt_buf: String,
}

impl From<McpServer> for McpWorking {
    fn from(s: McpServer) -> Self {
        Self {
            args_buf: s.args.join("\n"),
            env_buf: pairs_to_lines(&s.env),
            headers_buf: pairs_to_lines(&s.headers),
            st_buf: s
                .startup_timeout_sec
                .map(|v| v.to_string())
                .unwrap_or_default(),
            tt_buf: s
                .tool_timeout_sec
                .map(|v| v.to_string())
                .unwrap_or_default(),
            s,
        }
    }
}

impl McpWorking {
    fn synced(&self) -> McpServer {
        let mut s = self.s.clone();
        s.args = lines_to_list(&self.args_buf);
        s.env = lines_to_pairs(&self.env_buf);
        s.headers = lines_to_pairs(&self.headers_buf);
        s.startup_timeout_sec = self.st_buf.trim().parse().ok();
        s.tool_timeout_sec = self.tt_buf.trim().parse().ok();
        s
    }
}

#[derive(Clone, Debug)]
pub struct AgentWorking {
    pub a: AgentFile,
    pub original_name: String,
    pub enabled_buf: String,
    pub disabled_buf: String,
}

impl From<AgentFile> for AgentWorking {
    fn from(a: AgentFile) -> Self {
        Self {
            enabled_buf: a.enabled_tools.join("\n"),
            disabled_buf: a.disabled_tools.join("\n"),
            original_name: a.name.clone(),
            a,
        }
    }
}

impl AgentWorking {
    fn synced(&self) -> AgentFile {
        let mut a = self.a.clone();
        a.enabled_tools = lines_to_list(&self.enabled_buf);
        a.disabled_tools = lines_to_list(&self.disabled_buf);
        a
    }
}

#[derive(Clone, Debug)]
pub struct PromptWorking {
    pub p: PromptFile,
    pub original_name: String,
}

impl From<PromptFile> for PromptWorking {
    fn from(p: PromptFile) -> Self {
        Self {
            original_name: p.name.clone(),
            p,
        }
    }
}

fn lines_to_list(buf: &str) -> Vec<String> {
    buf.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .map(String::from)
        .collect()
}

fn lines_to_pairs(buf: &str) -> Vec<(String, String)> {
    buf.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .filter_map(|l| {
            l.split_once('=')
                .map(|(k, v)| (k.trim().to_string(), v.trim().to_string()))
        })
        .collect()
}

fn pairs_to_lines(pairs: &[(String, String)]) -> String {
    pairs
        .iter()
        .map(|(k, v)| format!("{k}={v}"))
        .collect::<Vec<_>>()
        .join("\n")
}

// -- pane -----------------------------------------------------------------------

#[derive(Default)]
pub struct SettingsPane {
    pub tab: SettingsTab,
    pub cfg: Option<VibeConfigFile>,

    // working copies of config.toml
    pub active_model: String,
    pub default_agent: String,
    pub cli_theme: String,
    pub providers: Vec<ProviderWorking>,
    pub models: Vec<ModelConfig>,
    pub mcp: Vec<McpWorking>,
    pub enabled_tools: String,
    pub disabled_tools: String,
    pub tool_permissions: Vec<(String, String)>,
    pub new_tool: String,
    pub new_perm: String,
    pub skill_paths: String,
    pub enabled_skills: String,
    pub disabled_skills: String,
    pub telemetry: bool,
    pub otel: bool,
    pub otel_endpoint: String,
    pub update_checks: bool,
    pub notifications: bool,
    pub trust_store: bool,
    pub sl_enabled: bool,
    pub sl_save_dir: String,
    pub sl_threshold: String,

    // on-disk extras
    pub agents: Vec<AgentWorking>,
    pub agent_sel: Option<usize>,
    pub prompts: Vec<PromptWorking>,
    pub prompt_sel: Option<usize>,
    pub skills: Vec<SkillFile>,
}

impl SettingsPane {
    pub fn ensure_loaded(&mut self) {
        if self.cfg.is_none() {
            self.reload();
        }
    }

    pub fn reload(&mut self) {
        let cfg = VibeConfigFile::load();

        self.active_model = cfg.active_model().unwrap_or_default();
        self.default_agent = cfg.default_agent();
        self.cli_theme = cfg.theme();
        self.providers = cfg
            .providers()
            .into_iter()
            .map(ProviderWorking::from)
            .collect();
        self.models = cfg.models();
        self.mcp = cfg
            .mcp_servers()
            .into_iter()
            .map(McpWorking::from)
            .collect();
        self.enabled_tools = cfg.enabled_tools().join("\n");
        self.disabled_tools = cfg.disabled_tools().join("\n");
        self.skill_paths = cfg.skill_paths().join("\n");
        self.enabled_skills = cfg.enabled_skills().join("\n");
        self.disabled_skills = cfg.disabled_skills().join("\n");
        self.telemetry = cfg.enable_telemetry();
        self.otel = cfg.enable_otel();
        self.otel_endpoint = cfg.otel_endpoint().unwrap_or_default();
        self.update_checks = cfg.enable_update_checks();
        self.notifications = cfg.enable_notifications();
        self.trust_store = cfg.enable_system_trust_store();

        self.tool_permissions = Vec::new();
        if let Some(Value::Table(tools)) = cfg.table.get("tools") {
            for (tool, entry) in tools {
                if let Some(p) = entry
                    .as_table()
                    .and_then(|t| t.get("permission"))
                    .and_then(Value::as_str)
                {
                    self.tool_permissions.push((tool.clone(), p.to_string()));
                }
            }
        }

        let sl = cfg.session_logging().unwrap_or_default();
        self.sl_enabled = sl.get("enabled").and_then(Value::as_bool).unwrap_or(true);
        self.sl_save_dir = sl
            .get("save_dir")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        self.sl_threshold = sl
            .get("auto_compact_threshold")
            .and_then(Value::as_integer)
            .map(|v| v.to_string())
            .unwrap_or_default();

        self.agents = AgentFile::load_all()
            .into_iter()
            .map(AgentWorking::from)
            .collect();
        self.agent_sel = None;
        self.prompts = PromptFile::load_all()
            .into_iter()
            .map(PromptWorking::from)
            .collect();
        self.prompt_sel = None;
        self.skills = SkillFile::load_all();

        self.cfg = Some(cfg);
    }

    /// Apply the working copy into the round-trip config and write to disk.
    pub fn save_to_disk(&mut self) -> anyhow::Result<()> {
        let Some(cfg) = self.cfg.as_mut() else {
            return Ok(());
        };

        if self.active_model.trim().is_empty() {
            cfg.remove_key("active_model");
        } else {
            cfg.set_active_model(self.active_model.trim());
        }

        if self.default_agent.trim().is_empty() {
            cfg.remove_key("default_agent");
        } else {
            cfg.set_default_agent(self.default_agent.trim());
        }
        if self.cli_theme.trim().is_empty() {
            cfg.remove_key("theme");
        } else {
            cfg.set_theme(self.cli_theme.trim());
        }

        cfg.remove_key("providers");
        for pw in &self.providers {
            let p = pw.synced();
            if !p.name.trim().is_empty() {
                cfg.upsert_provider(&p);
            }
        }

        cfg.remove_key("models");
        for m in &self.models {
            if !m.name.trim().is_empty() {
                cfg.upsert_model(m);
            }
        }

        cfg.remove_key("mcp_servers");
        for mw in &self.mcp {
            let s = mw.synced();
            if !s.name.trim().is_empty() {
                cfg.upsert_mcp_server(&s);
            }
        }

        set_list_or_remove(cfg, "enabled_tools", &self.enabled_tools);
        set_list_or_remove(cfg, "disabled_tools", &self.disabled_tools);
        for (tool, perm) in &self.tool_permissions {
            if !tool.trim().is_empty() {
                cfg.set_tool_permission(tool.trim(), perm);
            }
        }

        set_list_or_remove(cfg, "skill_paths", &self.skill_paths);
        set_list_or_remove(cfg, "enabled_skills", &self.enabled_skills);
        set_list_or_remove(cfg, "disabled_skills", &self.disabled_skills);

        cfg.set_bool("enable_telemetry", self.telemetry);
        cfg.set_bool("enable_otel", self.otel);
        if self.otel_endpoint.trim().is_empty() {
            cfg.remove_key("otel_endpoint");
        } else {
            cfg.set_str("otel_endpoint", self.otel_endpoint.trim());
        }
        cfg.set_bool("enable_update_checks", self.update_checks);
        cfg.set_bool("enable_notifications", self.notifications);
        cfg.set_bool("enable_system_trust_store", self.trust_store);

        cfg.set_session_logging_bool("enabled", self.sl_enabled);
        if self.sl_save_dir.trim().is_empty() {
            // keep whatever the CLI had, or nothing
        } else {
            cfg.set_session_logging_str("save_dir", self.sl_save_dir.trim());
        }
        if let Ok(v) = self.sl_threshold.trim().parse::<i64>() {
            cfg.set_session_logging_i64("auto_compact_threshold", v);
        }

        cfg.save()
    }
}

fn set_list_or_remove(cfg: &mut VibeConfigFile, key: &str, buf: &str) {
    let items = lines_to_list(buf);
    if items.is_empty() {
        cfg.remove_key(key);
    } else {
        cfg.set_string_list(key, items);
    }
}

// -- top-level view --------------------------------------------------------------

pub fn show(app: &mut MistraldeckApp, ui: &mut Ui) {
    let t = app.tr();
    let p = app.palette();
    app.settings_pane.ensure_loaded();

    let tabs = [
        (SettingsTab::General, t.settings_general),
        (SettingsTab::Acp, t.settings_acp_agent),
        (SettingsTab::Providers, t.providers),
        (SettingsTab::Models, t.models),
        (SettingsTab::Mcp, t.settings_mcp),
        (SettingsTab::Tools, t.settings_tools),
        (SettingsTab::Skills, t.settings_skills),
        (SettingsTab::Agents, t.settings_agents),
        (SettingsTab::Prompts, t.settings_prompts),
        (SettingsTab::Raw, t.settings_raw_toml),
    ];

    ui.horizontal(|ui| {
        ui.horizontal_wrapped(|ui| {
            for (tab, label) in tabs {
                let selected = app.settings_pane.tab == tab;
                if ui
                    .add(
                        egui::Button::new(
                            egui::RichText::new(label)
                                .small()
                                .color(if selected { p.text } else { p.muted }),
                        )
                        .fill(if selected {
                            p.panel_hover
                        } else {
                            egui::Color32::TRANSPARENT
                        })
                        .rounding(egui::Rounding::same(5.0)),
                    )
                    .on_hover_text(label)
                    .clicked()
                {
                    app.settings_pane.tab = tab;
                }
            }
        });
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui.button(t.reload).clicked() {
                app.settings_pane.reload();
            }
            if ui.button(format!("💾 {}", t.save)).clicked() {
                match app.settings_pane.save_to_disk() {
                    Ok(()) => app.notify(t.saved),
                    Err(e) => app.notify(format!("{}: {e}", t.error)),
                }
            }
        });
    });
    ui.separator();

    egui::ScrollArea::vertical()
        .auto_shrink(false)
        .show(ui, |ui| match app.settings_pane.tab {
            SettingsTab::General => general(app, ui, t),
            SettingsTab::Acp => acp(app, ui, t),
            SettingsTab::Providers => providers(app, ui, t),
            SettingsTab::Models => models(app, ui, t),
            SettingsTab::Mcp => mcp(app, ui, t),
            SettingsTab::Tools => tools(app, ui, t),
            SettingsTab::Skills => skills(app, ui, t),
            SettingsTab::Agents => agents(app, ui, t),
            SettingsTab::Prompts => prompts(app, ui, t),
            SettingsTab::Raw => raw(app, ui, t),
        });
}

// -- General ---------------------------------------------------------------------

fn general(app: &mut MistraldeckApp, ui: &mut Ui, t: &Str) {
    let p = app.palette();

    // -- Application -----------------------------------------------------------
    section(ui, &p, t.settings_section_app);

    // Language: Automatic (System) + all supported UI languages
    ui.horizontal(|ui| {
        ui.label(t.settings_language);
        let mut picked: Option<String> = None;
        egui::ComboBox::from_id_salt("settings-language")
            .selected_text(if app.settings.language == "system" {
                t.settings_system.to_string()
            } else {
                Lang::from_code(&app.settings.language)
                    .native_name()
                    .to_string()
            })
            .width(180.0)
            .show_ui(ui, |ui| {
                ui.selectable_value(
                    &mut picked,
                    Some("system".to_string()),
                    t.settings_system,
                );
                for lang in Lang::ALL {
                    ui.selectable_value(
                        &mut picked,
                        Some(lang.code().to_string()),
                        lang.native_name(),
                    );
                }
            });
        if let Some(code) = picked {
            app.lang = Lang::resolve(&code);
            app.settings.language = code;
            app.settings.save();
        }
    });

    // Theme: System / Dark / Light
    ui.horizontal(|ui| {
        ui.label(t.settings_theme);
        let theme_label = |code: &str| -> String {
            match code {
                "dark" => t.settings_dark.to_string(),
                "light" => t.settings_light.to_string(),
                _ => t.settings_system.to_string(),
            }
        };
        let mut picked: Option<&'static str> = None;
        egui::ComboBox::from_id_salt("settings-theme")
            .selected_text(theme_label(&app.settings.theme))
            .width(180.0)
            .show_ui(ui, |ui| {
                ui.selectable_value(&mut picked, Some("system"), theme_label("system"));
                ui.selectable_value(&mut picked, Some("dark"), theme_label("dark"));
                ui.selectable_value(&mut picked, Some("light"), theme_label("light"));
            });
        if let Some(th) = picked {
            // saves + applies (resolving "system" against the OS theme)
            app.set_theme(ui.ctx(), th);
        }
    });

    // Sessions shown in the sidebar
    ui.horizontal(|ui| {
        ui.label(t.settings_sessions_limit);
        let mut limit = app.settings.sidebar_sessions_limit.unwrap_or(30) as f64;
        if ui.add(egui::DragValue::new(&mut limit)).changed() {
            app.settings.sidebar_sessions_limit = Some(limit.clamp(5.0, 100.0) as usize);
            app.settings.save();
        }
    });

    // -- Agent (ACP) ---------------------------------------------------------------
}

fn acp(app: &mut MistraldeckApp, ui: &mut Ui, t: &Str) {
    let p = app.palette();

    section(ui, &p, t.settings_section_agent);

    ui.horizontal(|ui| {
        ui.label(t.settings_acp_path);
        if ui
            .add(
                egui::TextEdit::singleline(&mut app.settings.acp_path)
                    .desired_width(340.0)
                    .font(egui::TextStyle::Monospace),
            )
            .changed()
        {
            app.settings.save();
        }
    });
    ui.horizontal(|ui| {
        if ui.button(t.settings_acp_autodetect).clicked() {
            app.setup_pane.autodetect = true;
            app.setup_pane.refresh_status_async();
        }
        let connect_label = t.open;
        if ui.button(format!("🔌 {connect_label}")).clicked() {
            if !app.settings.acp_path.is_empty() {
                let path = app.settings.acp_path.clone();
                app.start_agent(&path);
                app.view = View::Chat;
            }
        }
        if ui.button(t.vibe_dir_open).clicked() {
            open_path(&mistraldeck_vibe::vibe_home());
        }
        if ui
            .button(format!("🛠 {}", t.settings_open_setup))
            .clicked()
        {
            app.setup_pane.refresh_status_async();
            app.view = View::Setup;
        }
    });

    // -- Working directory -------------------------------------------------------
    section(ui, &p, t.settings_section_workdir);
    ui.horizontal(|ui| {
        ui.label(t.working_dir);
        ui.label(
            egui::RichText::new(app.current_workdir().to_string_lossy().to_string())
                .monospace()
                .weak(),
        );
        if ui.button(t.browse).clicked() {
            app.folder_picker.dir = app.current_workdir();
            app.folder_picker.entry = app.settings.working_directory.clone();
            app.folder_picker.open = true;
        }
    });

    // -- Mistral Vibe CLI (~/.vibe/config.toml) ----------------------------------
    section(ui, &p, t.settings_section_cli);
    let pane = &mut app.settings_pane;

    // Active model
    ui.horizontal(|ui| {
        ui.label(t.settings_active_model);
        let mut choices: Vec<String> = pane
            .models
            .iter()
            .filter_map(|m| {
                m.alias
                    .as_deref()
                    .filter(|a| !a.is_empty())
                    .or_else(|| (!m.name.is_empty()).then_some(m.name.as_str()))
                    .map(str::to_string)
            })
            .collect();
        if !pane.active_model.is_empty() && !choices.contains(&pane.active_model) {
            choices.push(pane.active_model.clone());
        }
        let mut picked: Option<String> = None;
        egui::ComboBox::from_id_salt("settings-cli-model")
            .selected_text(if pane.active_model.is_empty() {
                "—".to_string()
            } else {
                pane.active_model.clone()
            })
            .width(220.0)
            .show_ui(ui, |ui| {
                for v in &choices {
                    ui.selectable_value(&mut picked, Some(v.clone()), v);
                }
            });
        if let Some(v) = picked {
            pane.active_model = v;
        }
    });

    // Default agent
    ui.horizontal(|ui| {
        ui.label(t.settings_default_agent);
        let mut choices: Vec<String> = vec!["accept-edits".into(), "ask".into()];
        for a in &pane.agents {
            if a.a.agent_type.as_deref() != Some("subagent")
                && !a.a.name.is_empty()
                && !choices.contains(&a.a.name)
            {
                choices.push(a.a.name.clone());
            }
        }
        if !pane.default_agent.is_empty() && !choices.contains(&pane.default_agent) {
            choices.push(pane.default_agent.clone());
        }
        let mut picked: Option<String> = None;
        egui::ComboBox::from_id_salt("settings-cli-agent")
            .selected_text(if pane.default_agent.is_empty() {
                "—".to_string()
            } else {
                pane.default_agent.clone()
            })
            .width(220.0)
            .show_ui(ui, |ui| {
                for v in &choices {
                    ui.selectable_value(&mut picked, Some(v.clone()), v);
                }
            });
        if let Some(v) = picked {
            pane.default_agent = v;
        }
    });

    // CLI theme
    ui.horizontal(|ui| {
        ui.label(t.settings_cli_theme);
        let theme_label = |code: &str| -> &str {
            match code {
                "light" => t.settings_light,
                _ => t.settings_dark,
            }
        };
        let mut picked: Option<String> = None;
        egui::ComboBox::from_id_salt("settings-cli-theme")
            .selected_text(if pane.cli_theme.is_empty() {
                t.settings_dark.to_string()
            } else {
                theme_label(&pane.cli_theme).to_string()
            })
            .width(180.0)
            .show_ui(ui, |ui| {
                for v in ["dark", "light"] {
                    ui.selectable_value(&mut picked, Some(v.to_string()), theme_label(v));
                }
            });
        if let Some(v) = picked {
            pane.cli_theme = v;
        }
    });

    ui.add_space(4.0);
    ui.label(egui::RichText::new(t.settings_cli_note).small().weak());

    // -- Session logging ----------------------------------------------------------
    section(ui, &p, t.settings_session_logging);
    egui::Grid::new("logging-grid")
        .num_columns(2)
        .spacing([20.0, 8.0])
        .show(ui, |ui| {
            checkbox_field(ui, &mut pane.sl_enabled, "enabled");
            label_field(ui, "save_dir", &mut pane.sl_save_dir);
            label_field(ui, "auto_compact_threshold", &mut pane.sl_threshold);
        });

    // -- System / privacy ---------------------------------------------------------
    section(ui, &p, t.settings_section_system);
    egui::Grid::new("system-grid")
        .num_columns(2)
        .spacing([20.0, 8.0])
        .show(ui, |ui| {
            checkbox_field(ui, &mut pane.trust_store, t.system_trust_store);
            checkbox_field(ui, &mut pane.telemetry, t.telemetry);
            checkbox_field(ui, &mut pane.otel, "enable_otel");
            label_field(ui, t.otel_endpoint, &mut pane.otel_endpoint);
            checkbox_field(ui, &mut pane.update_checks, t.update_checks);
            checkbox_field(ui, &mut pane.notifications, t.notifications);
        });
}

/// A settings section header.
fn section(ui: &mut Ui, p: &crate::theme::Palette, text: &str) {
    ui.add_space(10.0);
    ui.label(egui::RichText::new(text).strong().color(p.accent));
    ui.add_space(2.0);
}

fn open_path(path: &std::path::Path) {
    #[cfg(target_os = "windows")]
    {
        let _ = std::process::Command::new("explorer").arg(path).spawn();
    }
    #[cfg(target_os = "macos")]
    {
        let _ = std::process::Command::new("open").arg(path).spawn();
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        let _ = std::process::Command::new("xdg-open").arg(path).spawn();
    }
}

// -- Providers ----------------------------------------------------------------------

fn providers(app: &mut MistraldeckApp, ui: &mut Ui, t: &Str) {
    let pane = &mut app.settings_pane;

    egui::Grid::new("active-model")
        .num_columns(2)
        .spacing([20.0, 6.0])
        .show(ui, |ui| {
            ui.label(t.active_model);
            ui.add(egui::TextEdit::singleline(&mut pane.active_model).desired_width(280.0));
            ui.end_row();
            let aliases = pane
                .models
                .iter()
                .filter_map(|m| {
                    m.alias
                        .clone()
                        .or_else(|| (!m.name.is_empty()).then(|| m.name.clone()))
                })
                .collect::<Vec<_>>()
                .join(", ");
            ui.label("");
            ui.label(egui::RichText::new(aliases).small().monospace().weak());
            ui.end_row();
        });
    ui.add_space(12.0);

    ui.separator();
    ui.label(egui::RichText::new(t.providers).strong());
    let mut remove_provider = None;
    for i in 0..pane.providers.len() {
        let pw = &mut pane.providers[i];
        let title = if pw.p.name.trim().is_empty() {
            "(—)"
        } else {
            &pw.p.name
        };
        egui::CollapsingHeader::new(egui::RichText::new(format!("🔧 {title}")).strong())
            .default_open(false)
            .id_salt(format!("provider-{i}"))
            .show(ui, |ui| {
                egui::Grid::new(format!("provider-grid-{i}"))
                    .num_columns(2)
                    .spacing([20.0, 6.0])
                    .show(ui, |ui| {
                        label_field(ui, t.name, &mut pw.p.name);
                        label_field(ui, "api_base", &mut pw.p.api_base);
                        option_field(ui, "api_key_env_var", &mut pw.p.api_key_env_var);
                        option_field(ui, "backend", &mut pw.p.backend);
                        option_field(ui, "api_style", &mut pw.p.api_style);
                        kv_field(ui, "extra_headers", &mut pw.headers_buf, t);
                    });
            });
        if ui.small_button(format!("🗑 {}", t.delete)).clicked() {
            remove_provider = Some(i);
        }
    }
    if let Some(i) = remove_provider {
        pane.providers.remove(i);
    }
    if ui
        .button(format!("➕ {} ({})", t.add, t.providers))
        .clicked()
    {
        let mut n = 1;
        while pane
            .providers
            .iter()
            .any(|pw| pw.p.name == format!("provider-{n}"))
        {
            n += 1;
        }
        pane.providers.push(ProviderWorking::from(Provider {
            name: format!("provider-{n}"),
            ..Provider::default()
        }));
    }

    ui.add_space(12.0);
}

// -- Models ------------------------------------------------------------------------

fn models(app: &mut MistraldeckApp, ui: &mut Ui, t: &Str) {
    let pane = &mut app.settings_pane;

    ui.label(egui::RichText::new(t.models).strong());
    let mut remove_model = None;
    for i in 0..pane.models.len() {
        let m = &mut pane.models[i];
        egui::Grid::new(format!("model-grid-{i}"))
            .num_columns(5)
            .spacing([20.0, 6.0])
            .show(ui, |ui| {
                let mut name = m.name.clone();
                if ui
                    .add(
                        egui::TextEdit::singleline(&mut name)
                            .desired_width(160.0)
                            .hint_text(t.name),
                    )
                    .changed()
                {
                    m.name = name;
                }
                let mut provider = m.provider.clone();
                if ui
                    .add(
                        egui::TextEdit::singleline(&mut provider)
                            .desired_width(120.0)
                            .hint_text(t.provider),
                    )
                    .changed()
                {
                    m.provider = provider;
                }
                option_field_inline(ui, "alias", &mut m.alias);
                let mut enabled = m.enabled.unwrap_or(true);
                if ui.checkbox(&mut enabled, "").changed() {
                    m.enabled = Some(enabled);
                }
                if ui.small_button(format!("🗑 {}", t.delete)).clicked() {
                    remove_model = Some(i);
                }
                ui.end_row();
            });
    }
    if let Some(i) = remove_model {
        pane.models.remove(i);
    }
    if ui.button(format!("➕ {} ({})", t.add, t.models)).clicked() {
        pane.models.push(ModelConfig {
            name: format!("model-{}", pane.models.len() + 1),
            provider: String::new(),
            alias: None,
            enabled: Some(true),
            display_name: None,
            thinking: None,
            thinking_levels: None,
            max_context_length: None,
            auto_compact_threshold: None,
            temperature: None,
            top_p: None,
            max_output_tokens: None,
            input_price: None,
            output_price: None,
            cached_input_price: None,
            supports_images: None,
        });
    }
}

// -- MCP -------------------------------------------------------------------------

fn mcp(app: &mut MistraldeckApp, ui: &mut Ui, t: &Str) {
    let pane = &mut app.settings_pane;
    ui.label(egui::RichText::new(t.mcp_servers).strong());
    ui.label(
        egui::RichText::new(format!(
            "transport: http | streamable-http | stdio — {}",
            t.one_per_line
        ))
        .small()
        .weak(),
    );
    ui.add_space(6.0);

    let mut remove_server = None;
    for i in 0..pane.mcp.len() {
        let mw = &mut pane.mcp[i];
        let title = if mw.s.name.trim().is_empty() {
            "(—)"
        } else {
            &mw.s.name
        };
        egui::CollapsingHeader::new(egui::RichText::new(format!("🔌 {title}")).strong())
            .default_open(false)
            .id_salt(format!("mcp-{i}"))
            .show(ui, |ui| {
                egui::Grid::new(format!("mcp-grid-{i}"))
                    .num_columns(2)
                    .spacing([20.0, 6.0])
                    .show(ui, |ui| {
                        label_field(ui, t.name, &mut mw.s.name);
                        ui.label(egui::RichText::new(t.transport).weak());
                        ui.horizontal(|ui| {
                            egui::ComboBox::from_id_salt(format!("mcp-transport-{i}"))
                                .selected_text(&mw.s.transport)
                                .show_ui(ui, |ui| {
                                    for tr in TRANSPORTS {
                                        ui.selectable_value(
                                            &mut mw.s.transport,
                                            tr.to_string(),
                                            tr,
                                        );
                                    }
                                });
                        });
                        ui.end_row();
                        option_field(ui, t.url, &mut mw.s.url);
                        option_field(ui, t.command, &mut mw.s.command);
                        label_field_multiline(ui, t.args, &mut mw.args_buf, t);
                        kv_field(ui, t.env_vars, &mut mw.env_buf, t);
                        kv_field(ui, "headers", &mut mw.headers_buf, t);
                        option_field(ui, t.api_key_env, &mut mw.s.api_key_env);
                        option_field(ui, "api_key_header", &mut mw.s.api_key_header);
                        option_field(ui, "api_key_format", &mut mw.s.api_key_format);
                        label_field(ui, "startup_timeout_sec", &mut mw.st_buf);
                        label_field(ui, "tool_timeout_sec", &mut mw.tt_buf);
                    });
            });
        if ui.small_button(format!("🗑 {}", t.delete)).clicked() {
            remove_server = Some(i);
        }
    }
    if let Some(i) = remove_server {
        pane.mcp.remove(i);
    }
    if ui
        .button(format!("➕ {} ({})", t.add, t.mcp_servers))
        .clicked()
    {
        let mut n = 1;
        while pane.mcp.iter().any(|mw| mw.s.name == format!("mcp-{n}")) {
            n += 1;
        }
        pane.mcp.push(McpWorking::from(McpServer {
            name: format!("mcp-{n}"),
            transport: "stdio".into(),
            ..McpServer::default()
        }));
    }
}

// -- Tools & telemetry --------------------------------------------------------------

fn tools(app: &mut MistraldeckApp, ui: &mut Ui, t: &Str) {
    let pane = &mut app.settings_pane;

    egui::Grid::new("tools-grid")
        .num_columns(2)
        .spacing([20.0, 8.0])
        .show(ui, |ui| {
            multiline_field(ui, t.enabled_tools, &mut pane.enabled_tools, t);
            multiline_field(ui, t.disabled_tools, &mut pane.disabled_tools, t);
        });

    ui.add_space(12.0);
    ui.separator();
    ui.label(egui::RichText::new(t.permissions).strong());
    ui.label(
        egui::RichText::new("[tools.<name>] permission = allow | ask | deny")
            .small()
            .weak(),
    );
    ui.add_space(6.0);

    // New permission row
    ui.horizontal(|ui| {
        ui.label(t.tool_name);
        ui.add(egui::TextEdit::singleline(&mut pane.new_tool).desired_width(140.0));
        ui.label(t.permission);
        egui::ComboBox::from_id_salt("new-perm")
            .selected_text(if pane.new_perm.is_empty() {
                "allow".to_string()
            } else {
                pane.new_perm.clone()
            })
            .show_ui(ui, |ui| {
                for p in PERMISSIONS {
                    ui.selectable_value(&mut pane.new_perm, p.to_string(), p);
                }
            });
        if ui.button(format!("➕ {}", t.add)).clicked() && !pane.new_tool.trim().is_empty() {
            let perm = if pane.new_perm.is_empty() {
                "allow".into()
            } else {
                pane.new_perm.clone()
            };
            let name = pane.new_tool.trim().to_string();
            match pane
                .tool_permissions
                .iter_mut()
                .find(|(tool, _)| *tool == name)
            {
                Some(entry) => entry.1 = perm,
                None => pane.tool_permissions.push((name, perm)),
            }
            pane.new_tool.clear();
        }
    });

    egui::Grid::new("perm-grid")
        .num_columns(3)
        .spacing([20.0, 6.0])
        .show(ui, |ui| {
            ui.strong(t.tool_name);
            ui.strong(t.permission);
            ui.label("");
            ui.end_row();
            let mut remove = None;
            for (i, (tool, perm)) in pane.tool_permissions.clone().into_iter().enumerate() {
                ui.label(egui::RichText::new(tool.clone()).monospace());
                let mut perm = perm;
                egui::ComboBox::from_id_salt(format!("perm-{i}"))
                    .selected_text(perm.clone())
                    .show_ui(ui, |ui| {
                        for p in PERMISSIONS {
                            ui.selectable_value(&mut perm, p.to_string(), p);
                        }
                    });
                if pane.tool_permissions[i].1 != perm {
                    pane.tool_permissions[i].1 = perm;
                }
                if ui.small_button("🗑").clicked() {
                    remove = Some(i);
                }
                ui.end_row();
            }
            if let Some(i) = remove {
                pane.tool_permissions.remove(i);
            }
        });
}

// -- Skills -------------------------------------------------------------------------

fn skills(app: &mut MistraldeckApp, ui: &mut Ui, t: &Str) {
    let pane = &mut app.settings_pane;

    egui::Grid::new("skills-grid")
        .num_columns(2)
        .spacing([20.0, 8.0])
        .show(ui, |ui| {
            multiline_field(ui, "skill_paths", &mut pane.skill_paths, t);
            multiline_field(ui, "enabled_skills", &mut pane.enabled_skills, t);
            multiline_field(ui, "disabled_skills", &mut pane.disabled_skills, t);
        });

    ui.add_space(12.0);
    ui.separator();
    ui.label(egui::RichText::new(t.skills_discovered).strong());
    if pane.skills.is_empty() {
        ui.label(egui::RichText::new("~/.vibe/skills/").weak());
    }
    for skill in &pane.skills {
        egui::CollapsingHeader::new(format!("📚 {}", skill.name))
            .default_open(false)
            .id_salt(format!("skill-{}", skill.name))
            .show(ui, |ui| {
                egui::ScrollArea::vertical()
                    .max_height(240.0)
                    .id_salt(format!("skill-scroll-{}", skill.name))
                    .show(ui, |ui| {
                        ui.style_mut().override_text_style = Some(egui::TextStyle::Monospace);
                        ui.label(&skill.content);
                    });
            });
    }
}

// -- Agents ----------------------------------------------------------------------------

fn agents(app: &mut MistraldeckApp, ui: &mut Ui, t: &Str) {
    let pane = &mut app.settings_pane;

    ui.label(egui::RichText::new("~/.vibe/agents/").monospace().weak());
    ui.add_space(6.0);
    ui.horizontal(|ui| {
        if ui.button(format!("➕ {}", t.add)).clicked() {
            let mut n = 1;
            while pane
                .agents
                .iter()
                .any(|aw| aw.a.name == format!("agent-{n}"))
            {
                n += 1;
            }
            pane.agents.push(AgentWorking::from(AgentFile {
                name: format!("agent-{n}"),
                ..AgentFile::default()
            }));
        }
        if pane.agents.is_empty() {
            ui.label(egui::RichText::new("(—)").weak());
        }
    });

    let mut remove_agent: Option<usize> = None;
    for i in 0..pane.agents.len() {
        let aw = &mut pane.agents[i];
        let is_sub = aw.a.agent_type.as_deref() == Some("subagent");
        let title = format!("{} {}", if is_sub { "🤖" } else { "🧭" }, aw.a.name);
        let selected = pane.agent_sel == Some(i);
        let mut select_agent = false;
        let mut delete_agent = false;
        egui::CollapsingHeader::new(egui::RichText::new(title).strong().color(if selected {
            egui::Color32::from_rgb(0x4d, 0xa3, 0xff)
        } else {
            ui.visuals().text_color()
        }))
        .default_open(selected)
        .id_salt(format!("agent-{i}"))
        .show(ui, |ui| {
            if ui.button(t.edit).clicked() {
                select_agent = true;
            }
            egui::Grid::new(format!("agent-grid-{i}"))
                .num_columns(2)
                .spacing([20.0, 6.0])
                .show(ui, |ui| {
                    ui.label(t.name);
                    ui.label(egui::RichText::new(&aw.a.name).monospace());
                    ui.end_row();
                    ui.label(t.agent_type);
                    let mut sub = aw.a.agent_type.as_deref() == Some("subagent");
                    if ui.checkbox(&mut sub, t.subagent).changed() {
                        aw.a.agent_type = sub.then_some("subagent".to_string());
                    }
                    ui.end_row();
                    option_field(ui, t.active_model, &mut aw.a.active_model);
                    option_field(ui, t.system_prompt_id, &mut aw.a.system_prompt_id);
                    label_field_multiline(ui, t.enabled_tools, &mut aw.enabled_buf, t);
                    label_field_multiline(ui, t.disabled_tools, &mut aw.disabled_buf, t);
                });
            ui.horizontal(|ui| {
                if ui.button(format!("💾 {}", t.save)).clicked() {
                    let mut a = aw.synced();
                    a.name = a.name.trim().to_string();
                    if a.name.is_empty() {
                        a.name = aw.original_name.clone();
                    }
                    if a.name != aw.original_name {
                        let _ = AgentFile::delete(&aw.original_name);
                    }
                    if a.save().is_ok() {
                        aw.a = a;
                        aw.original_name = aw.a.name.clone();
                    }
                }
                if ui.button(format!("🗑 {}", t.delete)).clicked() {
                    delete_agent = true;
                }
            });
        });
        if select_agent {
            pane.agent_sel = Some(i);
        }
        if delete_agent {
            let _ = AgentFile::delete(&aw.original_name);
            remove_agent = Some(i);
        }
    }
    if let Some(i) = remove_agent {
        pane.agents.remove(i);
        if pane.agent_sel == Some(i) {
            pane.agent_sel = None;
        }
    }
}

// -- Prompts ------------------------------------------------------------------------------

fn prompts(app: &mut MistraldeckApp, ui: &mut Ui, t: &Str) {
    let pane = &mut app.settings_pane;

    ui.label(egui::RichText::new("~/.vibe/prompts/").monospace().weak());
    ui.add_space(6.0);
    if ui.button(format!("➕ {}", t.add)).clicked() {
        let mut n = 1;
        while pane
            .prompts
            .iter()
            .any(|pw| pw.p.name == format!("prompt-{n}"))
        {
            n += 1;
        }
        pane.prompts.push(PromptWorking {
            p: PromptFile {
                name: format!("prompt-{n}"),
                content: String::new(),
            },
            original_name: format!("prompt-{n}"),
        });
    }

    let mut select = None;
    for i in 0..pane.prompts.len() {
        let pw = &mut pane.prompts[i];
        if ui
            .selectable_label(pane.prompt_sel == Some(i), format!("📝 {}", pw.p.name))
            .clicked()
        {
            select = Some(i);
        }
    }
    if let Some(i) = select {
        pane.prompt_sel = Some(i);
    }

    if let Some(i) = pane.prompt_sel {
        if let Some(pw) = pane.prompts.get_mut(i) {
            egui::Grid::new("prompt-edit")
                .num_columns(2)
                .spacing([20.0, 6.0])
                .show(ui, |ui| {
                    ui.label(t.name);
                    if ui
                        .add(egui::TextEdit::singleline(&mut pw.p.name).desired_width(240.0))
                        .changed()
                    {
                        pw.p.name = pw.p.name.trim().to_string();
                    }
                    ui.end_row();
                });
            ui.add_space(4.0);
            ui.label(t.content);
            ui.add(
                egui::TextEdit::multiline(&mut pw.p.content)
                    .desired_width(f32::INFINITY)
                    .desired_rows(10)
                    .font(egui::TextStyle::Monospace),
            );
            let mut prompt_deleted = false;
            ui.horizontal(|ui| {
                if ui.button(format!("💾 {}", t.save)).clicked() {
                    if pw.p.name != pw.original_name {
                        let _ = PromptFile::delete(&pw.original_name);
                    }
                    if pw.p.name.trim().is_empty() {
                        pw.p.name = pw.original_name.clone();
                    }
                    if pw.p.save().is_ok() {
                        pw.original_name = pw.p.name.clone();
                    }
                }
                if ui.button(format!("🗑 {}", t.delete)).clicked() {
                    let _ = PromptFile::delete(&pw.original_name);
                    prompt_deleted = true;
                }
            });
            if prompt_deleted {
                pane.prompts.remove(i);
                pane.prompt_sel = None;
            }
        }
    }
}

// -- Raw config.toml -------------------------------------------------------------------------

fn raw(app: &mut MistraldeckApp, ui: &mut Ui, t: &Str) {
    ui.label(
        egui::RichText::new("~/.vibe/config.toml")
            .monospace()
            .strong(),
    );
    ui.add_space(4.0);
    let toml_str = app
        .settings_pane
        .cfg
        .as_ref()
        .map(|cfg| toml::to_string_pretty(&cfg.table).unwrap_or_default())
        .unwrap_or_default();
    egui::ScrollArea::vertical()
        .max_height(320.0)
        .show(ui, |ui| {
            ui.style_mut().override_text_style = Some(egui::TextStyle::Monospace);
            ui.label(&toml_str);
        });

    ui.add_space(12.0);
    ui.separator();
    ui.label(egui::RichText::new(t.settings_project_config).strong());
    ui.add_space(4.0);
    match mistraldeck_vibe::find_project_config(&app.current_workdir()) {
        Some(path) => {
            ui.label(
                egui::RichText::new(path.to_string_lossy().to_string())
                    .small()
                    .monospace()
                    .weak(),
            );
            if let Ok(content) = std::fs::read_to_string(&path) {
                egui::ScrollArea::vertical()
                    .max_height(260.0)
                    .show(ui, |ui| {
                        ui.style_mut().override_text_style = Some(egui::TextStyle::Monospace);
                        ui.label(&content);
                    });
            }
        }
        None => {
            ui.label(egui::RichText::new(t.settings_no_project_config).weak());
        }
    }
}

// -- field helpers -------------------------------------------------------------------------------

fn label_field(ui: &mut Ui, label: &str, s: &mut String) {
    ui.label(egui::RichText::new(label).weak());
    ui.add(egui::TextEdit::singleline(s).desired_width(320.0));
    ui.end_row();
}

fn label_field_multiline(ui: &mut Ui, label: &str, s: &mut String, t: &Str) {
    ui.label(egui::RichText::new(format!("{label} {}", t.one_per_line)).weak());
    ui.add(
        egui::TextEdit::multiline(s)
            .desired_width(320.0)
            .desired_rows(3),
    );
    ui.end_row();
}

fn multiline_field(ui: &mut Ui, label: &str, s: &mut String, t: &Str) {
    ui.label(egui::RichText::new(format!("{label} {}", t.one_per_line)).weak());
    ui.add(
        egui::TextEdit::multiline(s)
            .desired_width(320.0)
            .desired_rows(3),
    );
    ui.end_row();
}

fn kv_field(ui: &mut Ui, label: &str, buf: &mut String, t: &Str) {
    ui.label(egui::RichText::new(format!("{label} (KEY=VALUE {})", t.one_per_line)).weak());
    ui.add(
        egui::TextEdit::multiline(buf)
            .desired_width(320.0)
            .desired_rows(3),
    );
    ui.end_row();
}

fn option_field(ui: &mut Ui, label: &str, opt: &mut Option<String>) {
    ui.label(egui::RichText::new(label).weak());
    let mut s = opt.clone().unwrap_or_default();
    if ui
        .add(egui::TextEdit::singleline(&mut s).desired_width(320.0))
        .changed()
    {
        *opt = (!s.is_empty()).then_some(s);
    }
    ui.end_row();
}

fn option_field_inline(ui: &mut Ui, label: &str, opt: &mut Option<String>) {
    let mut s = opt.clone().unwrap_or_default();
    let changed = ui
        .add(
            egui::TextEdit::singleline(&mut s)
                .desired_width(120.0)
                .hint_text(label),
        )
        .changed();
    if changed {
        *opt = (!s.is_empty()).then_some(s);
    }
}

fn checkbox_field(ui: &mut Ui, checked: &mut bool, label: &str) {
    ui.checkbox(checked, label);
    ui.end_row();
}
