//! Settings view: General (language / theme / sidebar limit), ACP (paths +
//! status), Remote control (login + port + enabled), and the Vibe CLI
//! configuration tabs: Providers, Models, MCP, Tools, Skills, Agents,
//! Prompts and raw TOML editing. Data flows through token-gated server
//! functions in `vibeconfig.rs`; same CSS classes as the Nexus settings.

use dioxus::prelude::*;

use crate::components::code_editor::CodeEditor;

use crate::i18n::tr;
use crate::state::{lang, save_prefs, token, ClientState};
use crate::vibeconfig::{FileCard, VibeConfigData};

fn loc(lang: &str, pl: &str, en: &str) -> String {
    if lang == "pl" {
        pl.to_string()
    } else {
        en.to_string()
    }
}

/// Per-connection remote CLI state in the Settings Updates section (Nexus
/// remoteCli map: "checking" sentinel + last check result).
#[derive(Clone, PartialEq)]
struct RemCliRow {
    checking: bool,
    info: Option<mistraldeck_core::snapshot::CliUpdateInfo>,
}

/// Editable draft of one SSH remote connection (Settings editor).
#[derive(Clone)]
struct SshDraft {
    id: String,
    name: String,
    ssh_cmd: String,
    enabled: bool,
    /// One location per line.
    locations: String,
    install_command: String,
}

impl SshDraft {
    fn from_conn(c: &mistraldeck_core::snapshot::RemoteConnection) -> Self {
        Self {
            id: c.id.clone(),
            name: c.name.clone(),
            ssh_cmd: c.ssh_cmd.clone(),
            enabled: c.enabled,
            locations: c.locations.join("\n"),
            install_command: c.install_command.clone(),
        }
    }

    fn new_blank() -> Self {
        Self {
            id: format!("conn-{}", mistraldeck_core::types::now_ms()),
            name: String::new(),
            ssh_cmd: String::new(),
            enabled: true,
            locations: String::new(),
            install_command: String::new(),
        }
    }

    fn to_conn(&self) -> mistraldeck_core::snapshot::RemoteConnection {
        mistraldeck_core::snapshot::RemoteConnection {
            id: self.id.clone(),
            name: self.name.trim().to_string(),
            enabled: self.enabled,
            ssh_cmd: self.ssh_cmd.trim().to_string(),
            locations: self
                .locations
                .lines()
                .map(str::trim)
                .filter(|l| !l.is_empty())
                .map(str::to_string)
                .collect(),
            install_command: self.install_command.trim().to_string(),
        }
    }
}

/// Effective target of the CLI-scoped settings tabs (Nexus scope picker):
/// agent (local or 📍 SSH connection) + target (global or project cwd).
#[derive(Clone)]
struct ScopeSel {
    scope: String,
    cwd: String,
    conn: Option<mistraldeck_core::snapshot::RemoteConnection>,
}

async fn reload_cfg(
    state: Signal<ClientState>,
    mut cfg: Signal<Option<VibeConfigData>>,
    mut err: Signal<String>,
    sc: ScopeSel,
) {
    match crate::vibeconfig::api_vibe_config(
        token(&state),
        sc.scope.clone(),
        sc.cwd.clone(),
        sc.conn.clone(),
    )
    .await
    {
        Ok(d) => {
            err.set(String::new());
            cfg.set(Some(d));
        }
        Err(e) => err.set(e.to_string()),
    }
}

async fn reload_files(
    state: Signal<ClientState>,
    kind: String,
    sc: ScopeSel,
    mut files: Signal<Vec<FileCard>>,
) {
    match crate::vibeconfig::api_list_vibe_files(
        token(&state),
        sc.scope.clone(),
        sc.cwd.clone(),
        sc.conn.clone(),
        kind,
    )
    .await
    {
        Ok(f) => files.set(f),
        Err(_) => files.set(Vec::new()),
    }
}

/// A generic config row: title + meta + optional trailing element.
fn cfg_row(key: String, title: String, meta: String, trail: Option<Element>) -> Element {
    match trail {
        Some(t) => rsx! {
            div { key: "{key}", class: "session-row",
                div { class: "session-row-main",
                    div { class: "session-row-title", "{title}" }
                    div { class: "session-row-meta", "{meta}" }
                }
                {t}
            }
        },
        None => rsx! {
            div { key: "{key}", class: "session-row",
                div { class: "session-row-main",
                    div { class: "session-row-title", "{title}" }
                    div { class: "session-row-meta", "{meta}" }
                }
            }
        },
    }
}

fn del_btn(mut on_click: impl FnMut() + 'static) -> Element {
    rsx! {
        button {
            class: "refresh-btn",
            style: "margin-right:6px;",
            onclick: move |_| on_click(),
            "🗑"
        }
    }
}

/// Shared agents/prompts/skills file manager: list + editor + create/delete.
fn files_panel(
    lang: String,
    state: Signal<ClientState>,
    kind: &'static str,
    files: Signal<Vec<FileCard>>,
    sel_path: Signal<String>,
    mut editor: Signal<String>,
    mut new_name: Signal<String>,
    mut saved: Signal<String>,
    sc: ScopeSel,
) -> Element {
    let list = files.read().clone();
    let sel = sel_path.read().clone();
    let rows: Vec<Element> = list
        .iter()
        .enumerate()
        .map(|(i, f)| {
            let cls = if f.path == *sel_path.read() {
                "session-row".to_string()
            } else {
                "session-row".to_string()
            };
            let _ = cls;
            let state2 = state;
            let files2 = files;
            let mut sel_path2 = sel_path;
            let mut editor2 = editor;
            let sc2 = sc.clone();
            let path = f.path.clone();
            let name = f.display_name.clone();
            let meta = f.meta.clone();
            let content = f.content.clone();
            let path_del = path.clone();
            let del = del_btn(move || {
                let p = path_del.clone();
                let sc2 = sc2.clone();
                spawn(async move {
                    let _ = crate::vibeconfig::api_delete_vibe_file(
                        token(&state2),
                        sc2.scope.clone(),
                        sc2.cwd.clone(),
                        sc2.conn.clone(),
                        p.clone(),
                    )
                    .await;
                    sel_path2.set(String::new());
                    editor2.set(String::new());
                    reload_files(state2, kind.to_string(), sc2, files2).await;
                    saved.set(String::new());
                });
            });
            rsx! {
                div { key: "{i}", class: "session-row",
                    onclick: move |_| {
                        sel_path2.set(path.clone());
                        editor2.set(content.clone());
                    },
                    div { class: "session-row-main",
                        div { class: "session-row-title", "{name}" }
                        if !meta.is_empty() {
                            div { class: "session-row-meta", "{meta}" }
                        }
                    }
                    {del}
                }
            }
        })
        .collect();

    let template = match kind {
        "agents" => "active_model = \"mistral-medium-3.5\"\n".to_string(),
        _ => "# new\n".to_string(),
    };

    rsx! {
        div { class: "settings-section",
            div { class: "settings-section-header",
                div { class: "settings-section-title", "{tr(&lang, kind)}" }
            }
            div { class: "session-rows",
                if rows.is_empty() {
                    div { class: "empty-text", "{tr(&lang, \"noData\")}" }
                }
                {rows.iter().cloned()}
            }
            if !sel.is_empty() {
                div { class: "field",
                    div { class: "field-label", "{sel}" }
                    textarea {
                        class: "field-input",
                        style: "min-height:220px;font-family:monospace;font-size:12px;white-space:pre;",
                        value: "{editor.read()}",
                        oninput: move |e| editor.set(e.value()),
                    }
                    button {
                        class: "save-btn",
                        style: "margin-top:8px;",
                        onclick: {
                            let state = state.clone();
                            let sel = sel.clone();
                            let editor = editor.clone();
                            let files = files.clone();
                            let saved = saved.clone();
                            let sc_oc = sc.clone();
                            move |_| {
                                let state = state.clone();
                                let sel = sel.clone();
                                let editor = editor.clone();
                                let files = files.clone();
                                let mut saved = saved.clone();
                                let sc = sc_oc.clone();
                                async move {
                                    let res = crate::vibeconfig::api_save_vibe_file(
                                        token(&state), sc.scope.clone(), sc.cwd.clone(), sc.conn.clone(), sel.clone(), editor.read().clone(),
                                    )
                                    .await;
                                    match res {
                                        Ok(()) => {
                                            saved.set(String::new());
                                            reload_files(state, kind.to_string(), sc, files).await;
                                        }
                                        Err(e) => saved.set(e.to_string()),
                                    }
                                }
                            }
                        },
                        "{tr(&lang, \"saveLabel\")}"
                    }
                    if !saved.read().is_empty() {
                        span { class: "field-hint", style: "margin-left:10px;", "{saved.read()}" }
                    }
                }
            }
            div { class: "field-row",
                input {
                    class: "field-input",
                    style: "flex:1;",
                    r#type: "text",
                    placeholder: "{tr(&lang, \"name\")}",
                    value: "{new_name.read()}",
                    oninput: move |e| new_name.set(e.value()),
                }
                button {
                    class: "save-btn",
                    onclick: {
                        let state = state.clone();
                        let new_name = new_name.clone();
                        let files = files.clone();
                        let saved = saved.clone();
                        let lang_oc = lang.clone();
                        let template_oc = template.clone();
                        let sc_oc = sc.clone();
                        move |_| {
                            let state = state.clone();
                            let mut new_name = new_name.clone();
                            let files = files.clone();
                            let mut saved = saved.clone();
                            let lang = lang_oc.clone();
                            let template = template_oc.clone();
                            let sc = sc_oc.clone();
                            async move {
                                let name = new_name.read().trim().to_string();
                                if name.is_empty() {
                                    saved.set(loc(&lang, "Podaj nazwę.", "Enter a name."));
                                    return;
                                }
                                let path = match kind {
                                    "agents" => format!("agents/{name}.toml"),
                                    "prompts" => format!("prompts/{name}.md"),
                                    _ => format!("skills/{name}/SKILL.md"),
                                };
                                let res = crate::vibeconfig::api_save_vibe_file(
                                        token(&state), sc.scope.clone(), sc.cwd.clone(), sc.conn.clone(), path.clone(), template.clone(),
                                    )
                                    .await;
                                match res {
                                    Ok(()) => {
                                        new_name.set(String::new());
                                        saved.set(String::new());
                                        reload_files(state, kind.to_string(), sc, files).await;
                                    }
                                    Err(e) => saved.set(e.to_string()),
                                }
                            }
                        }
                    },
                    "{tr(&lang, \"add\")}"
                }
            }
        }
    }
}

pub fn SettingsView() -> Element {
    let state = use_context::<Signal<ClientState>>();
    let mut tab = use_signal(|| "general".to_string());
    let saved = use_signal(String::new);

    let s = state.read();
    let lang = lang(&state);
    let language = s.snapshot.config.language.clone();
    let theme = s.snapshot.config.theme.clone();
    let dock_open = s.snapshot.config.dock_open;
    let limit = s.snapshot.config.sidebar_sessions_limit.unwrap_or(0);
    let acp_path = s.snapshot.config.acp_path.clone();
    let wd = s.snapshot.config.working_directory.clone();
    let remote_enabled = s.snapshot.config.remote_enabled;
    let auth_configured = s.snapshot.config.auth_configured;
    let remote_port = s.snapshot.config.remote_port;
    let ssh_conns = s.snapshot.config.remote_connections.clone();
    let serve_url = format!("https://<host>:{remote_port}");
    let status = s.snapshot.status.clone();
    let session_cwd = s
        .snapshot
        .current
        .and_then(|i| s.snapshot.sessions.get(i).map(|m| m.working_directory.clone()))
        .unwrap_or_default();
    drop(s);

    // Local form state for the remote-control tab.
    let mut r_user = use_signal(String::new);
    let mut r_pass = use_signal(String::new);
    let mut r_port = use_signal(move || remote_port.to_string());
    let mut r_enable = use_signal(move || remote_enabled || auth_configured);

    // SSH remote-connections editor state (remote tab, second section).
    let ssh_draft = use_signal(|| Option::<SshDraft>::None);
    let ssh_msg = use_signal(String::new);
    let ssh_busy = use_signal(|| false);

    // Element 6: Updates section - per-remote-connection CLI state (the
    // app + local CLI results live in ClientState, shared with the banner).
    let upd_rem = use_signal(Vec::<(String, RemCliRow)>::new);

    // Vibe CLI configuration state (loaded once, refetched after edits).
    let cfg = use_signal(|| Option::<VibeConfigData>::None);
    let cfg_err = use_signal(String::new);
    let mut tools_en = use_signal(String::new);
    let mut tools_dis = use_signal(String::new);
    let mut skills_en = use_signal(String::new);
    let mut skills_dis = use_signal(String::new);
    let raw_toml = use_signal(String::new);
    let list_saved = use_signal(String::new);

    // file-manager state per kind
    let files = use_signal(|| Vec::<FileCard>::new());
    let mut files_kind = use_signal(|| "agents".to_string());
    let sel_path = use_signal(String::new);
    let editor = use_signal(String::new);
    let new_name = use_signal(String::new);
    let file_saved = use_signal(String::new);
    // Scope picker state (element 7; Nexus scopeAgent/scopeTarget): the
    // agent select ("local" or a remote connection id) and the target
    // ("global" or a project cwd).
    let scope_agent = use_signal(|| "local".to_string());
    let scope_target = use_signal(|| "global".to_string());
    let scope_key = use_signal(String::new);

    // add-form state
    let mut p_name = use_signal(String::new);
    let mut p_base = use_signal(String::new);
    let mut p_key = use_signal(String::new);
    let mut p_backend = use_signal(String::new);
    let mut m_name = use_signal(String::new);
    let mut m_provider = use_signal(String::new);
    let mut m_alias = use_signal(String::new);
    let mut c_name = use_signal(String::new);
    let mut c_transport = use_signal(|| "stdio".to_string());
    let mut c_url = use_signal(String::new);
    let mut c_command = use_signal(String::new);
    let mut c_args = use_signal(String::new);
    let c_api_key_env = use_signal(String::new);

    // Effective scope from the agent + target selects (Nexus SettingsView):
    // local global / local project / remote global / remote project. All
    // CLI-scoped operations flow through `sc`; changing either select
    // reloads the config and clears stale detail panels (v0.13.3).
    let agent = scope_agent.read().clone();
    let target = scope_target.read().clone();
    let conn_sel: Option<mistraldeck_core::snapshot::RemoteConnection> = if agent == "local" {
        None
    } else {
        ssh_conns.iter().find(|c| c.id == agent).cloned()
    };
    let eff_scope = if target == "global" {
        "global".to_string()
    } else {
        "project".to_string()
    };
    let eff_cwd = if target == "global" {
        String::new()
    } else {
        target.clone()
    };
    let sc = ScopeSel {
        scope: eff_scope.clone(),
        cwd: eff_cwd.clone(),
        conn: conn_sel.clone(),
    };
    let scope_path = if eff_scope == "project" {
        match &conn_sel {
            Some(c) => format!("📍 {}: {}/.vibe", c.name, eff_cwd),
            None => format!("{}/.vibe", eff_cwd),
        }
    } else {
        match &conn_sel {
            Some(c) => format!("{}: ~/.vibe", c.name),
            None => "~/.vibe".to_string(),
        }
    };
    let sc_key = format!("{agent}|{target}");
    if *scope_key.read() != sc_key {
        let mut scope_key = scope_key.clone();
        scope_key.set(sc_key.clone());
        // Stale detail panel fix (v0.13.3): selections never survive a
        // scope switch.
        let mut sel_path = sel_path.clone();
        let mut editor = editor.clone();
        let mut new_name = new_name.clone();
        sel_path.set(String::new());
        editor.set(String::new());
        new_name.set(String::new());
        let mut files = files.clone();
        let mut files_kind = files_kind.clone();
        files.set(Vec::new());
        files_kind.set(String::new());
        let st = state.clone();
        let sc2 = sc.clone();
        let mut tools_en = tools_en.clone();
        let mut tools_dis = tools_dis.clone();
        let mut skills_en = skills_en.clone();
        let mut skills_dis = skills_dis.clone();
        let mut raw_toml = raw_toml.clone();
        let mut cfg = cfg.clone();
        let mut cfg_err = cfg_err.clone();
        spawn(async move {
            match crate::vibeconfig::api_vibe_config(
                token(&st),
                sc2.scope.clone(),
                sc2.cwd.clone(),
                sc2.conn.clone(),
            )
            .await
            {
                Ok(d) => {
                    tools_en.set(d.enabled_tools.join("\n"));
                    tools_dis.set(d.disabled_tools.join("\n"));
                    skills_en.set(d.enabled_skills.join("\n"));
                    skills_dis.set(d.disabled_skills.join("\n"));
                    raw_toml.set(d.raw_toml.clone());
                    cfg_err.set(String::new());
                    cfg.set(Some(d));
                }
                Err(e) => cfg_err.set(e.to_string()),
            }
        });
    }

    // lazily load the file list when a file tab is opened
    let cur_tab = tab.read().clone();
    if matches!(cur_tab.as_str(), "agents" | "prompts" | "skills") && *files_kind.read() != cur_tab
    {
        files_kind.set(cur_tab.clone());
        let st = state.clone();
        let sc2 = sc.clone();
        let mut sel_path = sel_path.clone();
        let mut editor = editor.clone();
        sel_path.set(String::new());
        editor.set(String::new());
        spawn(async move {
            reload_files(st, cur_tab, sc2, files).await;
        });
    }

    let cfgd = cfg.read().clone().unwrap_or_default();

    let tab_cls = |name: &str| {
        if *tab.read() == name {
            "scope-pill active".to_string()
        } else {
            "scope-pill".to_string()
        }
    };

    let err_el: Element = if cfg_err.read().is_empty() {
        VNode::empty()
    } else {
        rsx! { div { class: "git-error", "{cfg_err.read()}" } }
    };

    // Scope-picker options (Nexus SettingsView scopeAgent/scopeTarget): the
    // agent select offers "local" plus every enabled connection; the target
    // select offers "global" plus candidate project folders (session cwd +
    // engine workdir locally, configured locations remotely).
    let proj_opts: Vec<String> = if agent == "local" {
        let mut v: Vec<String> = Vec::new();
        if !session_cwd.is_empty() {
            v.push(session_cwd.clone());
        }
        if !wd.is_empty() && !v.contains(&wd) {
            v.push(wd.clone());
        }
        v
    } else {
        conn_sel
            .as_ref()
            .map(|c| {
                c.locations
                    .iter()
                    .map(|l| l.trim().to_string())
                    .filter(|l| !l.is_empty())
                    .collect::<Vec<String>>()
            })
            .unwrap_or_default()
    };
    let agent_opts: Vec<Element> = {
        let local_lbl = tr(&lang, "settingsScopeLocal");
        let mut v: Vec<Element> = vec![rsx! {
            option { value: "local", selected: agent == "local", "{local_lbl}" }
        }];
        for c in ssh_conns.iter().filter(|c| c.enabled) {
            let sel = c.id == agent;
            let lbl = format!("📍 {}", c.name);
            v.push(rsx! { option { value: "{c.id}", selected: sel, "{lbl}" } });
        }
        v
    };
    let target_opts: Vec<Element> = {
        let global_lbl = tr(&lang, "settingsScopeGlobal");
        let mut v: Vec<Element> = vec![rsx! {
            option { value: "global", selected: target == "global", "{global_lbl}" }
        }];
        for p in proj_opts.iter() {
            let sel = *p == target;
            v.push(rsx! { option { value: "{p}", selected: sel, "{p}" } });
        }
        v
    };
    // Only the CLI-scoped tabs show the picker (Nexus shows it next to the
    // title on those tabs).
    let cli_scoped = matches!(
        tab.read().as_str(),
        "agents" | "mcp" | "tools" | "prompts" | "skills" | "providers" | "models" | "raw"
    );

    let body_el = match tab.read().as_str() {
        "acp" => rsx! {
            div { class: "settings-section",
                div { class: "settings-section-header",
                    div { class: "settings-section-title", "ACP" }
                }
                div { class: "field",
                    div { class: "field-label", "vibe-acp" }
                    input { class: "field-input", readonly: true, value: "{acp_path}" }
                    div { class: "field-hint", "{loc(&lang, \"Zmiana ścieżki wymaga edycji pliku konfiguracyjnego i restartu.\", \"Changing the path requires editing the config file and a restart.\")}" }
                }
                div { class: "field",
                    div { class: "field-label", "{tr(&lang, \"workingDirectory\")}" }
                    input { class: "field-input", readonly: true, value: "{wd}" }
                }
                div { class: "field",
                    div { class: "field-label", "Status" }
                    div { class: "acp-status {status}",
                        span { class: "status-dot" }
                        span { class: "status-label", "{status}" }
                    }
                }
            }
        },
        "remote" => rsx! {
            div { class: "settings-section",
                div { class: "settings-section-header",
                    div { class: "settings-section-title", "{loc(&lang, \"Zdalne sterowanie\", \"Remote control\")}" }
                }
                div { class: "field-row",
                    label { class: "checkbox-label",
                        input {
                            r#type: "checkbox",
                            checked: *r_enable.read(),
                            onchange: move |e| r_enable.set(e.checked()),
                        }
                        "{loc(&lang, \"Włącz dostęp z przeglądarki\", \"Enable browser access\")}"
                    }
                }
                div { class: "field",
                    div { class: "field-label", "{loc(&lang, \"Login\", \"Username\")}" }
                    input {
                        class: "field-input",
                        r#type: "text",
                        autocomplete: "off",
                        value: "{*r_user.read()}",
                        oninput: move |e| r_user.set(e.value()),
                    }
                }
                div { class: "field",
                    div { class: "field-label", "{loc(&lang, \"Hasło\", \"Password\")}" }
                    input {
                        class: "field-input",
                        r#type: "password",
                        autocomplete: "new-password",
                        value: "{*r_pass.read()}",
                        oninput: move |e| r_pass.set(e.value()),
                    }
                    div { class: "field-hint",
                        "{loc(&lang, \"Hasło zapisywane jest wyłącznie jako skrót (sha256 + sól) — nigdy w jawnej postaci i nigdy nie jest wyświetlane.\", \"The password is stored only as a salted sha256 hash — never in plain text and never displayed.\")}"
                    }
                }
                div { class: "field",
                    div { class: "field-label", "{loc(&lang, \"Port\", \"Port\")}" }
                    input {
                        class: "field-input",
                        r#type: "number",
                        value: "{*r_port.read()}",
                        oninput: move |e| r_port.set(e.value()),
                    }
                    div { class: "field-hint",
                        "{loc(&lang, \"Zmiana portu/interfejsu wchodzi w życie po restarcie serwera Mistral Deck.\", \"Port/interface changes take effect after a Mistral Deck server restart.\")}"
                    }
                }
                div { class: "field",
                    button {
                        class: "save-btn",
                        onclick: {
                            let state = state.clone();
                            let saved = saved.clone();
                            let lang_c = lang.clone();
                            move |_| {
                                let state = state.clone();
                                let mut saved = saved.clone();
                                let lang = lang_c.clone();
                                async move {
                                    let user = r_user.read().clone();
                                    let pass = r_pass.read().clone();
                                    let enable = *r_enable.read();
                                    let port = r_port.read().parse::<u16>().unwrap_or(8080);
                                    if user.trim().is_empty() || pass.is_empty() {
                                        saved.set(loc(&lang, "Podaj login i hasło.", "Enter username and password."));
                                        return;
                                    }
                                    let res = crate::api::api_set_remote(
                                        token(&state), user, pass, port, enable,
                                    )
                                    .await;
                                    match res {
                                        Ok(()) => {
                                            saved.set(String::new());
                                            refresh_after_save(state).await;
                                        }
                                        Err(e) => {
                                            let lbl = loc(&lang, "Nie udało się zapisać", "Failed to save");
                                            saved.set(format!("{}: {e}", lbl));
                                        }
                                    }
                                }
                            }
                        },
                        "{tr(&lang, \"saveLabel\")}"
                    }
                    if !saved.read().is_empty() {
                        span { class: "field-hint", style: "margin-left:10px;", "{*saved.read()}" }
                    }
                }
                div { class: "field",
                    div { class: "field-hint",
                        if remote_enabled || auth_configured {
                            span { style: "display:block;margin-bottom:4px;",
                                "{loc(&lang, \"Dostęp zdalny aktywny —\", \"Remote access active —\")} {serve_url}"
                            }
                            span { style: "display:block;margin-bottom:4px;",
                                "{loc(&lang, \"Dostęp zdalny jest szyfrowany (HTTPS z certyfikatem self-signed); przeglądarka zapyta o zaufanie certyfikatu przy pierwszym połączeniu.\", \"Remote access is encrypted (HTTPS with a self-signed certificate); the browser asks to trust the certificate on first connect.\")}"
                            }
                            span { style: "display:block;",
                                "{loc(&lang, \"Dane logowania oraz port można edytować wyłącznie z lokalnego interfejsu.\", \"Credentials and the port can only be edited from the local interface.\")}"
                            }
                        } else {
                            "{loc(&lang, \"Dostęp zdalny wyłączony — interfejs lokalny działa bez hasła na 127.0.0.1.\", \"Remote access disabled — the local interface runs without a login on 127.0.0.1.\")}"
                        }
                    }
                }
            }
            div { class: "settings-section",
                div { class: "settings-section-header",
                    div { class: "settings-section-title", "{loc(&lang, \"Serwery zdalne (SSH)\", \"Remote servers (SSH)\")}" }
                    button {
                        class: "save-btn",
                        style: "margin-left:auto;",
                        onclick: {
                            let mut ssh_draft = ssh_draft.clone();
                            let mut ssh_msg = ssh_msg.clone();
                            move |_| {
                                ssh_msg.set(String::new());
                                ssh_draft.set(Some(SshDraft::new_blank()));
                            }
                        },
                        "＋ {loc(&lang, \"Dodaj serwer\", \"Add server\")}"
                    }
                }
                div { class: "field-hint",
                    "{loc(&lang, \"Połącz się przez SSH z innym serwerem i steruj agentem vibe-acp zdalnie — nowe sesje, historia CLI i podgląd postępu w grupie 📍 w pasku bocznym.\", \"Connect over SSH to another server and drive its vibe-acp agent remotely — new sessions, CLI history and live progress in the 📍 group in the sidebar.\")}"
                }
                {ssh_conn_rows(state, lang.clone(), &ssh_conns, ssh_draft.clone(), ssh_msg.clone()).iter().cloned()}
                if let Some(d) = ssh_draft.read().clone() {
                    {ssh_editor_el(state.clone(), lang.clone(), d, ssh_draft.clone(), ssh_msg.clone(), ssh_busy.clone())}
                }
                if !ssh_msg.read().is_empty() {
                    div { class: "field-hint", style: "white-space:pre-wrap;", "{ssh_msg.read().clone()}" }
                }
            }
        },
        "providers" => {
            let mut rows: Vec<Element> = cfgd
                .providers
                .iter()
                .enumerate()
                .map(|(i, p)| {
                    let state2 = state;
                    let cfg2 = cfg;
                    let cfg_err2 = cfg_err;
                    let sc2 = sc.clone();
                    let name = p.name.clone();
                    cfg_row(
                        i.to_string(),
                        p.name.clone(),
                        format!("{} · {}", p.api_base, if p.backend.is_empty() { "—".into() } else { p.backend.clone() }),
                        Some(del_btn(move || {
                            let name = name.clone();
                            let sc2 = sc2.clone();
                            spawn(async move {
                                let _ = crate::vibeconfig::api_remove_provider(
                                    token(&state2),
                                    name,
                                    sc2.scope.clone(),
                                    sc2.cwd.clone(),
                                    sc2.conn.clone(),
                                )
                                .await;
                                reload_cfg(state2, cfg2, cfg_err2, sc2).await;
                            });
                        })),
                    )
                })
                .collect();
            rows.push(rsx! {
                div { class: "field-row",
                    input { class: "field-input", style: "flex:1;", r#type: "text",
                        placeholder: "{tr(&lang, \"name\")}", value: "{p_name.read()}",
                        oninput: move |e| p_name.set(e.value()) }
                    input { class: "field-input", style: "flex:2;", r#type: "text",
                        placeholder: "base URL", value: "{p_base.read()}",
                        oninput: move |e| p_base.set(e.value()) }
                    input { class: "field-input", style: "flex:1;", r#type: "text",
                        placeholder: "{tr(&lang, \"apiKey\")} env", value: "{p_key.read()}",
                        oninput: move |e| p_key.set(e.value()) }
                    input { class: "field-input", style: "flex:1;", r#type: "text",
                        placeholder: "backend", value: "{p_backend.read()}",
                        oninput: move |e| p_backend.set(e.value()) }
                    button { class: "save-btn",
                        onclick: {
                            let state = state.clone();
                            let cfg = cfg.clone();
                            let cfg_err = cfg_err.clone();
                            let sc = sc.clone();
                            move |_| {
                                let state = state.clone();
                                let cfg = cfg.clone();
                                let cfg_err = cfg_err.clone();
                                let sc = sc.clone();
                                async move {
                                    let row = crate::vibeconfig::ProviderRow {
                                        name: p_name.read().trim().to_string(),
                                        api_base: p_base.read().trim().to_string(),
                                        api_key_env_var: p_key.read().trim().to_string(),
                                        backend: p_backend.read().trim().to_string(),
                                        api_style: String::new(),
                                    };
                                    if row.name.is_empty() || row.api_base.is_empty() {
                                        return;
                                    }
                                    let _ = crate::vibeconfig::api_upsert_provider(
                                        token(&state),
                                        row,
                                        sc.scope.clone(),
                                        sc.cwd.clone(),
                                        sc.conn.clone(),
                                    )
                                    .await;
                                    p_name.set(String::new());
                                    p_base.set(String::new());
                                    p_key.set(String::new());
                                    p_backend.set(String::new());
                                    reload_cfg(state, cfg, cfg_err, sc).await;
                                }
                            }
                        },
                        "{tr(&lang, \"add\")}" }
                }
            });
            rsx! {
                div { class: "settings-section",
                    div { class: "settings-section-header",
                        div { class: "settings-section-title", "{tr(&lang, \"providers\")}" }
                    }
                    div { class: "session-rows", {rows.iter().cloned()} }
                    {err_el}
                }
            }
        }
        "models" => {
            let active = cfgd.active_model.clone();
            let mut rows: Vec<Element> = cfgd
                .models
                .iter()
                .enumerate()
                .map(|(i, m)| {
                    let state2 = state;
                    let cfg2 = cfg;
                    let cfg_err2 = cfg_err;
                    let sc2 = sc.clone();
                    let name = m.name.clone();
                    let _alias = if m.alias.is_empty() { m.name.clone() } else { m.alias.clone() };
                    cfg_row(
                        i.to_string(),
                        m.name.clone(),
                        format!("{} · alias: {} · {}", m.provider, m.alias, if m.enabled { tr(&lang, "enabled") } else { "—" }),
                        Some(del_btn(move || {
                            let name = name.clone();
                            let sc2 = sc2.clone();
                            spawn(async move {
                                let _ = crate::vibeconfig::api_remove_model(
                                    token(&state2),
                                    name,
                                    sc2.scope.clone(),
                                    sc2.cwd.clone(),
                                    sc2.conn.clone(),
                                )
                                .await;
                                reload_cfg(state2, cfg2, cfg_err2, sc2).await;
                            });
                        })),
                    )
                })
                .collect();
            rows.push(rsx! {
                div { class: "field-row",
                    input { class: "field-input", style: "flex:1;", r#type: "text",
                        placeholder: "{tr(&lang, \"name\")}", value: "{m_name.read()}",
                        oninput: move |e| m_name.set(e.value()) }
                    input { class: "field-input", style: "flex:1;", r#type: "text",
                        placeholder: "{tr(&lang, \"providers\")}", value: "{m_provider.read()}",
                        oninput: move |e| m_provider.set(e.value()) }
                    input { class: "field-input", style: "flex:1;", r#type: "text",
                        placeholder: "alias", value: "{m_alias.read()}",
                        oninput: move |e| m_alias.set(e.value()) }
                    button { class: "save-btn",
                        onclick: {
                            let state = state.clone();
                            let cfg = cfg.clone();
                            let cfg_err = cfg_err.clone();
                            let sc = sc.clone();
                            move |_| {
                                let state = state.clone();
                                let cfg = cfg.clone();
                                let cfg_err = cfg_err.clone();
                                let sc = sc.clone();
                                async move {
                                    let row = crate::vibeconfig::ModelRow {
                                        name: m_name.read().trim().to_string(),
                                        provider: m_provider.read().trim().to_string(),
                                        alias: m_alias.read().trim().to_string(),
                                        enabled: true,
                                    };
                                    if row.name.is_empty() || row.provider.is_empty() {
                                        return;
                                    }
                                    let _ = crate::vibeconfig::api_upsert_model(
                                        token(&state),
                                        row,
                                        sc.scope.clone(),
                                        sc.cwd.clone(),
                                        sc.conn.clone(),
                                    )
                                    .await;
                                    m_name.set(String::new());
                                    m_provider.set(String::new());
                                    m_alias.set(String::new());
                                    reload_cfg(state, cfg, cfg_err, sc).await;
                                }
                            }
                        },
                        "{tr(&lang, \"add\")}" }
                }
            });
            let opts: Vec<Element> = cfgd
                .models
                .iter()
                .map(|m| {
                    let a = if m.alias.is_empty() { m.name.clone() } else { m.alias.clone() };
                    let sel = a == active;
                    let label = m.name.clone();
                    rsx! { option { value: "{a}", selected: sel, "{label}" } }
                })
                .collect();
            rsx! {
                div { class: "settings-section",
                    div { class: "settings-section-header",
                        div { class: "settings-section-title", "{tr(&lang, \"models\")}" }
                    }
                    div { class: "field",
                        div { class: "field-label", "{loc(&lang, \"Model aktywny\", \"Active model\")}" }
                        select {
                            class: "field-select",
                            onchange: {
                                let state = state.clone();
                                let cfg = cfg.clone();
                                let cfg_err = cfg_err.clone();
                                let sc = sc.clone();
                                move |e| {
                                    let state = state.clone();
                                    let cfg = cfg.clone();
                                    let cfg_err = cfg_err.clone();
                                    let sc = sc.clone();
                                    async move {
                                        let _ = crate::vibeconfig::api_set_active_model(
                                            token(&state),
                                            e.value(),
                                            sc.scope.clone(),
                                            sc.cwd.clone(),
                                            sc.conn.clone(),
                                        )
                                        .await;
                                        reload_cfg(state, cfg, cfg_err, sc).await;
                                    }
                                }
                            },
                            if cfgd.models.is_empty() {
                                option { value: "", selected: true, "—" }
                            }
                            {opts.iter().cloned()}
                        }
                    }
                    div { class: "session-rows", {rows.iter().cloned()} }
                    {err_el}
                }
            }
        }
        "mcp" => {
            let mut rows: Vec<Element> = cfgd
                .mcp
                .iter()
                .enumerate()
                .map(|(i, s)| {
                    let state2 = state;
                    let cfg2 = cfg;
                    let cfg_err2 = cfg_err;
                    let sc2 = sc.clone();
                    let name = s.name.clone();
                    cfg_row(
                        i.to_string(),
                        s.name.clone(),
                        format!("{} · {}", s.transport, if s.url.is_empty() { s.command.clone() } else { s.url.clone() }),
                        Some(del_btn(move || {
                            let name = name.clone();
                            let sc2 = sc2.clone();
                            spawn(async move {
                                let _ = crate::vibeconfig::api_remove_mcp(
                                    token(&state2),
                                    name,
                                    sc2.scope.clone(),
                                    sc2.cwd.clone(),
                                    sc2.conn.clone(),
                                )
                                .await;
                                reload_cfg(state2, cfg2, cfg_err2, sc2).await;
                            });
                        })),
                    )
                })
                .collect();
            rows.push(rsx! {
                div { class: "field-row",
                    input { class: "field-input", style: "flex:1;", r#type: "text",
                        placeholder: "{tr(&lang, \"name\")}", value: "{c_name.read()}",
                        oninput: move |e| c_name.set(e.value()) }
                    select { class: "field-select", style: "flex:1;",
                        onchange: move |e| c_transport.set(e.value()),
                        option { value: "stdio", selected: *c_transport.read() == "stdio", "stdio" }
                        option { value: "http", selected: *c_transport.read() == "http", "http" }
                        option { value: "streamable-http", selected: *c_transport.read() == "streamable-http", "streamable-http" }
                    }
                    input { class: "field-input", style: "flex:2;", r#type: "text",
                        placeholder: "URL", value: "{c_url.read()}",
                        oninput: move |e| c_url.set(e.value()) }
                    input { class: "field-input", style: "flex:1;", r#type: "text",
                        placeholder: "{tr(&lang, \"command\")}", value: "{c_command.read()}",
                        oninput: move |e| c_command.set(e.value()) }
                    input { class: "field-input", style: "flex:2;", r#type: "text",
                        placeholder: "args · {tr(&lang, \"apiKey\")} env", value: "{c_args.read()}",
                        oninput: move |e| c_args.set(e.value()) }
                    button { class: "save-btn",
                        onclick: {
                            let state = state.clone();
                            let cfg = cfg.clone();
                            let cfg_err = cfg_err.clone();
                            let sc = sc.clone();
                            move |_| {
                                let state = state.clone();
                                let cfg = cfg.clone();
                                let cfg_err = cfg_err.clone();
                                let sc = sc.clone();
                                async move {
                                    let name = c_name.read().trim().to_string();
                                    if name.is_empty() {
                                        return;
                                    }
                                    let row = crate::vibeconfig::McpRow {
                                        name,
                                        transport: c_transport.read().clone(),
                                        url: c_url.read().trim().to_string(),
                                        command: c_command.read().trim().to_string(),
                                        args: c_args.read().trim().to_string(),
                                    };
                                    let _ = crate::vibeconfig::api_upsert_mcp(
                                        token(&state),
                                        row,
                                        sc.scope.clone(),
                                        sc.cwd.clone(),
                                        sc.conn.clone(),
                                    )
                                    .await;
                                    c_name.set(String::new());
                                    c_url.set(String::new());
                                    c_command.set(String::new());
                                    c_args.set(String::new());
                                    reload_cfg(state, cfg, cfg_err, sc).await;
                                }
                            }
                        },
                        "{tr(&lang, \"add\")}" }
                }
            });
            let _ = &c_api_key_env;
            rsx! {
                div { class: "settings-section",
                    div { class: "settings-section-header",
                        div { class: "settings-section-title", "{tr(&lang, \"mcp\")}" }
                    }
                    div { class: "session-rows", {rows.iter().cloned()} }
                    {err_el}
                }
            }
        }
        "tools" => rsx! {
            div { class: "settings-section",
                div { class: "settings-section-header",
                    div { class: "settings-section-title", "{tr(&lang, \"tools\")}" }
                }
                div { class: "field",
                    div { class: "field-label", "{loc(&lang, \"Włączone narzędzia (jedno na linię)\", \"Enabled tools (one per line)\")}" }
                    textarea {
                        class: "field-input",
                        style: "min-height:120px;font-family:monospace;font-size:12px;white-space:pre;",
                        value: "{tools_en.read()}",
                        oninput: move |e| tools_en.set(e.value()),
                    }
                }
                div { class: "field",
                    div { class: "field-label", "{loc(&lang, \"Wyłączone narzędzia\", \"Disabled tools\")}" }
                    textarea {
                        class: "field-input",
                        style: "min-height:120px;font-family:monospace;font-size:12px;white-space:pre;",
                        value: "{tools_dis.read()}",
                        oninput: move |e| tools_dis.set(e.value()),
                    }
                }
                div { class: "field",
                    button {
                        class: "save-btn",
                        onclick: {
                            let state = state.clone();
                            let cfg = cfg.clone();
                            let cfg_err = cfg_err.clone();
                            let list_saved = list_saved.clone();
                            let lang_c = lang.clone();
                            let sc = sc.clone();
                            move |_| {
                                let state = state.clone();
                                let cfg = cfg.clone();
                                let cfg_err = cfg_err.clone();
                                let mut list_saved = list_saved.clone();
                                let lang = lang_c.clone();
                                let sc = sc.clone();
                                async move {
                                    let en = tools_en.read().split('\n').map(|s| s.to_string()).collect();
                                    let dis = tools_dis.read().split('\n').map(|s| s.to_string()).collect();
                                    let r1 = crate::vibeconfig::api_set_vibe_list(token(&state), "enabled_tools".into(), en, sc.scope.clone(), sc.cwd.clone(), sc.conn.clone()).await;
                                    let r2 = crate::vibeconfig::api_set_vibe_list(token(&state), "disabled_tools".into(), dis, sc.scope.clone(), sc.cwd.clone(), sc.conn.clone()).await;
                                    match (r1, r2) {
                                        (Ok(()), Ok(())) => list_saved.set(String::new()),
                                        _ => list_saved.set(loc(&lang, "Nie udało się zapisać.", "Failed to save.")),
                                    }
                                    reload_cfg(state, cfg, cfg_err, sc).await;
                                }
                            }
                        },
                        "{tr(&lang, \"saveLabel\")}"
                    }
                    if !list_saved.read().is_empty() {
                        span { class: "field-hint", style: "margin-left:10px;", "{list_saved.read()}" }
                    }
                }
                {err_el}
            }
        },
        "skills" => rsx! {
            div { class: "settings-section",
                div { class: "settings-section-header",
                    div { class: "settings-section-title", "{tr(&lang, \"skills\")}" }
                }
                div { class: "field",
                    div { class: "field-label", "{loc(&lang, \"Włączone (wzorce)\", \"Enabled (patterns)\")}" }
                    textarea {
                        class: "field-input",
                        style: "min-height:80px;font-family:monospace;font-size:12px;white-space:pre;",
                        value: "{skills_en.read()}",
                        oninput: move |e| skills_en.set(e.value()),
                    }
                }
                div { class: "field",
                    div { class: "field-label", "{loc(&lang, \"Wyłączone\", \"Disabled\")}" }
                    textarea {
                        class: "field-input",
                        style: "min-height:80px;font-family:monospace;font-size:12px;white-space:pre;",
                        value: "{skills_dis.read()}",
                        oninput: move |e| skills_dis.set(e.value()),
                    }
                }
                div { class: "field",
                    button {
                        class: "save-btn",
                        onclick: {
                            let state = state.clone();
                            let cfg = cfg.clone();
                            let cfg_err = cfg_err.clone();
                            let list_saved = list_saved.clone();
                            let lang_c = lang.clone();
                            let sc = sc.clone();
                            move |_| {
                                let state = state.clone();
                                let cfg = cfg.clone();
                                let cfg_err = cfg_err.clone();
                                let mut list_saved = list_saved.clone();
                                let lang = lang_c.clone();
                                let sc = sc.clone();
                                async move {
                                    let en = skills_en.read().split('\n').map(|s| s.to_string()).collect();
                                    let dis = skills_dis.read().split('\n').map(|s| s.to_string()).collect();
                                    let r1 = crate::vibeconfig::api_set_vibe_list(token(&state), "enabled_skills".into(), en, sc.scope.clone(), sc.cwd.clone(), sc.conn.clone()).await;
                                    let r2 = crate::vibeconfig::api_set_vibe_list(token(&state), "disabled_skills".into(), dis, sc.scope.clone(), sc.cwd.clone(), sc.conn.clone()).await;
                                    match (r1, r2) {
                                        (Ok(()), Ok(())) => list_saved.set(String::new()),
                                        _ => list_saved.set(loc(&lang, "Nie udało się zapisać.", "Failed to save.")),
                                    }
                                    reload_cfg(state, cfg, cfg_err, sc).await;
                                }
                            }
                        },
                        "{tr(&lang, \"saveLabel\")}"
                    }
                    if !list_saved.read().is_empty() {
                        span { class: "field-hint", style: "margin-left:10px;", "{list_saved.read()}" }
                    }
                }
                {files_panel(lang.clone(), state.clone(), "skills", files, sel_path, editor, new_name, file_saved, sc.clone())}
                {err_el}
            }
        },
        "agents" => files_panel(lang.clone(), state.clone(), "agents", files, sel_path, editor, new_name, file_saved, sc.clone()),
        "prompts" => files_panel(lang.clone(), state.clone(), "prompts", files, sel_path, editor, new_name, file_saved, sc.clone()),
        "raw" => rsx! {
            div { class: "settings-section",
                div { class: "settings-section-header",
                    div { class: "settings-section-title", "config.toml" }
                }
                div { class: "field",
                    CodeEditor {
                        code: raw_toml,
                        id: "raw".to_string(),
                    }
                    div { class: "field-hint", "{loc(&lang, \"Zapis waliduje TOML — błędny plik nie nadpisze konfiguracji.\", \"Saving validates the TOML — an invalid file will not overwrite the config.\")}" }
                    button {
                        class: "save-btn",
                        style: "margin-top:8px;",
                        onclick: {
                            let state = state.clone();
                            let cfg = cfg.clone();
                            let cfg_err = cfg_err.clone();
                            let list_saved = list_saved.clone();
                            let lang_c = lang.clone();
                            let sc = sc.clone();
                            move |_| {
                                let state = state.clone();
                                let cfg = cfg.clone();
                                let cfg_err = cfg_err.clone();
                                let mut list_saved = list_saved.clone();
                                let _lang = lang_c.clone();
                                let sc = sc.clone();
                                async move {
                                    match crate::vibeconfig::api_save_vibe_config_raw(
                                        token(&state), raw_toml.read().clone(),
                                        sc.scope.clone(), sc.cwd.clone(), sc.conn.clone(),
                                    )
                                    .await
                                    {
                                        Ok(()) => list_saved.set(String::new()),
                                        Err(e) => list_saved.set(e.to_string()),
                                    }
                                    reload_cfg(state, cfg, cfg_err, sc).await;
                                }
                            }
                        },
                        "{tr(&lang, \"saveLabel\")}"
                    }
                    if !list_saved.read().is_empty() {
                        span { class: "field-hint", style: "margin-left:10px;color:#f87171;", "{list_saved.read()}" }
                    }
                }
                {err_el}
            }
        },
        _ => {
            // Element 6: precomputed update values (app + local CLI results
            // are shared with the banner through ClientState).
            let upd = state.read();
            let app = upd.update_app.clone();
            let cli = upd.update_cli.clone();
            let busy = upd.update_busy;
            let upd_err = upd.update_error.clone();
            let prog = upd.update_progress.clone();
            drop(upd);
            let desktop = crate::state::is_desktop();
            let app_cur = app
                .as_ref()
                .map(|a| a.current_version.clone())
                .unwrap_or_else(|| "?".into());
            let app_latest = app
                .as_ref()
                .and_then(|a| a.latest_version.clone())
                .unwrap_or_else(|| "?".into());
            let app_avail = app.as_ref().map(|a| a.update_available).unwrap_or(false);
            let app_err = app.as_ref().and_then(|a| a.error.clone());
            let cli_cur = cli
                .as_ref()
                .and_then(|c| c.current_version.clone())
                .unwrap_or_else(|| "?".into());
            let cli_latest = cli
                .as_ref()
                .and_then(|c| c.latest_version.clone())
                .unwrap_or_else(|| "?".into());
            let cli_avail = cli.as_ref().map(|c| c.update_available).unwrap_or(false);
            let cli_err = cli.as_ref().and_then(|c| c.error.clone());
            let prog_text = prog.as_ref().map(|p| match p.phase.as_str() {
                "download" => format!(
                    "{} {}% ({:.1} / {} MB)",
                    loc(&lang, "Pobieranie", "Downloading"),
                    p.percent.unwrap_or(0),
                    p.received as f64 / 1048576.0,
                    p.total
                        .map(|t| format!("{:.1}", t as f64 / 1048576.0))
                        .unwrap_or_else(|| "?".into()),
                ),
                "install" => loc(&lang, "Instalowanie…", "Installing…"),
                _ => loc(&lang, "Restartowanie…", "Restarting…"),
            });
            let rem_rows: Vec<(
                mistraldeck_core::snapshot::RemoteConnection,
                String,
                bool,
                bool,
            )> = ssh_conns
                .iter()
                .filter(|c| c.enabled)
                .map(|c| {
                    let row = upd_rem
                        .read()
                        .iter()
                        .find(|(id, _)| id == &c.id)
                        .map(|(_, r)| r.clone())
                        .unwrap_or(RemCliRow { checking: false, info: None });
                    let status = if row.checking {
                        loc(&lang, "Sprawdzam…", "Checking…")
                    } else if let Some(info) = &row.info {
                        match &info.error {
                            Some(e) => format!("❌ {e}"),
                            None => format!(
                                "{}: {} · {}: {}",
                                loc(&lang, "Obecna", "Current"),
                                info.current_version
                                    .clone()
                                    .unwrap_or_else(|| "?".into()),
                                loc(&lang, "Najnowsza", "Latest"),
                                info.latest_version
                                    .clone()
                                    .unwrap_or_else(|| "?".into()),
                            ),
                        }
                    } else {
                        loc(&lang, "Nie sprawdzono", "Not checked")
                    };
                    let can_upgrade = !row.checking
                        && row.info.as_ref().map(|i| i.update_available).unwrap_or(false);
                    ((*c).clone(), status, row.checking, can_upgrade)
                })
                .collect();
            rsx! {
            div { class: "settings-section",
                div { class: "settings-section-header",
                    div { class: "settings-section-title", "{tr(&lang, \"general\")}" }
                }
                div { class: "field",
                    div { class: "field-label", "{tr(&lang, \"language\")}" }
                    select {
                        class: "field-select",
                        value: "{language}",
                        onchange: {
                            let state = state.clone();
                            let theme = theme.clone();
                            let dock_open = dock_open;
                            let limit = limit;
                            move |e| {
                                let state = state.clone();
                                let theme = theme.clone();
                                let dock_open = dock_open;
                                let limit = limit;
                                async move {
                                    save_prefs(state, e.value(), theme, dock_open, limit).await;
                                }
                            }
                        },
                        option { value: "auto", selected: language == "auto", "{tr(&lang, \"autoSystem\")}" }
                        option { value: "en", selected: language == "en", "English" }
                        option { value: "pl", selected: language == "pl", "Polski" }
                        option { value: "de", selected: language == "de", "Deutsch" }
                        option { value: "fr", selected: language == "fr", "Français" }
                        option { value: "es", selected: language == "es", "Español" }
                        option { value: "it", selected: language == "it", "Italiano" }
                    }
                }
                div { class: "field",
                    div { class: "field-label", "{tr(&lang, \"theme\")}" }
                    select {
                        class: "field-select",
                        value: "{theme}",
                        onchange: {
                            let state = state.clone();
                            let language = language.clone();
                            let dock_open = dock_open;
                            let limit = limit;
                            move |e| {
                                let state = state.clone();
                                let language = language.clone();
                                let dock_open = dock_open;
                                let limit = limit;
                                async move {
                                    save_prefs(state, language, e.value(), dock_open, limit).await;
                                }
                            }
                        },
                        option { value: "auto", selected: theme == "auto", "{tr(&lang, \"auto\")}" }
                        option { value: "dark", selected: theme == "dark", "{tr(&lang, \"dark\")}" }
                        option { value: "light", selected: theme == "light", "{tr(&lang, \"light\")}" }
                    }
                }
                div { class: "field",
                    div { class: "field-label", "{loc(&lang, \"Limit sesji w panelu\", \"Sidebar sessions limit\")}" }
                    input {
                        class: "field-input",
                        r#type: "number",
                        value: "{limit}",
                        onchange: {
                            let state = state.clone();
                            let language = language.clone();
                            let theme = theme.clone();
                            let dock_open = dock_open;
                            move |e| {
                                let state = state.clone();
                                let language = language.clone();
                                let theme = theme.clone();
                                let dock_open = dock_open;
                                async move {
                                    let n = e.value().parse::<u32>().unwrap_or(0);
                                    save_prefs(state, language, theme, dock_open, n).await;
                                }
                            }
                        },
                    }
                    div { class: "field-hint", "0 = {loc(&lang, \"bez limitu\", \"no limit\")}" }
                }

                // ---- Updates (element 6; Nexus SettingsView general tab) ----
                div { class: "settings-section-header",
                    div { class: "settings-section-title", "{loc(&lang, \"Aktualizacje\", \"Updates\")}" }
                }
                div { class: "updates-row",
                    div { class: "updates-info",
                        span { class: "updates-name", "{loc(&lang, \"Aplikacja\", \"App\")}" }
                        span { class: "updates-versions",
                            "{loc(&lang, \"Obecna\", \"Current\")}: {app_cur} · {loc(&lang, \"Najnowsza\", \"Latest\")}: {app_latest}"
                        }
                        if let Some(pt) = &prog_text {
                            span { class: "updates-progress", "{pt}" }
                        }
                    }
                    div { class: "updates-actions",
                        button {
                            class: "field-btn",
                            disabled: busy,
                            onclick: {
                                let state = state;
                                move |_| {
                                    let state = state;
                                    spawn(async move { crate::state::check_updates(state).await; });
                                }
                            },
                            "{loc(&lang, \"Sprawdź aktualizacje\", \"Check for updates\")}"
                        }
                        button {
                            class: "action-btn",
                            disabled: busy || !app_avail || !desktop,
                            onclick: {
                                let state = state;
                                move |_| {
                                    let state = state;
                                    spawn(async move { crate::state::start_app_update(state).await; });
                                }
                            },
                            "{loc(&lang, \"Aktualizuj\", \"Update now\")}"
                        }
                    }
                }
                if let Some(e) = &app_err {
                    div { class: "field-hint", "❌ {e}" }
                }
                div { class: "field-hint", "{loc(&lang, \"Aktualizacja aplikacji pobiera nową wersję, instaluje ją i uruchamia aplikację ponownie.\", \"The app update downloads the new version, installs it and relaunches the app.\")}" }
                div { class: "updates-row",
                    div { class: "updates-info",
                        span { class: "updates-name", "CLI (mistral-vibe)" }
                        span { class: "updates-versions",
                            "{loc(&lang, \"Obecna\", \"Current\")}: {cli_cur} · {loc(&lang, \"Najnowsza\", \"Latest\")}: {cli_latest}"
                        }
                    }
                    div { class: "updates-actions",
                        button {
                            class: "action-btn",
                            disabled: busy || !cli_avail,
                            onclick: {
                                let state = state;
                                move |_| {
                                    let state = state;
                                    spawn(async move { crate::state::upgrade_local_cli(state).await; });
                                }
                            },
                            "{loc(&lang, \"Aktualizuj CLI\", \"Upgrade CLI\")}"
                        }
                    }
                }
                if let Some(e) = &cli_err {
                    div { class: "field-hint", "❌ {e}" }
                }
                div { class: "field-hint", "{loc(&lang, \"Aktualizacja CLI działa przez uv (do 5 minut).\", \"The CLI upgrade runs through uv (up to to 5 minutes).\")}" }
                div { class: "remote-cli-updates",
                    div { class: "field-label", "{loc(&lang, \"Zdalne CLI\", \"Remote CLI\")}" }
                    if rem_rows.is_empty() {
                        div { class: "field-hint", "{loc(&lang, \"Brak włączonych połączeń zdalnych.\", \"No enabled remote connections.\")}" }
                    }
                    for (conn, status, checking, can_upgrade) in &rem_rows {
                        div { class: "updates-row",
                            div { class: "updates-info",
                                span { class: "updates-name", "{conn.name}" }
                                span { class: "updates-versions", "{status}" }
                            }
                            div { class: "updates-actions",
                                button {
                                    class: "field-btn",
                                    disabled: *checking || busy,
                                    onclick: {
                                        let state = state;
                                        let upd_rem = upd_rem;
                                        let conn = conn.clone();
                                        move |_| {
                                            let state = state;
                                            let upd_rem = upd_rem;
                                            let conn = conn.clone();
                                            spawn(async move {
                                                let mut upd_rem = upd_rem;
                                                let cid = conn.id.clone();
                                                {
                                                    let mut w = upd_rem.write();
                                                    w.retain(|(id, _)| id != &cid);
                                                    w.push((cid.clone(), RemCliRow { checking: true, info: None }));
                                                }
                                                let t = crate::state::token(&state);
                                                let res = crate::api::api_check_remote_cli_update(t, conn.clone()).await;
                                                let row = match res {
                                                    Ok(info) => RemCliRow { checking: false, info: Some(info) },
                                                    Err(e) => RemCliRow {
                                                        checking: false,
                                                        info: Some(mistraldeck_core::snapshot::CliUpdateInfo {
                                                            error: Some(e.to_string()),
                                                            ..Default::default()
                                                        }),
                                                    },
                                                };
                                                let mut w = upd_rem.write();
                                                w.retain(|(id, _)| id != &cid);
                                                w.push((cid, row));
                                            });
                                        }
                                    },
                                    "{loc(&lang, \"Sprawdź\", \"Check\")}"
                                }
                                button {
                                    class: "action-btn",
                                    disabled: !*can_upgrade || busy,
                                    onclick: {
                                        let state = state;
                                        let upd_rem = upd_rem;
                                        let conn = conn.clone();
                                        move |_| {
                                            let state = state;
                                            let upd_rem = upd_rem;
                                            let conn = conn.clone();
                                            spawn(async move {
                                                let mut upd_rem = upd_rem;
                                                let cid = conn.id.clone();
                                                {
                                                    let mut w = upd_rem.write();
                                                    w.retain(|(id, _)| id != &cid);
                                                    w.push((cid.clone(), RemCliRow { checking: true, info: None }));
                                                }
                                                let t = crate::state::token(&state);
                                                let res = crate::api::api_upgrade_remote_cli(t, conn.clone()).await;
                                                let err = match res {
                                                    Ok(r) => r.error,
                                                    Err(e) => Some(e.to_string()),
                                                };
                                                if let Some(e) = err {
                                                    let mut w = upd_rem.write();
                                                    w.retain(|(id, _)| id != &cid);
                                                    w.push((cid, RemCliRow {
                                                        checking: false,
                                                        info: Some(mistraldeck_core::snapshot::CliUpdateInfo {
                                                            error: Some(e),
                                                            ..Default::default()
                                                        }),
                                                    }));
                                                    return;
                                                }
                                                // Re-check after a successful upgrade (Nexus pattern).
                                                let t2 = crate::state::token(&state);
                                                let row = match crate::api::api_check_remote_cli_update(t2, conn.clone()).await {
                                                    Ok(info) => RemCliRow { checking: false, info: Some(info) },
                                                    Err(e) => RemCliRow {
                                                        checking: false,
                                                        info: Some(mistraldeck_core::snapshot::CliUpdateInfo {
                                                            error: Some(e.to_string()),
                                                            ..Default::default()
                                                        }),
                                                    },
                                                };
                                                let mut w = upd_rem.write();
                                                w.retain(|(id, _)| id != &cid);
                                                w.push((cid, row));
                                            });
                                        }
                                    },
                                    "{loc(&lang, \"Aktualizuj\", \"Update\")}"
                                }
                            }
                        }
                    }
                }
                if let Some(e) = &upd_err {
                    div { class: "field-hint", "❌ {e}" }
                }
            }
            }
        },
    };

    rsx! {
        div { class: "settings-view",
            div { class: "settings-header",
                div { class: "settings-title-row",
                    h2 { class: "settings-title", "{tr(&lang, \"settings\")}" }
                    if cli_scoped {
                        div { class: "scope-switch",
                            select {
                                class: "scope-select",
                                value: "{agent}",
                                onchange: move |e| {
                                    let mut scope_agent = scope_agent.clone();
                                    let mut scope_target = scope_target.clone();
                                    scope_agent.set(e.value());
                                    // Reset the target when the agent changes so a
                                    // stale remote project folder cannot leak in.
                                    scope_target.set("global".to_string());
                                },
                                {agent_opts.iter().cloned()}
                            }
                            select {
                                class: "scope-select",
                                value: "{target}",
                                onchange: move |e| {
                                    let mut scope_target = scope_target.clone();
                                    scope_target.set(e.value());
                                },
                                {target_opts.iter().cloned()}
                            }
                            span { class: "scope-path", "{scope_path}" }
                        }
                    }
                }
            }
            div { class: "settings-tabs scope-switch",
                button { class: "{tab_cls(\"general\")}", onclick: move |_| tab.set("general".into()), "{tr(&lang, \"general\")}" }
                button { class: "{tab_cls(\"acp\")}", onclick: move |_| tab.set("acp".into()), "ACP" }
                button { class: "{tab_cls(\"remote\")}", onclick: move |_| tab.set("remote".into()), "{loc(&lang, \"Zdalne\", \"Remote\")}" }
                button { class: "{tab_cls(\"providers\")}", onclick: move |_| tab.set("providers".into()), "{tr(&lang, \"providers\")}" }
                button { class: "{tab_cls(\"models\")}", onclick: move |_| tab.set("models".into()), "{tr(&lang, \"models\")}" }
                button { class: "{tab_cls(\"mcp\")}", onclick: move |_| tab.set("mcp".into()), "{tr(&lang, \"mcp\")}" }
                button { class: "{tab_cls(\"tools\")}", onclick: move |_| tab.set("tools".into()), "{tr(&lang, \"tools\")}" }
                button { class: "{tab_cls(\"skills\")}", onclick: move |_| tab.set("skills".into()), "{tr(&lang, \"skills\")}" }
                button { class: "{tab_cls(\"agents\")}", onclick: move |_| tab.set("agents".into()), "{tr(&lang, \"agents\")}" }
                button { class: "{tab_cls(\"prompts\")}", onclick: move |_| tab.set("prompts".into()), "{tr(&lang, \"prompts\")}" }
                button { class: "{tab_cls(\"raw\")}", onclick: move |_| tab.set("raw".into()), "TOML" }
            }
            div { class: "settings-content", {body_el} }
        }
    }
}

async fn refresh_after_save(state: Signal<ClientState>) {
    crate::state::refresh(state).await;
}

/// Saved SSH connection rows (name + command + edit).
fn ssh_conn_rows(
    _state: Signal<ClientState>,
    lang: String,
    conns: &[mistraldeck_core::snapshot::RemoteConnection],
    ssh_draft: Signal<Option<SshDraft>>,
    ssh_msg: Signal<String>,
) -> Vec<Element> {
    conns
        .iter()
        .map(|c| {
            let name = c.name.clone();
            let cmd = c.ssh_cmd.clone();
            let draft = SshDraft::from_conn(c);
            rsx! {
                div { class: "remote-conn-row",
                    span {
                        class: "remote-dot",
                        title: if c.enabled {
                            "{loc(&lang, \"Włączone\", \"Enabled\")}"
                        } else {
                            "{loc(&lang, \"Wyłączone\", \"Disabled\")}"
                        },
                        style: if c.enabled {
                            "background:#22c55e;margin-left:0;"
                        } else {
                            "background:#6b6b70;margin-left:0;"
                        },
                    }
                    span { class: "remote-conn-name", "{name}" }
                    span { class: "remote-conn-cmd", "{cmd}" }
                    button {
                        class: "save-btn",
                        style: "padding:2px 10px;",
                        onclick: {
                            let mut ssh_draft = ssh_draft.clone();
                            let mut ssh_msg = ssh_msg.clone();
                            let draft = draft.clone();
                            move |_| {
                                ssh_msg.set(String::new());
                                ssh_draft.set(Some(draft.clone()));
                            }
                        },
                        "{loc(&lang, \"Edytuj\", \"Edit\")}"
                    }
                }
            }
        })
        .collect()
}

/// The SSH connection draft editor: name, ssh command, locations,
/// install command + Test / Install / Save / Delete / Cancel.
fn ssh_editor_el(
    state: Signal<ClientState>,
    lang: String,
    d: SshDraft,
    ssh_draft: Signal<Option<SshDraft>>,
    ssh_msg: Signal<String>,
    ssh_busy: Signal<bool>,
) -> Element {
    rsx! {
        div { class: "field", style: "border:1px solid var(--border-subtle);border-radius:8px;padding:10px;",
            div { class: "field",
                div { class: "field-label", "{loc(&lang, \"Nazwa\", \"Name\")}" }
                input {
                    class: "field-input",
                    r#type: "text",
                    value: "{d.name}",
                    oninput: {
                        let mut ssh_draft = ssh_draft.clone();
                        move |e| {
                            let mut w = ssh_draft.write();
                            if let Some(x) = w.as_mut() { x.name = e.value(); }
                        }
                    },
                }
            }
            div { class: "field",
                div { class: "field-label", "SSH {loc(&lang, \"polecenie\", \"command\")}" }
                input {
                    class: "field-input",
                    r#type: "text",
                    style: "font-family:monospace;",
                    placeholder: "ssh user@host -p 2222",
                    value: "{d.ssh_cmd}",
                    oninput: {
                        let mut ssh_draft = ssh_draft.clone();
                        move |e| {
                            let mut w = ssh_draft.write();
                            if let Some(x) = w.as_mut() { x.ssh_cmd = e.value(); }
                        }
                    },
                }
            }
            div { class: "field-row",
                label { class: "checkbox-label",
                    input {
                        r#type: "checkbox",
                        checked: d.enabled,
                        onchange: {
                            let mut ssh_draft = ssh_draft.clone();
                            move |e| {
                                let mut w = ssh_draft.write();
                                if let Some(x) = w.as_mut() { x.enabled = e.checked(); }
                            }
                        },
                    }
                    "{loc(&lang, \"Włączony\", \"Enabled\")}"
                }
            }
            div { class: "field",
                div { class: "field-label", "{loc(&lang, \"Katalogi projektów (jeden na linię)\", \"Project directories (one per line)\")}" }
                textarea {
                    class: "field-input",
                    style: "min-height:60px;font-family:monospace;font-size:12px;",
                    value: "{d.locations}",
                    oninput: {
                        let mut ssh_draft = ssh_draft.clone();
                        move |e| {
                            let mut w = ssh_draft.write();
                            if let Some(x) = w.as_mut() { x.locations = e.value(); }
                        }
                    },
                }
            }
            div { class: "field",
                div { class: "field-label", "{loc(&lang, \"Własna komenda instalacji (opcjonalna)\", \"Custom install command (optional)\")}" }
                input {
                    class: "field-input",
                    r#type: "text",
                    style: "font-family:monospace;font-size:12px;",
                    value: "{d.install_command}",
                    oninput: {
                        let mut ssh_draft = ssh_draft.clone();
                        move |e| {
                            let mut w = ssh_draft.write();
                            if let Some(x) = w.as_mut() { x.install_command = e.value(); }
                        }
                    },
                }
                div { class: "field-hint",
                    "{loc(&lang, \"Instalator domyślnie używa uv (uv tool install mistral-vibe).\", \"The installer uses uv by default (uv tool install mistral-vibe).\")}"
                }
            }
            div { class: "field-row", style: "gap:8px;flex-wrap:wrap;",
                button {
                    class: "save-btn",
                    disabled: *ssh_busy.read(),
                    onclick: {
                        let state = state.clone();
                        let lang = lang.clone();
                        let d = d.clone();
                        let ssh_msg = ssh_msg.clone();
                        let ssh_busy = ssh_busy.clone();
                        move |_| {
                            let state = state.clone();
                            let lang = lang.clone();
                            let d = d.clone();
                            let mut ssh_msg = ssh_msg.clone();
                            let mut ssh_busy = ssh_busy.clone();
                            spawn(async move {
                                ssh_busy.set(true);
                                ssh_msg.set(loc(&lang, "Testuję...", "Testing..."));
                                let res = crate::api::api_test_remote(token(&state), d.to_conn()).await;
                                match res {
                                    Ok(r) => {
                                        if !r.reachable {
                                            ssh_msg.set(format!(
                                                "❌ {}",
                                                r.error.unwrap_or_else(|| "SSH connection failed".into())
                                            ));
                                        } else if r.acp_installed {
                                            ssh_msg.set(format!(
                                                "✅ {} — vibe-acp {} ({})",
                                                loc(&lang, "Połączenie OK", "Connection OK"),
                                                r.version.unwrap_or_default(),
                                                r.acp_path.unwrap_or_default(),
                                            ));
                                        } else {
                                            ssh_msg.set(format!(
                                                "⚠️ {} — {}",
                                                loc(&lang, "Połączono, ale vibe-acp nie jest zainstalowany", "Connected, but vibe-acp is not installed"),
                                                loc(&lang, "użyj Install", "use Install"),
                                            ));
                                        }
                                    }
                                    Err(e) => ssh_msg.set(format!("❌ {e}")),
                                }
                                ssh_busy.set(false);
                            });
                        }
                    },
                    "{loc(&lang, \"Test\", \"Test\")}"
                }
                button {
                    class: "save-btn",
                    disabled: *ssh_busy.read(),
                    onclick: {
                        let state = state.clone();
                        let lang = lang.clone();
                        let d = d.clone();
                        let ssh_msg = ssh_msg.clone();
                        let ssh_busy = ssh_busy.clone();
                        move |_| {
                            let state = state.clone();
                            let lang = lang.clone();
                            let d = d.clone();
                            let mut ssh_msg = ssh_msg.clone();
                            let mut ssh_busy = ssh_busy.clone();
                            spawn(async move {
                                ssh_busy.set(true);
                                ssh_msg.set(loc(&lang, "Instaluję (do 5 min)...", "Installing (up to 5 min)..."));
                                let res = crate::api::api_install_remote_acp(token(&state), d.to_conn()).await;
                                match res {
                                    Ok(r) if r.ok => {
                                        ssh_msg.set(format!(
                                            "✅ {}",
                                            loc(&lang, "Zainstalowano vibe-acp na zdalnym serwerze", "vibe-acp installed on the remote host"),
                                        ));
                                    }
                                    Ok(r) => {
                                        let err = r.error.unwrap_or_else(|| "install failed".into());
                                        ssh_msg.set(format!("❌ {err}\n{}", r.log));
                                    }
                                    Err(e) => ssh_msg.set(format!("❌ {e}")),
                                }
                                ssh_busy.set(false);
                            });
                        }
                    },
                    "{loc(&lang, \"Install\", \"Install\")}"
                }
                button {
                    class: "save-btn",
                    disabled: *ssh_busy.read(),
                    onclick: {
                        let state = state.clone();
                        let lang = lang.clone();
                        let d = d.clone();
                        let ssh_msg = ssh_msg.clone();
                        let ssh_busy = ssh_busy.clone();
                        move |_| {
                            let state = state.clone();
                            let lang = lang.clone();
                            let d = d.clone();
                            let mut ssh_msg = ssh_msg.clone();
                            let mut ssh_busy = ssh_busy.clone();
                            spawn(async move {
                                ssh_busy.set(true);
                                ssh_msg.set(loc(&lang, "Sprawdzam CLI...", "Checking CLI..."));
                                match crate::api::api_check_remote_cli_update(token(&state), d.to_conn()).await {
                                    Ok(r) => {
                                        if let Some(e) = r.error {
                                            ssh_msg.set(format!("❌ {e}"));
                                        } else if r.update_available {
                                            ssh_msg.set(format!(
                                                "⬆️ CLI {} → {} ({})",
                                                r.current_version.unwrap_or_default(),
                                                r.latest_version.unwrap_or_default(),
                                                loc(&lang, "dostępna aktualizacja", "update available")
                                            ));
                                        } else {
                                            ssh_msg.set(format!(
                                                "✅ CLI {} ({})",
                                                r.current_version.unwrap_or_default(),
                                                loc(&lang, "aktualna wersja", "up to date")
                                            ));
                                        }
                                    }
                                    Err(e) => ssh_msg.set(format!("❌ {e}")),
                                }
                                ssh_busy.set(false);
                            });
                        }
                    },
                    "{loc(&lang, \"Sprawdź CLI\", \"Check CLI\")}"
                }
                button {
                    class: "save-btn",
                    disabled: *ssh_busy.read(),
                    onclick: {
                        let state = state.clone();
                        let lang = lang.clone();
                        let d = d.clone();
                        let ssh_msg = ssh_msg.clone();
                        let ssh_busy = ssh_busy.clone();
                        move |_| {
                            let state = state.clone();
                            let lang = lang.clone();
                            let d = d.clone();
                            let mut ssh_msg = ssh_msg.clone();
                            let mut ssh_busy = ssh_busy.clone();
                            spawn(async move {
                                ssh_busy.set(true);
                                ssh_msg.set(loc(&lang, "Aktualizuję CLI (uv/pip3, do 5 min)...", "Upgrading CLI (uv/pip3, up to 5 min)..."));
                                match crate::api::api_upgrade_remote_cli(token(&state), d.to_conn()).await {
                                    Ok(r) => {
                                        if let Some(e) = r.error {
                                            ssh_msg.set(format!("❌ {e}"));
                                        } else {
                                            ssh_msg.set(format!("✅ CLI {}", r.version.unwrap_or_default()));
                                        }
                                    }
                                    Err(e) => ssh_msg.set(format!("❌ {e}")),
                                }
                                ssh_busy.set(false);
                            });
                        }
                    },
                    "{loc(&lang, \"Aktualizuj CLI\", \"Upgrade CLI\")}"
                }
                button {
                    class: "save-btn",
                    disabled: *ssh_busy.read(),
                    onclick: {
                        let state = state.clone();
                        let lang = lang.clone();
                        let d = d.clone();
                        let ssh_draft = ssh_draft.clone();
                        let ssh_msg = ssh_msg.clone();
                        let ssh_busy = ssh_busy.clone();
                        move |_| {
                            let state = state.clone();
                            let lang = lang.clone();
                            let d = d.clone();
                            let mut ssh_draft = ssh_draft.clone();
                            let mut ssh_msg = ssh_msg.clone();
                            let mut ssh_busy = ssh_busy.clone();
                            spawn(async move {
                                if d.name.trim().is_empty() || d.ssh_cmd.trim().is_empty() {
                                    ssh_msg.set(loc(&lang, "Podaj nazwę i polecenie SSH.", "Enter a name and the SSH command."));
                                    return;
                                }
                                ssh_busy.set(true);
                                let mut list = state.read().snapshot.config.remote_connections.clone();
                                let conn = d.to_conn();
                                match list.iter_mut().find(|c| c.id == conn.id) {
                                    Some(existing) => *existing = conn.clone(),
                                    None => list.push(conn.clone()),
                                }
                                match crate::api::api_save_remote_connections(token(&state), list).await {
                                    Ok(()) => {
                                        ssh_msg.set(String::new());
                                        ssh_draft.set(None);
                                        refresh_after_save(state).await;
                                    }
                                    Err(e) => ssh_msg.set(format!("❌ {e}")),
                                }
                                ssh_busy.set(false);
                            });
                        }
                    },
                    "{tr(&lang, \"saveLabel\")}"
                }
                button {
                    class: "save-btn",
                    style: "background:var(--panel-hover);",
                    disabled: *ssh_busy.read(),
                    onclick: {
                        let state = state.clone();
                        let d = d.clone();
                        let ssh_draft = ssh_draft.clone();
                        let ssh_msg = ssh_msg.clone();
                        move |_| {
                            let state = state.clone();
                            let d = d.clone();
                            let mut ssh_draft = ssh_draft.clone();
                            let mut ssh_msg = ssh_msg.clone();
                            spawn(async move {
                                let list = state.read().snapshot.config.remote_connections.clone();
                                let mut next: Vec<_> = list
                                    .into_iter()
                                    .filter(|c| c.id != d.id)
                                    .collect();
                                if next.is_empty() {
                                    next = Vec::new();
                                }
                                match crate::api::api_save_remote_connections(token(&state), next).await {
                                    Ok(()) => {
                                        ssh_msg.set(String::new());
                                        ssh_draft.set(None);
                                        refresh_after_save(state).await;
                                    }
                                    Err(e) => ssh_msg.set(format!("❌ {e}")),
                                }
                            });
                        }
                    },
                    "{loc(&lang, \"Usuń\", \"Delete\")}"
                }
                button {
                    class: "save-btn",
                    style: "background:var(--panel-hover);",
                    onclick: {
                        let mut ssh_draft = ssh_draft.clone();
                        let mut ssh_msg = ssh_msg.clone();
                        move |_| {
                            ssh_draft.set(None);
                            ssh_msg.set(String::new());
                        }
                    },
                    "{tr(&lang, \"cancelLabel\")}"
                }
            }
        }
    }
}
