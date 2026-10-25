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

async fn reload_cfg(
    state: Signal<ClientState>,
    mut cfg: Signal<Option<VibeConfigData>>,
    mut err: Signal<String>,
) {
    match crate::vibeconfig::api_vibe_config(token(&state)).await {
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
    scope: String,
    mut files: Signal<Vec<FileCard>>,
) {
    match crate::vibeconfig::api_list_vibe_files(token(&state), scope, kind).await {
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
    scope: Signal<String>,
) -> Element {
    let list = files.read().clone();
    let sel = sel_path.read().clone();
    let cur_scope = scope.read().clone();
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
            let scope2 = scope;
            let path = f.path.clone();
            let name = f.display_name.clone();
            let meta = f.meta.clone();
            let content = f.content.clone();
            let path_del = path.clone();
            let del = del_btn(move || {
                let p = path_del.clone();
                spawn(async move {
                    let _ = crate::vibeconfig::api_delete_vibe_file(
                        token(&state2), scope2.read().clone(), p.clone(),
                    )
                    .await;
                    sel_path2.set(String::new());
                    editor2.set(String::new());
                    reload_files(state2, kind.to_string(), scope2.read().clone(), files2).await;
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

    // Global/Project scope switch (Nexus scope pills)
    let g_cls = if cur_scope == "global" {
        "scope-pill active"
    } else {
        "scope-pill"
    };
    let p_cls = if cur_scope == "project" {
        "scope-pill active"
    } else {
        "scope-pill"
    };
    let switch_scope = move |sc: &'static str| {
        let state = state.clone();
        let files = files.clone();
        let mut sel_path = sel_path.clone();
        let mut editor = editor.clone();
        let mut scope = scope.clone();
        let kind = kind;
        move |_: dioxus::prelude::Event<dioxus::prelude::MouseData>| {
            if scope.read().clone() == sc {
                return;
            }
            scope.set(sc.to_string());
            sel_path.set(String::new());
            editor.set(String::new());
            let state = state.clone();
            let files = files.clone();
            spawn(async move {
                reload_files(state, kind.to_string(), sc.to_string(), files).await;
            });
        }
    };
    let on_global = switch_scope("global");
    let on_project = switch_scope("project");

    rsx! {
        div { class: "settings-section",
            div { class: "settings-section-header",
                div { class: "settings-section-title", "{tr(&lang, kind)}" }
                div { class: "scope-switch",
                    button { class: "{g_cls}", onclick: on_global, "{tr(&lang, \"scopeGlobal\")}" }
                    button { class: "{p_cls}", onclick: on_project, "{tr(&lang, \"scopeProject\")}" }
                }
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
                            let scope = scope.clone();
                            move |_| {
                                let state = state.clone();
                                let sel = sel.clone();
                                let editor = editor.clone();
                                let files = files.clone();
                                let mut saved = saved.clone();
                                let scope = scope.clone();
                                async move {
                                    let res = crate::vibeconfig::api_save_vibe_file(
                                        token(&state), scope.read().clone(), sel.clone(), editor.read().clone(),
                                    )
                                    .await;
                                    match res {
                                        Ok(()) => {
                                            saved.set(String::new());
                                            reload_files(state, kind.to_string(), scope.read().clone(), files).await;
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
                        let scope_oc = scope.clone();
                        move |_| {
                            let state = state.clone();
                            let mut new_name = new_name.clone();
                            let files = files.clone();
                            let mut saved = saved.clone();
                            let lang = lang_oc.clone();
                            let template = template_oc.clone();
                            let scope = scope_oc.clone();
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
                                    token(&state), scope.read().clone(), path.clone(), template.clone(),
                                )
                                .await;
                                match res {
                                    Ok(()) => {
                                        new_name.set(String::new());
                                        saved.set(String::new());
                                        reload_files(state, kind.to_string(), scope.read().clone(), files).await;
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
    let serve_url = format!("https://<host>:{remote_port}");
    let status = s.snapshot.status.clone();
    drop(s);

    // Local form state for the remote-control tab.
    let mut r_user = use_signal(String::new);
    let mut r_pass = use_signal(String::new);
    let mut r_port = use_signal(move || remote_port.to_string());
    let mut r_enable = use_signal(move || remote_enabled || auth_configured);

    // Vibe CLI configuration state (loaded once, refetched after edits).
    let mut cfg = use_signal(|| Option::<VibeConfigData>::None);
    let mut cfg_err = use_signal(String::new);
    let mut cfg_loaded = use_signal(|| false);
    let mut tools_en = use_signal(String::new);
    let mut tools_dis = use_signal(String::new);
    let mut skills_en = use_signal(String::new);
    let mut skills_dis = use_signal(String::new);
    let mut raw_toml = use_signal(String::new);
    let list_saved = use_signal(String::new);

    // file-manager state per kind
    let files = use_signal(|| Vec::<FileCard>::new());
    let mut files_kind = use_signal(|| "agents".to_string());
    let sel_path = use_signal(String::new);
    let editor = use_signal(String::new);
    let new_name = use_signal(String::new);
    let file_saved = use_signal(String::new);
    let scope = use_signal(|| "global".to_string());

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

    if !*cfg_loaded.read() {
        cfg_loaded.set(true);
        let st = state.clone();
        spawn(async move {
            match crate::vibeconfig::api_vibe_config(token(&st)).await {
                Ok(d) => {
                    tools_en.set(d.enabled_tools.join("\n"));
                    tools_dis.set(d.disabled_tools.join("\n"));
                    skills_en.set(d.enabled_skills.join("\n"));
                    skills_dis.set(d.disabled_skills.join("\n"));
                    raw_toml.set(d.raw_toml.clone());
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
        let sc = scope.read().clone();
        let mut sel_path = sel_path.clone();
        let mut editor = editor.clone();
        sel_path.set(String::new());
        editor.set(String::new());
        spawn(async move {
            reload_files(st, cur_tab, sc, files).await;
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
                        "{loc(&lang, \"Zmiana portu/interfejsu wchodzi w życie po restarcie serwera VibeDeck.\", \"Port/interface changes take effect after a VibeDeck server restart.\")}"
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
                    let name = p.name.clone();
                    cfg_row(
                        i.to_string(),
                        p.name.clone(),
                        format!("{} · {}", p.api_base, if p.backend.is_empty() { "—".into() } else { p.backend.clone() }),
                        Some(del_btn(move || {
                            let name = name.clone();
                            spawn(async move {
                                let _ = crate::vibeconfig::api_remove_provider(token(&state2), name).await;
                                reload_cfg(state2, cfg2, cfg_err2).await;
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
                            move |_| {
                                let state = state.clone();
                                let cfg = cfg.clone();
                                let cfg_err = cfg_err.clone();
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
                                    let _ = crate::vibeconfig::api_upsert_provider(token(&state), row).await;
                                    p_name.set(String::new());
                                    p_base.set(String::new());
                                    p_key.set(String::new());
                                    p_backend.set(String::new());
                                    reload_cfg(state, cfg, cfg_err).await;
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
                    let name = m.name.clone();
                    let _alias = if m.alias.is_empty() { m.name.clone() } else { m.alias.clone() };
                    cfg_row(
                        i.to_string(),
                        m.name.clone(),
                        format!("{} · alias: {} · {}", m.provider, m.alias, if m.enabled { tr(&lang, "enabled") } else { "—" }),
                        Some(del_btn(move || {
                            let name = name.clone();
                            spawn(async move {
                                let _ = crate::vibeconfig::api_remove_model(token(&state2), name).await;
                                reload_cfg(state2, cfg2, cfg_err2).await;
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
                            move |_| {
                                let state = state.clone();
                                let cfg = cfg.clone();
                                let cfg_err = cfg_err.clone();
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
                                    let _ = crate::vibeconfig::api_upsert_model(token(&state), row).await;
                                    m_name.set(String::new());
                                    m_provider.set(String::new());
                                    m_alias.set(String::new());
                                    reload_cfg(state, cfg, cfg_err).await;
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
                                move |e| {
                                    let state = state.clone();
                                    let cfg = cfg.clone();
                                    let cfg_err = cfg_err.clone();
                                    async move {
                                        let _ = crate::vibeconfig::api_set_active_model(token(&state), e.value()).await;
                                        reload_cfg(state, cfg, cfg_err).await;
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
                    let name = s.name.clone();
                    cfg_row(
                        i.to_string(),
                        s.name.clone(),
                        format!("{} · {}", s.transport, if s.url.is_empty() { s.command.clone() } else { s.url.clone() }),
                        Some(del_btn(move || {
                            let name = name.clone();
                            spawn(async move {
                                let _ = crate::vibeconfig::api_remove_mcp(token(&state2), name).await;
                                reload_cfg(state2, cfg2, cfg_err2).await;
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
                            move |_| {
                                let state = state.clone();
                                let cfg = cfg.clone();
                                let cfg_err = cfg_err.clone();
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
                                    let _ = crate::vibeconfig::api_upsert_mcp(token(&state), row).await;
                                    c_name.set(String::new());
                                    c_url.set(String::new());
                                    c_command.set(String::new());
                                    c_args.set(String::new());
                                    reload_cfg(state, cfg, cfg_err).await;
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
                            move |_| {
                                let state = state.clone();
                                let cfg = cfg.clone();
                                let cfg_err = cfg_err.clone();
                                let mut list_saved = list_saved.clone();
                                let lang = lang_c.clone();
                                async move {
                                    let en = tools_en.read().split('\n').map(|s| s.to_string()).collect();
                                    let dis = tools_dis.read().split('\n').map(|s| s.to_string()).collect();
                                    let r1 = crate::vibeconfig::api_set_vibe_list(token(&state), "enabled_tools".into(), en).await;
                                    let r2 = crate::vibeconfig::api_set_vibe_list(token(&state), "disabled_tools".into(), dis).await;
                                    match (r1, r2) {
                                        (Ok(()), Ok(())) => list_saved.set(String::new()),
                                        _ => list_saved.set(loc(&lang, "Nie udało się zapisać.", "Failed to save.")),
                                    }
                                    reload_cfg(state, cfg, cfg_err).await;
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
                            move |_| {
                                let state = state.clone();
                                let cfg = cfg.clone();
                                let cfg_err = cfg_err.clone();
                                let mut list_saved = list_saved.clone();
                                let lang = lang_c.clone();
                                async move {
                                    let en = skills_en.read().split('\n').map(|s| s.to_string()).collect();
                                    let dis = skills_dis.read().split('\n').map(|s| s.to_string()).collect();
                                    let r1 = crate::vibeconfig::api_set_vibe_list(token(&state), "enabled_skills".into(), en).await;
                                    let r2 = crate::vibeconfig::api_set_vibe_list(token(&state), "disabled_skills".into(), dis).await;
                                    match (r1, r2) {
                                        (Ok(()), Ok(())) => list_saved.set(String::new()),
                                        _ => list_saved.set(loc(&lang, "Nie udało się zapisać.", "Failed to save.")),
                                    }
                                    reload_cfg(state, cfg, cfg_err).await;
                                }
                            }
                        },
                        "{tr(&lang, \"saveLabel\")}"
                    }
                    if !list_saved.read().is_empty() {
                        span { class: "field-hint", style: "margin-left:10px;", "{list_saved.read()}" }
                    }
                }
                {files_panel(lang.clone(), state.clone(), "skills", files, sel_path, editor, new_name, file_saved, scope)}
                {err_el}
            }
        },
        "agents" => files_panel(lang.clone(), state.clone(), "agents", files, sel_path, editor, new_name, file_saved, scope),
        "prompts" => files_panel(lang.clone(), state.clone(), "prompts", files, sel_path, editor, new_name, file_saved, scope),
        "sandbox" => rsx! {
            div { class: "settings-section",
                div { class: "sandbox-planned-card",
                    span { class: "sandbox-planned-badge", "{tr(&lang, \"sandboxPlannedBadge\")}" }
                    h3 { class: "sandbox-planned-title", "{tr(&lang, \"sandbox\")}" }
                    p { class: "field-hint", "{tr(&lang, \"sandboxPlannedDesc\")}" }
                }
            }
        },
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
                            move |_| {
                                let state = state.clone();
                                let cfg = cfg.clone();
                                let cfg_err = cfg_err.clone();
                                let mut list_saved = list_saved.clone();
                                let _lang = lang_c.clone();
                                async move {
                                    match crate::vibeconfig::api_save_vibe_config_raw(
                                        token(&state), raw_toml.read().clone(),
                                    )
                                    .await
                                    {
                                        Ok(()) => list_saved.set(String::new()),
                                        Err(e) => list_saved.set(e.to_string()),
                                    }
                                    reload_cfg(state, cfg, cfg_err).await;
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
        _ => rsx! {
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
            }
        },
    };

    rsx! {
        div { class: "settings-view",
            div { class: "settings-header",
                div { class: "settings-title-row",
                    h2 { class: "settings-title", "{tr(&lang, \"settings\")}" }
                }
            }
            div { class: "settings-tabs scope-switch",
                button { class: "{tab_cls(\"general\")}", onclick: move |_| tab.set("general".into()), "{tr(&lang, \"general\")}" }
                button { class: "{tab_cls(\"acp\")}", onclick: move |_| tab.set("acp".into()), "ACP" }
                button { class: "{tab_cls(\"sandbox\")}", onclick: move |_| tab.set("sandbox".into()), "{tr(&lang, \"sandbox\")}" }
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
