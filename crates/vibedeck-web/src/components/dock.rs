//! Dock: Files / Git panel on the right side of the chat view.
//!
//! Files tab: working-directory browser (breadcrumb + directory rows),
//! click a file to preview it (text / markdown / csv / image / xlsx /
//! docx / pdf — server-side extraction, see `fsgit.rs`).
//! Git tab: `git status --porcelain` entries; click a file to see its
//! diff. Same CSS classes as the Nexus dock.

use dioxus::prelude::*;

use crate::components::code_editor::CodeEditor;
use crate::fsgit::{
    api_git_diff, api_git_status, api_list_dir, api_read_file, api_save_file, FileEntry,
    GitStatus, PreviewData,
};
use crate::i18n::tr;
use crate::state::{lang, token, ClientState, View};

fn fmt_size(n: u64) -> String {
    if n >= 1024 * 1024 {
        format!("{:.1} MB", n as f64 / (1024.0 * 1024.0))
    } else if n >= 1024 {
        format!("{:.1} kB", n as f64 / 1024.0)
    } else {
        format!("{n} B")
    }
}

fn basename_or(p: &str) -> String {
    p.trim_end_matches('/')
        .rsplit('/')
        .next()
        .unwrap_or(".")
        .to_string()
}

/// Split csv/tsv source into rows (client-side, capped).
fn parse_csv(src: &str) -> Vec<Vec<String>> {
    let sep = if src
        .lines()
        .next()
        .map(|l| l.contains('\t') && !l.contains(','))
        .unwrap_or(false)
    {
        '\t'
    } else {
        ','
    };
    src.lines()
        .take(300)
        .map(|line| {
            line.split(sep)
                .take(64)
                .map(|c| c.trim_matches('"').to_string())
                .collect()
        })
        .collect()
}

pub fn Dock() -> Element {
    let state = use_context::<Signal<ClientState>>();

    let tab = use_signal(|| "files".to_string());
    let mut entries = use_signal(Vec::<FileEntry>::new);
    let git = use_signal(|| None::<GitStatus>);
    let mut boot = use_signal(|| false);

    let s = state.read();
    let lang = lang(&state);
    let open = s.snapshot.config.dock_open;
    let view = s.view;
    let wd = s.snapshot.working_directory.clone();
    drop(s);

    if !open || view != View::Chat {
        return VNode::empty();
    }

    // local state that survives tab switches but resets with the component
    let cwd = use_signal(String::new);
    let mut files_msg = use_signal(String::new);
    let preview = use_signal(|| None::<(String, PreviewData)>);
    let git_diff = use_signal(|| None::<(String, String)>);

    // one-shot bootstrap: fetch the root listing when the dock first shows
    if !*boot.read() {
        boot.set(true);
        let st = state.clone();
        spawn(async move {
            match api_list_dir(token(&st), String::new()).await {
                Ok(v) => entries.set(v),
                Err(e) => files_msg.set(e.to_string()),
            }
        });
    }

    let files_cls = if *tab.read() == "files" {
        "dock-tab active"
    } else {
        "dock-tab"
    };
    let git_cls = if *tab.read() == "git" {
        "dock-tab active"
    } else {
        "dock-tab"
    };

    let current_tab = tab.read().clone();
    let body_el: Element = if current_tab == "files" {
        files_panel(
            state.clone(),
            lang.clone(),
            wd,
            cwd,
            entries,
            files_msg,
            preview,
        )
    } else {
        git_panel(state.clone(), lang.clone(), git, git_diff)
    };

    rsx! {
        aside { class: "dock",
            div { class: "dock-tabs",
                button {
                    class: "{files_cls}",
                    onclick: {
                        let state = state.clone();
                        let mut tab = tab.clone();
                        let mut entries = entries.clone();
                        let mut files_msg = files_msg.clone();
                        move |_| {
                            tab.set("files".into());
                            spawn(async move {
                                match api_list_dir(token(&state), String::new()).await {
                                    Ok(v) => entries.set(v),
                                    Err(e) => files_msg.set(e.to_string()),
                                }
                            });
                        }
                    },
                    "{tr(&lang, \"filesTab\")}"
                }
                button {
                    class: "{git_cls}",
                    onclick: {
                        let state = state.clone();
                        let mut tab = tab.clone();
                        let mut git = git.clone();
                        move |_| {
                            tab.set("git".into());
                            spawn(async move {
                                if let Ok(s) = api_git_status(token(&state)).await {
                                    git.set(Some(s));
                                }
                            });
                        }
                    },
                    "{tr(&lang, \"gitTab\")}"
                }
            }
            div { class: "dock-body",
                {body_el}
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Files panel
// ---------------------------------------------------------------------------

fn files_panel(
    state: Signal<ClientState>,
    lang: String,
    wd: String,
    cwd: Signal<String>,
    entries: Signal<Vec<FileEntry>>,
    files_msg: Signal<String>,
    preview: Signal<Option<(String, PreviewData)>>,
) -> Element {
    let list = entries.read().clone();
    let msg = files_msg.read().clone();
    let pv = preview.read().clone();

    // preview overlay takes over the dock body
    if let Some((path, data)) = pv {
        return rsx! { PreviewPanel { state, lang, path, data, preview } };
    }

    // ---------------- breadcrumb ----------------
    let cur = cwd.read().clone();
    let mut crumb_els: Vec<Element> = Vec::new();
    crumb_els.push(rsx! {
        button {
            class: "filetree-root-name",
            onclick: {
                let state = state.clone();
                let mut cwd = cwd.clone();
                let mut entries = entries.clone();
                let mut files_msg = files_msg.clone();
                move |_| {
                    cwd.set(String::new());
                    spawn(async move {
                        match api_list_dir(token(&state), String::new()).await {
                            Ok(v) => entries.set(v),
                            Err(e) => files_msg.set(e.to_string()),
                        }
                    });
                }
            },
            "📁 {basename_or(&wd)}"
        }
    });
    if !cur.is_empty() {
        let parts: Vec<&str> = cur.split('/').collect();
        let mut acc = String::new();
        for part in parts {
            if !acc.is_empty() {
                acc.push('/');
            }
            acc.push_str(part);
            crumb_els.push(rsx! {
                button {
                    class: "filetree-root-name",
                    onclick: {
                        let state = state.clone();
                        let mut cwd = cwd.clone();
                        let mut entries = entries.clone();
                        let mut files_msg = files_msg.clone();
                        let target = acc.clone();
                        move |_| {
                            cwd.set(target.clone());
                            let target = target.clone();
                            spawn(async move {
                                match api_list_dir(token(&state), target).await {
                                    Ok(v) => entries.set(v),
                                    Err(e) => files_msg.set(e.to_string()),
                                }
                            });
                        }
                    },
                    "/ {part}"
                }
            });
        }
    }

    // ---------------- rows ----------------
    let row_els: Vec<Element> = list
        .iter()
        .map(|e| {
            file_row(
                state.clone(),
                e,
                cwd.clone(),
                entries.clone(),
                files_msg.clone(),
                preview.clone(),
            )
        })
        .collect();

    let empty_el: Element = if list.is_empty() && msg.is_empty() {
        rsx! { div { class: "filetree-empty", "{tr(&lang, \"fileTreeEmpty\")}" } }
    } else {
        VNode::empty()
    };
    let msg_el: Element = if msg.is_empty() {
        VNode::empty()
    } else {
        rsx! { div { class: "git-error", "{msg}" } }
    };

    rsx! {
        div { class: "filetree",
            div { class: "filetree-root", style: "display:flex;gap:2px;flex-wrap:wrap;",
                {crumb_els.iter().cloned()}
            }
            {msg_el}
            div { class: "filetree-body",
                {row_els.iter().cloned()}
                {empty_el}
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn file_row(
    state: Signal<ClientState>,
    e: &FileEntry,
    cwd: Signal<String>,
    entries: Signal<Vec<FileEntry>>,
    files_msg: Signal<String>,
    preview: Signal<Option<(String, PreviewData)>>,
) -> Element {
    let name = e.name.clone();
    let path = e.path.clone();
    let is_dir = e.is_dir;
    let size = fmt_size(e.size);
    let icon = if is_dir { "📁" } else { "📄" };

    rsx! {
        div {
            class: "filetree-item",
            onclick: {
                let state = state.clone();
                let mut cwd = cwd.clone();
                let mut entries = entries.clone();
                let mut files_msg = files_msg.clone();
                let mut preview = preview.clone();
                let name = name.clone();
                let path = path.clone();
                move |_| {
                    if is_dir {
                        let path = path.clone();
                        cwd.set(path.clone());
                        spawn(async move {
                            match api_list_dir(token(&state), path).await {
                                Ok(v) => entries.set(v),
                                Err(e) => files_msg.set(e.to_string()),
                            }
                        });
                    } else {
                        let path = path.clone();
                        let name = name.clone();
                        preview.set(Some((
                            name.clone(),
                            PreviewData {
                                kind: "loading".into(),
                                file_name: name,
                                ..PreviewData::empty()
                            },
                        )));
                        spawn(async move {
                            let data = match api_read_file(token(&state), path.clone()).await {
                                Ok(d) => d,
                                Err(e) => PreviewData {
                                    kind: "error".into(),
                                    text: e.to_string(),
                                    file_name: path.clone(),
                                    ..PreviewData::empty()
                                },
                            };
                            preview.set(Some((path, data)));
                        });
                    }
                }
            },
            span { "{icon}" }
            span { "{name}" }
            span { style: "color:var(--text-dim);font-size:10px;margin-left:auto;",
                if is_dir { "" } else { "{size}" }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Preview panel
// ---------------------------------------------------------------------------

#[component]
fn PreviewPanel(
    state: Signal<ClientState>,
    lang: String,
    path: String,
    data: PreviewData,
    preview: Signal<Option<(String, PreviewData)>>,
) -> Element {
    let name = data.file_name.clone();
    let size = fmt_size(data.file_size);

    // editor state — the component instance is re-created whenever the
    // preview signal swaps the file, so this is per-file state.
    let editing = use_signal(|| false);
    let content = use_signal(String::new);
    let saving = use_signal(|| false);
    let saved = use_signal(|| false);
    let save_err = use_signal(String::new);
    let active_sheet = use_signal(|| 0usize);

    let is_editing = *editing.read();
    let is_saving = *saving.read();
    let is_saved = *saved.read();
    let dirty = is_editing && *content.read() != data.text;
    let err_txt = save_err.read().clone();

    let body_el: Element = if is_editing {
        rsx! {
            div { class: "preview-edit",
                CodeEditor { code: content, id: "pv".to_string() }
            }
        }
    } else {
        match data.kind.as_str() {
            "loading" => rsx! { div { class: "preview-loading", "{tr(&lang, \"previewLoading\")}" } },
            "error" => rsx! { div { class: "preview-error-box", "{data.text}" } },
            "binary" => rsx! {
                div { class: "preview-loading",
                    "{tr(&lang, \"binaryPreviewHint\")} ({name}, {size})"
                }
            },
            "image" => rsx! { img { class: "preview-image", src: "{data.data_url}", alt: "{name}" } },
            "markdown" => {
                let html = crate::md::to_html(&data.text);
                rsx! { div { class: "preview-markdown", dangerous_inner_html: html } }
            }
            "csv" => sheet_table(&parse_csv(&data.text)),
            "xlsx" => {
                if data.sheets.is_empty() {
                    rsx! { div { class: "preview-loading", "{tr(&lang, \"previewLoading\")}" } }
                } else {
                    let idx = (*active_sheet.read()).min(data.sheets.len() - 1);
                    let tabs: Vec<Element> = data
                        .sheets
                        .iter()
                        .enumerate()
                        .map(|(i, sh)| {
                            let cls = if i == idx { "sheet-tab active" } else { "sheet-tab" };
                            let mut active_sheet = active_sheet.clone();
                            rsx! {
                                button {
                                    class: "{cls}",
                                    onclick: move |_| active_sheet.set(i),
                                    "{sh.name}"
                                }
                            }
                        })
                        .collect();
                    let rows = data.sheets[idx].rows.clone();
                    rsx! {
                        div { class: "preview-sheet",
                            div { class: "sheet-tabs", {tabs.iter().cloned()} }
                            {sheet_table(&rows)}
                        }
                    }
                }
            }
            "doc" | "slides" => rsx! {
                div { class: "preview-doc", dangerous_inner_html: "{data.html}" }
            },
            "pdf" => {
                if data.pages.is_empty() {
                    let lines: Vec<Element> = data
                        .text
                        .lines()
                        .map(|l| rsx! { p { style: "margin:0 0 8px 0;", "{l}" } })
                        .collect();
                    rsx! {
                        div { class: "preview-pdf",
                            {lines.iter().cloned()}
                        }
                    }
                } else {
                    let imgs: Vec<Element> = data
                        .pages
                        .iter()
                        .map(|p| rsx! { img { class: "pdf-page", src: "{p}", alt: "page" } })
                        .collect();
                    rsx! { div { class: "pdf-pages", {imgs.iter().cloned()} } }
                }
            }
            _ => {
                let html = crate::highlight::highlight_auto(&name, &data.text);
                rsx! { pre { class: "preview-text code-view", dangerous_inner_html: html } }
            }
        }
    };

    let edit_btn: Element = if data.editable {
        if is_editing {
            let mut editing = editing.clone();
            rsx! {
                button {
                    class: "file-preview-btn",
                    title: "{tr(&lang, \"previewEdit\")}",
                    onclick: move |_| editing.set(false),
                    "✎"
                }
            }
        } else {
            let mut editing = editing.clone();
            let text = data.text.clone();
            let mut content = content.clone();
            rsx! {
                button {
                    class: "file-preview-btn",
                    title: "{tr(&lang, \"previewEdit\")}",
                    onclick: move |_| {
                        content.set(text.clone());
                        editing.set(true);
                    },
                    "✎"
                }
            }
        }
    } else {
        VNode::empty()
    };

    let save_btn: Element = if is_editing {
        rsx! {
            button {
                class: if dirty { "file-preview-btn primary" } else { "file-preview-btn" },
                disabled: !dirty || is_saving,
                onclick: {
                    let st = state.clone();
                    let p = path.clone();
                    let mut saving = saving.clone();
                    let saved = saved.clone();
                    let save_err = save_err.clone();
                    let content = content.clone();
                    let preview = preview.clone();
                    let data = data.clone();
                    move |_| {
                        let c = content.read().clone();
                        saving.set(true);
                        let st = st.clone();
                        let p = p.clone();
                        let data = data.clone();
                        let mut saving = saving.clone();
                        let mut saved = saved.clone();
                        let mut save_err = save_err.clone();
                        let mut preview = preview.clone();
                        spawn(async move {
                            match api_save_file(token(&st), p.clone(), c.clone()).await {
                                Ok(()) => {
                                    saved.set(true);
                                    save_err.set(String::new());
                                    let mut d = data.clone();
                                    d.text = c.clone();
                                    preview.set(Some((p, d)));
                                }
                                Err(e) => {
                                    saved.set(false);
                                    save_err.set(e.to_string());
                                }
                            }
                            saving.set(false);
                        });
                    }
                },
                if is_saving { "{tr(&lang, \"previewSaving\")}" } else { "{tr(&lang, \"previewSave\")}" }
            }
        }
    } else {
        VNode::empty()
    };

    let save_status: Element = if is_editing {
        if !err_txt.is_empty() {
            rsx! { span { class: "preview-save-status error", "{err_txt}" } }
        } else if is_saved && !dirty {
            rsx! { span { class: "preview-save-status ok", "✓ {tr(&lang, \"previewSaved\")}" } }
        } else {
            VNode::empty()
        }
    } else {
        VNode::empty()
    };

    let mut preview = preview.clone();
    rsx! {
        div { class: "file-preview",
            div { class: "file-preview-header",
                div { class: "file-preview-title",
                    span { class: "file-preview-name", "{name}" }
                    span { class: "file-preview-size", "{size}" }
                }
                div { class: "file-preview-actions",
                    {save_status}
                    {edit_btn}
                    {save_btn}
                    button {
                        class: "file-preview-btn",
                        onclick: move |_| preview.set(None),
                        "✕"
                    }
                }
            }
            div { class: "file-preview-body",
                {body_el}
            }
        }
    }
}

fn sheet_table(rows: &[Vec<String>]) -> Element {
    let row_els: Vec<Element> = rows
        .iter()
        .enumerate()
        .map(|(ri, row)| {
            let cells: Vec<Element> = row
                .iter()
                .map(|c| {
                    if ri == 0 {
                        rsx! { th { style: "border:1px solid var(--border);padding:4px 8px;text-align:left;", "{c}" } }
                    } else {
                        rsx! { td { style: "border:1px solid var(--border);padding:4px 8px;", "{c}" } }
                    }
                })
                .collect();
            rsx! { tr { {cells.iter().cloned()} } }
        })
        .collect();
    rsx! {
        div { class: "preview-sheet",
            div { style: "overflow:auto;flex:1;",
                table { style: "border-collapse:collapse;font-size:12px;font-family:monospace;",
                    {row_els.iter().cloned()}
                }
            }
        }
    }
}

fn git_panel(
    state: Signal<ClientState>,
    lang: String,
    git: Signal<Option<GitStatus>>,
    git_diff: Signal<Option<(String, String)>>,
) -> Element {
    let st = git.read().clone();
    let diff = git_diff.read().clone();

    // diff view replaces the list while open
    if let Some((path, text)) = diff {
        return diff_panel(path, text, git_diff);
    }

    let (branch, ahead, behind, entries, err) = match &st {
        Some(s) => (
            s.branch.clone(),
            s.ahead,
            s.behind,
            s.entries.clone(),
            s.error.clone(),
        ),
        None => (String::new(), 0, 0, Vec::new(), String::new()),
    };

    let ab_el: Element = if ahead > 0 || behind > 0 {
        rsx! { span { class: "git-ahead-behind", "↑{ahead} ↓{behind}" } }
    } else {
        VNode::empty()
    };

    let entry_els: Vec<Element> = entries
        .iter()
        .map(|e| {
            let code = e.code.clone();
            let path = e.path.clone();
            let staged = e.staged;
            let dir = path.rsplit('/').next().unwrap_or(&path).to_string();
            let dir_part = match path.rfind('/') {
                Some(pos) => path[..pos].to_string(),
                None => String::new(),
            };
            let cls = match code.as_str() {
                "A" => "git-status-code git-status-A",
                "M" => "git-status-code git-status-M",
                "D" => "git-status-code git-status-D",
                "R" | "C" => "git-status-code git-status-R",
                "?" => "git-status-code git-status-\\?",
                _ => "git-status-code",
            };
            rsx! {
                div {
                    class: "git-file",
                    onclick: {
                        let state = state.clone();
                        let mut git_diff = git_diff.clone();
                        let path = path.clone();
                        move |_| {
                            let path = path.clone();
                            spawn(async move {
                                let d = api_git_diff(token(&state), staged, path.clone())
                                    .await
                                    .unwrap_or_default();
                                git_diff.set(Some((path, d)));
                            });
                        }
                    },
                    span { class: "{cls}", "{code}" }
                    span { class: "git-file-name", "{dir}" }
                    span { class: "git-file-dir", "{dir_part}" }
                }
            }
        })
        .collect();

    let list_el: Element = if entries.is_empty() {
        if err.is_empty() {
            rsx! { div { class: "git-empty", "{tr(&lang, \"gitNoChanges\")}" } }
        } else {
            rsx! { div { class: "git-error", "{err}" } }
        }
    } else {
        rsx! { div { class: "git-files", {entry_els.iter().cloned()} } }
    };

    let branch_label = if branch.is_empty() {
        tr(&lang, "noProject").to_string()
    } else {
        branch.clone()
    };

    rsx! {
        div { class: "git-panel",
            div { class: "git-toolbar",
                span { class: "git-branch",
                    span { class: "git-branch-icon", "⎇" }
                    "{branch_label}"
                }
                {ab_el}
                button {
                    class: "git-refresh-btn",
                    onclick: {
                        let state = state.clone();
                        let mut git = git.clone();
                        move |_| {
                            spawn(async move {
                                if let Ok(s) = api_git_status(token(&state)).await {
                                    git.set(Some(s));
                                }
                            });
                        }
                    },
                    "⟳"
                }
            }
            {list_el}
        }
    }
}

fn diff_panel(path: String, text: String, mut git_diff: Signal<Option<(String, String)>>) -> Element {
    let line_els: Vec<Element> = text
        .lines()
        .take(2000)
        .map(|l| {
            let style = if l.starts_with('+') && !l.starts_with("+++") {
                "background:rgba(46,160,67,0.15);"
            } else if l.starts_with('-') && !l.starts_with("---") {
                "background:rgba(248,81,73,0.12);"
            } else if l.starts_with('@') {
                "color:var(--text-dim);"
            } else {
                ""
            };
            rsx! {
                div { class: "diff-line", style: "{style}",
                    span { class: "diff-text", "{l}" }
                }
            }
        })
        .collect();

    rsx! {
        div { class: "file-preview",
            div { class: "file-preview-header",
                div { class: "file-preview-title",
                    span { class: "file-preview-name", "{path}" }
                }
                button {
                    class: "file-preview-btn",
                    onclick: move |_| git_diff.set(None),
                    "✕"
                }
            }
            div { class: "file-preview-body",
                div { class: "preview-diff",
                    {line_els.iter().cloned()}
                }
            }
        }
    }
}
