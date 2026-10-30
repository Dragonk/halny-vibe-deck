//! Dock: Files / Git panel on the right side of the chat view.
//!
//! Mirrors the Nexus dock: Files/Git main tabs plus dynamically opened
//! preview tabs (file previews and git diffs) that live next to them in
//! the tab bar, each closable with its own ✕ button. Tab state survives
//! switching between Files/Git.

use dioxus::prelude::*;

use crate::api::api_worktrees_list;
use crate::components::code_editor::CodeEditor;
use crate::fsgit::{
    api_git_commit, api_git_diff, api_git_log, api_git_push, api_git_stage, api_git_status,
    api_git_unstage, api_list_dir, api_read_file, api_save_file, FileEntry, GitCommit, GitStatus,
    PreviewData,
};
use crate::i18n::tr;
use crate::state::{lang, session_git_ctx, token, ClientState, View};

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

/// An open preview tab in the dock tab bar (Nexus `PreviewTab`).
#[derive(Clone, Debug, PartialEq)]
struct PreviewTab {
    id: u64,
    /// "file" | "diff"
    kind: String,
    name: String,
    path: String,
    /// remote context (v0.7.0 remoteFs parity): which SSH connection the
    /// file lives on and the session root its browse paths are relative to
    remote_id: Option<String>,
    root: String,
    staged: bool,
    data: Option<PreviewData>,
    diff: String,
    loaded: bool,
    /// Open straight into edit mode (context-menu "Edit", round 4 C5).
    start_edit: bool,
}

fn close_tab(mut tabs: Signal<Vec<PreviewTab>>, mut active: Signal<Option<u64>>, id: u64) {
    {
        let mut v = tabs.write();
        v.retain(|t| t.id != id);
    }
    if *active.read() == Some(id) {
        active.set(None);
    }
}

#[allow(clippy::too_many_arguments)]
fn open_file_tab(
    state: Signal<ClientState>,
    mut tabs: Signal<Vec<PreviewTab>>,
    mut active: Signal<Option<u64>>,
    mut next_id: Signal<u64>,
    path: String,
    name: String,
    remote_id: Option<String>,
    root: String,
) {
    open_file_tab_edit(state, tabs, active, next_id, path, name, remote_id, root, false)
}

#[allow(clippy::too_many_arguments)]
fn open_file_tab_edit(
    state: Signal<ClientState>,
    mut tabs: Signal<Vec<PreviewTab>>,
    mut active: Signal<Option<u64>>,
    mut next_id: Signal<u64>,
    path: String,
    name: String,
    remote_id: Option<String>,
    root: String,
    start_edit: bool,
) {
    let existing = tabs
        .read()
        .iter()
        .find(|t| t.kind == "file" && t.path == path && t.remote_id == remote_id)
        .map(|t| t.id);
    if let Some(id) = existing {
        if start_edit {
            let mut v = tabs.write();
            if let Some(t) = v.iter_mut().find(|t| t.id == id) {
                t.start_edit = true;
            }
        }
        active.set(Some(id));
        return;
    }
    let id = *next_id.read();
    next_id.set(id + 1);
    {
        let mut v = tabs.write();
        v.push(PreviewTab {
            id,
            kind: "file".into(),
            name: name.clone(),
            path: path.clone(),
            remote_id: remote_id.clone(),
            root: root.clone(),
            staged: false,
            data: None,
            diff: String::new(),
            loaded: false,
            start_edit,
        });
    }
    active.set(Some(id));
    spawn(async move {
        let data = match api_read_file(token(&state), path.clone(), remote_id, root).await {
            Ok(d) => d,
            Err(e) => PreviewData {
                kind: "error".into(),
                text: e.to_string(),
                file_name: path.clone(),
                ..PreviewData::empty()
            },
        };
        let mut v = tabs.write();
        if let Some(t) = v.iter_mut().find(|t| t.id == id) {
            t.data = Some(data);
            t.loaded = true;
        }
    });
}

fn open_diff_tab(
    state: Signal<ClientState>,
    mut tabs: Signal<Vec<PreviewTab>>,
    mut active: Signal<Option<u64>>,
    mut next_id: Signal<u64>,
    path: String,
    staged: bool,
) {
    let existing = tabs
        .read()
        .iter()
        .find(|t| t.kind == "diff" && t.path == path && t.staged == staged)
        .map(|t| t.id);
    if let Some(id) = existing {
        active.set(Some(id));
        return;
    }
    let id = *next_id.read();
    next_id.set(id + 1);
    let name = basename_or(&path);
    {
        let mut v = tabs.write();
        v.push(PreviewTab {
            id,
            kind: "diff".into(),
            name,
            path: path.clone(),
            remote_id: None,
            root: String::new(),
            staged,
            data: None,
            diff: String::new(),
            loaded: false,
            start_edit: false,
        });
    }
    active.set(Some(id));
    spawn(async move {
        // git runs on the session cwd (Nexus openDiff), over SSH for
        // remote sessions
        let (gwd, grid) = session_git_ctx(&state);
        let d = api_git_diff(token(&state), gwd, grid, staged, path.clone())
            .await
            .unwrap_or_default();
        let mut v = tabs.write();
        if let Some(t) = v.iter_mut().find(|t| t.id == id) {
            t.diff = d;
            t.loaded = true;
        }
    });
}

// ---------------------------------------------------------------------------
// Git panel actions (Nexus GitPanel refresh/stage/unstage/commit/push).
// Git runs on the current session's cwd — remote SSH sessions pipe the
// command through the connection's ssh command.
// ---------------------------------------------------------------------------

/// Fetch status + log for the current session (Nexus `refresh`).
async fn git_refresh(
    state: Signal<ClientState>,
    mut git: Signal<Option<GitStatus>>,
    mut git_log: Signal<Vec<GitCommit>>,
) {
    let (cwd, rid) = session_git_ctx(&state);
    let tok = token(&state);
    if let Ok(s) = api_git_status(tok.clone(), cwd.clone(), rid.clone()).await {
        git.set(Some(s));
    }
    if let Ok(l) = api_git_log(tok.clone(), cwd.clone(), rid.clone(), 30).await {
        git_log.set(l);
    }
    // linked worktrees of this repo (workspace/git/worktrees/list)
    let _ = api_worktrees_list(tok, cwd).await;
}

/// Stage (or unstage) paths, then refresh (Nexus stagePaths/unstagePaths).
async fn git_stage_action(
    state: Signal<ClientState>,
    git: Signal<Option<GitStatus>>,
    git_log: Signal<Vec<GitCommit>>,
    mut git_error: Signal<String>,
    paths: Vec<String>,
    unstage: bool,
) {
    let (cwd, rid) = session_git_ctx(&state);
    let tok = token(&state);
    let res = if unstage {
        api_git_unstage(tok, cwd, rid, paths).await
    } else {
        api_git_stage(tok, cwd, rid, paths).await
    };
    match res {
        Ok(e) if !e.is_empty() => git_error.set(e),
        Err(e) => git_error.set(e.to_string()),
        _ => {}
    }
    git_refresh(state, git, git_log).await;
}

/// Commit the staged changes with the drafted message (Nexus handleCommit).
async fn git_commit_action(
    state: Signal<ClientState>,
    git: Signal<Option<GitStatus>>,
    git_log: Signal<Vec<GitCommit>>,
    mut git_error: Signal<String>,
    mut commit_msg: Signal<String>,
    mut git_busy: Signal<bool>,
) {
    let msg = commit_msg.read().trim().to_string();
    if msg.is_empty() || *git_busy.read() {
        return;
    }
    git_busy.set(true);
    let (cwd, rid) = session_git_ctx(&state);
    let tok = token(&state);
    match api_git_commit(tok, cwd, rid, msg).await {
        Ok(e) if !e.is_empty() => git_error.set(e),
        Err(e) => git_error.set(e.to_string()),
        Ok(_) => {
            commit_msg.set(String::new());
            git_error.set(String::new());
        }
    }
    git_refresh(state, git, git_log).await;
    git_busy.set(false);
}

/// Push to the remote (Nexus handlePush).
async fn git_push_action(
    state: Signal<ClientState>,
    git: Signal<Option<GitStatus>>,
    git_log: Signal<Vec<GitCommit>>,
    mut git_error: Signal<String>,
    mut git_busy: Signal<bool>,
) {
    if *git_busy.read() {
        return;
    }
    git_busy.set(true);
    let (cwd, rid) = session_git_ctx(&state);
    let tok = token(&state);
    match api_git_push(tok, cwd, rid).await {
        Ok(e) if !e.is_empty() => git_error.set(e),
        Err(e) => git_error.set(e.to_string()),
        _ => {}
    }
    git_refresh(state, git, git_log).await;
    git_busy.set(false);
}

fn loc2(lang: &str, pl: &str, en: &str) -> String {
    if lang == "pl" { pl.to_string() } else { en.to_string() }
}

/// JSON-string-escape for JS eval snippets.
fn js_str(s: &str) -> String {
    serde_json::to_string(s).unwrap_or_else(|_| "\"\"".to_string())
}

/// Drag & drop upload onto the dock file tree (stable v1.0.26 parity):
/// files dropped on a folder row (or the root crumb) are read as base64,
/// parked on `window.__vdDropTree` as `{dir, files:[{name,b64}]}`, and the
/// hidden `#vd-droptree-bridge` button is clicked — the Rust handler writes
/// them into the target directory (session root confined, local or SSH)
/// and reloads the listing. Hovering a collapsed folder for 800 ms expands
/// it; the hovered row gets the `drop-target` highlight. Scoped to the dock
/// (capture phase + stopPropagation) so the chat drop handler never sees
/// these events.
#[cfg(any(feature = "web", feature = "desktop"))]
const DROP_TREE_JS: &str = r#"
(function () {
  if (window.__vdDropTreeInit) return; window.__vdDropTreeInit = 1;
  function hasFiles(e) {
    try { return Array.from(e.dataTransfer.types || []).includes('Files'); } catch (err) { return false; }
  }
  function treeItem(e) {
    var t = e.target;
    if (!t || !t.closest) return null;
    return t.closest('.dock .filetree-item');
  }
  function rootEl(e) {
    var t = e.target;
    if (!t || !t.closest) return null;
    return t.closest('.dock .filetree-root');
  }
  var holdTimers = {};
  function clearHold(p) { if (holdTimers[p]) { clearTimeout(holdTimers[p]); delete holdTimers[p]; } }
  function clearHighlights() {
    var els = document.querySelectorAll('.filetree-item.drop-target, .filetree-root.drop-target');
    for (var i = 0; i < els.length; i++) els[i].classList.remove('drop-target');
  }
  document.addEventListener('dragover', function (e) {
    if (!hasFiles(e)) return;
    var item = treeItem(e);
    var root = item ? null : rootEl(e);
    if (!item && !root) return;
    e.preventDefault();
    e.stopPropagation();
    clearHighlights();
    var el = item || root;
    el.classList.add('drop-target');
    if (item) {
      var p = item.getAttribute('data-path') || '';
      // hovering a collapsed folder briefly expands it
      if (item.getAttribute('data-isdir') === '1' && !item.classList.contains('open')) {
        if (!holdTimers[p]) {
          holdTimers[p] = setTimeout(function () { delete holdTimers[p]; item.click(); }, 800);
        }
      }
    }
  }, true);
  document.addEventListener('dragleave', function (e) {
    var item = treeItem(e);
    var rel = e.relatedTarget;
    if (item) {
      if (rel && item.contains(rel)) return;
      item.classList.remove('drop-target');
      clearHold(item.getAttribute('data-path') || '');
    }
    var root = rootEl(e);
    if (root && !(rel && root.contains(rel))) root.classList.remove('drop-target');
  }, true);
  document.addEventListener('drop', function (e) {
    if (!hasFiles(e)) return;
    var item = treeItem(e);
    var root = item ? null : rootEl(e);
    if (!item && !root) return;
    e.preventDefault();
    e.stopPropagation();
    clearHighlights();
    for (var k in holdTimers) clearHold(k);
    var dir = '';
    if (item) {
      var p = item.getAttribute('data-path') || '';
      // dropping on a file row uploads into its parent directory
      dir = item.getAttribute('data-isdir') === '1' ? p : p.split('/').slice(0, -1).join('/');
    } else {
      dir = root.getAttribute('data-path') || '';
    }
    var files = Array.from(e.dataTransfer.files || []);
    if (!files.length) return;
    window.__vdDropTree = { dir: dir, files: [] };
    var pending = files.length;
    files.forEach(function (f) {
      var reader = new FileReader();
      reader.onload = function () {
        var url = String(reader.result || '');
        window.__vdDropTree.files.push({ name: f.name, b64: url.split(',')[1] || '' });
        if (--pending === 0) { var b = document.getElementById('vd-droptree-bridge'); if (b) b.click(); }
      };
      reader.onerror = function () { if (--pending === 0) { var b = document.getElementById('vd-droptree-bridge'); if (b) b.click(); } };
      reader.readAsDataURL(f);
    });
  }, true);
})();
"#;

pub fn Dock() -> Element {
    let state = use_context::<Signal<ClientState>>();

    let tab = use_signal(|| "files".to_string());
    let entries = use_signal(Vec::<FileEntry>::new);
    let git = use_signal(|| None::<GitStatus>);
    let git_log = use_signal(Vec::<GitCommit>::new);
    let git_commit_msg = use_signal(String::new);
    let git_busy = use_signal(|| false);
    let git_error = use_signal(String::new);
    let mut boot = use_signal(String::new);

    let s = state.read();
    let lang = lang(&state);
    let open = s.snapshot.config.dock_open;
    let view = s.view;
    drop(s);

    // Files tab context (Nexus Dock cwd/remoteId): the *session* working
    // directory — a remote path over SSH for remote sessions. The root
    // crumb label and every list/preview/save call carry it.
    let (mut fs_root, fs_rid) = session_git_ctx(&state);
    if fs_root.trim().is_empty() {
        let s = state.read();
        fs_root = s.snapshot.working_directory.clone();
    }
    let wd = fs_root.clone();

    if view != View::Chat {
        return VNode::empty();
    }
    // Rendered (hidden) even when closed so the mobile slide-in animation
    // has a mounted element to animate.
    let dock_cls = if open { "dock open" } else { "dock" };

    // local state that survives tab switches but resets with the component
    let cwd = use_signal(String::new);
    // Z22: tree expansion + per-directory listings live here, so opening a
    // preview tab and coming back keeps the tree exactly as it was.
    let children = use_signal(std::collections::HashMap::<String, Vec<FileEntry>>::new);
    let expanded = use_signal(Vec::<String>::new);
    let files_msg = use_signal(String::new);
    let tabs = use_signal(Vec::<PreviewTab>::new);
    let active = use_signal(|| None::<u64>);
    let next_id = use_signal(|| 1u64);

    // clickable chat paths (state::open_path, Nexus openFilePreview):
    // consume one request per `preview_open_seq` bump — a directory
    // reveals in the Files tab (revealInFileTree: files tab, no active
    // preview, listing loaded), anything else opens a preview tab.
    let mut seen_open = use_signal(|| 0u64);
    {
        let s = state.read();
        let seq = s.preview_open_seq;
        let req = s.preview_open.clone();
        drop(s);
        if seq != *seen_open.read() {
            seen_open.set(seq);
            if let Some(req) = req {
                let mut tab_w = tab.clone();
                let mut active_w = active.clone();
                let mut cwd_w = cwd.clone();
                if req.reveal {
                    tab_w.set("files".to_string());
                    active_w.set(None);
                    // remote reveal paths are absolute on the remote host;
                    // the breadcrumb works relative to the session root
                    let target_rel = if fs_rid.is_some()
                        && !fs_root.is_empty()
                        && req.path.starts_with(&fs_root)
                    {
                        req.path[fs_root.len()..].trim_start_matches('/').to_string()
                    } else {
                        req.path.clone()
                    };
                    // Round 7 (R12): reveal = the tree from the root with the
                    // target folder (and every ancestor) expanded, its files
                    // listed — not a flat replacement of the listing.
                    cwd_w.set(String::new());
                    let mut ancestors: Vec<String> = Vec::new();
                    let mut acc = String::new();
                    // round 8: split on both separators — Windows/remote
                    // paths use '\' and produced one bogus ancestor
                    for part in target_rel.split(['/', '\\']).filter(|p| !p.is_empty()) {
                        if !acc.is_empty() {
                            acc.push('/');
                        }
                        acc.push_str(part);
                        ancestors.push(acc.clone());
                    }
                    {
                        let mut ex_sig = expanded.clone();
                        let mut ex = ex_sig.write();
                        for a in &ancestors {
                            if !ex.iter().any(|p| p == a) {
                                ex.push(a.clone());
                            }
                        }
                    }
                    let st = state.clone();
                    let mut entries_w = entries.clone();
                    let mut files_msg_w = files_msg.clone();
                    let mut children_w = children.clone();
                    let rid = fs_rid.clone();
                    let root = fs_root.clone();
                    spawn(async move {
                        // root listing drives the tree
                        match api_list_dir(token(&st), String::new(), rid.clone(), root.clone()).await {
                            Ok(v) => entries_w.set(v),
                            Err(e) => files_msg_w.set(e.to_string()),
                        }
                        // every ancestor (incl. the target) gets its children
                        for a in &ancestors {
                            if let Ok(v) = api_list_dir(token(&st), a.clone(), rid.clone(), root.clone()).await {
                                children_w.write().insert(a.clone(), v);
                            }
                        }
                    });
                } else {
                    open_file_tab(
                        state.clone(),
                        tabs.clone(),
                        active.clone(),
                        next_id.clone(),
                        req.path.clone(),
                        basename_or(&req.path),
                        fs_rid.clone(),
                        fs_root.clone(),
                    );
                }
            }
        }
    }

    // one-shot bootstrap: fetch the root listing when the dock first
    // shows, and refetch it whenever the session context changes — browse
    // state is per-session (Nexus fileTreeKey namespaces entries as
    // `r:<rid>:<dir>`, so switching sessions must never keep the previous
    // session's file listing).
    let ctx_key = format!("{}|{}", fs_root, fs_rid.as_deref().unwrap_or(""));
    if *boot.read() != ctx_key {
        boot.set(ctx_key);
        let mut cwd_r = cwd.clone();
        cwd_r.set(String::new());
        let st = state.clone();
        let mut entries = entries.clone();
        let mut files_msg = files_msg.clone();
        let rid = fs_rid.clone();
        let root = fs_root.clone();
        spawn(async move {
            match api_list_dir(token(&st), String::new(), rid, root).await {
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

    // preview tabs row (next to Files/Git, after a separator)
    let tabs_list = tabs.read().clone();
    let active_id = *active.read();
    let preview_tab_els: Vec<Element> = tabs_list
        .iter()
        .map(|t| {
            let cls = if active_id == Some(t.id) {
                "dock-tab dock-tab-preview active"
            } else {
                "dock-tab dock-tab-preview"
            };
            let id = t.id;
            let name = t.name.clone();
            let tabs_c2 = tabs.clone();
            let mut active_c2 = active.clone();
            rsx! {
                div {
                    class: "{cls}",
                    onclick: move |_| active_c2.set(Some(id)),
                    span { class: "dock-tab-name", "{name}" }
                    button {
                        class: "dock-tab-close",
                        onclick: move |ev| {
                            ev.stop_propagation();
                            close_tab(tabs_c2, active_c2, id);
                        },
                        "✕"
                    }
                }
            }
        })
        .collect();

    // body: active preview tab wins over the Files/Git panels
    let open_tab = tabs_list.iter().find(|t| Some(t.id) == active_id).cloned();
    let current_tab = tab.read().clone();
    let body_el: Element = if let Some(t) = open_tab {
        if t.kind == "diff" {
            rsx! {
                DiffPanel { key: "{t.id}", lang: lang.clone(), tab: t.clone(), tabs, active }
            }
        } else {
            rsx! {
                FilePreviewTab {
                    key: "{t.id}",
                    state: state.clone(),
                    lang: lang.clone(),
                    tab: t.clone(),
                    tabs,
                    active,
                }
            }
        }
    } else if current_tab == "files" {
        files_panel(
            state.clone(),
            lang.clone(),
            wd,
            fs_rid.clone(),
            fs_root.clone(),
            cwd,
            entries,
            children,
            expanded,
            files_msg,
            tabs,
            active,
            next_id,
        )
    } else {
        git_panel(
            state.clone(),
            lang.clone(),
            git,
            git_log,
            git_commit_msg,
            git_busy,
            git_error,
            tabs,
            active,
            next_id,
        )
    };

    let mut tab = tab.clone();
    let mut active = active.clone();
    let state_f = state.clone();
    let mut entries_f = entries.clone();
    let mut files_msg_f = files_msg.clone();
    let fs_rid_f = fs_rid.clone();
    let fs_root_f = fs_root.clone();
    let git_g = git.clone();
    let git_log_g = git_log.clone();
    let state_g = state.clone();
    let mut tab_g = tab.clone();
    let mut active_g = active.clone();

    rsx! {
        aside { class: "{dock_cls}",
            div { class: "dock-resize-handle" }
            button {
                class: "mobile-panel-close",
                title: "{crate::i18n::tr(&lang, \"closePanelLabel\")}",
                onclick: {
                    let state = state.clone();
                    move |_| {
                        let state = state.clone();
                        spawn(async move {
                            crate::components::mobile::set_dock_open(state, false).await;
                        });
                    }
                },
                "✕"
            }
            div { class: "dock-tabs",
                button {
                    class: "{files_cls}",
                    onclick: move |_| {
                        tab.set("files".into());
                        active.set(None);
                        let rid = fs_rid_f.clone();
                        let root = fs_root_f.clone();
                        spawn(async move {
                            match api_list_dir(token(&state_f), String::new(), rid, root).await {
                                Ok(v) => entries_f.set(v),
                                Err(e) => files_msg_f.set(e.to_string()),
                            }
                        });
                    },
                    "{tr(&lang, \"filesTab\")}"
                }
                button {
                    class: "{git_cls}",
                    onclick: move |_| {
                        tab_g.set("git".into());
                        active_g.set(None);
                        spawn(async move {
                            git_refresh(state_g, git_g, git_log_g).await;
                        });
                    },
                    "{tr(&lang, \"gitTab\")}"
                }
                span { class: "dock-tab-sep" }
                {preview_tab_els.iter().cloned()}
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

#[allow(clippy::too_many_arguments)]
#[allow(clippy::too_many_arguments)]
fn files_panel(
    state: Signal<ClientState>,
    lang: String,
    wd: String,
    remote_id: Option<String>,
    root: String,
    cwd: Signal<String>,
    entries: Signal<Vec<FileEntry>>,
    children: Signal<std::collections::HashMap<String, Vec<FileEntry>>>,
    expanded: Signal<Vec<String>>,
    files_msg: Signal<String>,
    tabs: Signal<Vec<PreviewTab>>,
    active: Signal<Option<u64>>,
    next_id: Signal<u64>,
) -> Element {
    let file_ctx = use_signal(|| None::<FileCtx>);
    let uploading = use_signal(|| false);
    // one-shot: install the file-tree drag & drop bridge (client only)
    #[cfg(any(feature = "web", feature = "desktop"))]
    {
        let mut js_init = use_signal(|| false);
        if !*js_init.read() {
            js_init.set(true);
            spawn(async move {
                let _ = dioxus::document::eval(DROP_TREE_JS);
            });
        }
    }
    let list = entries.read().clone();
    let msg = files_msg.read().clone();

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
                let remote_id = remote_id.clone();
                let root = root.clone();
                move |_| {
                    cwd.set(String::new());
                    let rid = remote_id.clone();
                    let root = root.clone();
                    spawn(async move {
                        match api_list_dir(token(&state), String::new(), rid, root).await {
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
                        let remote_id = remote_id.clone();
                        let root = root.clone();
                        move |_| {
                            cwd.set(target.clone());
                            let target = target.clone();
                            let rid = remote_id.clone();
                            let root = root.clone();
                            spawn(async move {
                                match api_list_dir(token(&state), target, rid, root).await {
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

    // ---------------- rows (tree, Z22) ----------------
    // Flatten the expanded tree: each row carries its depth for indentation.
    let expanded_set = expanded.read().clone();
    let child_map = children.read().clone();
    let mut flat: Vec<(FileEntry, usize)> = Vec::new();
    fn walk(
        list: &[FileEntry],
        depth: usize,
        expanded: &[String],
        children: &std::collections::HashMap<String, Vec<FileEntry>>,
        out: &mut Vec<(FileEntry, usize)>,
    ) {
        for e in list {
            out.push((e.clone(), depth));
            if e.is_dir && expanded.iter().any(|p| p == &e.path) {
                if let Some(kids) = children.get(&e.path) {
                    if depth < 8 {
                        walk(kids, depth + 1, expanded, children, out);
                    }
                }
            }
        }
    }
    walk(&list, 0, &expanded_set, &child_map, &mut flat);

    let row_els: Vec<Element> = flat
        .iter()
        .map(|(e, depth)| {
            file_row(
                state.clone(),
                e,
                *depth,
                remote_id.clone(),
                root.clone(),
                children.clone(),
                expanded.clone(),
                files_msg.clone(),
                tabs.clone(),
                active.clone(),
                next_id.clone(),
                file_ctx.clone(),
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

    // right-click menu (stable FileTree parity: preview / download / open
    // externally / rename / delete)
    let ctx_el: Element = if let Some(c) = file_ctx.read().clone() {
        let lang2 = lang.clone();
        let state2 = state.clone();
        let rid2 = remote_id.clone();
        let root2 = root.clone();
        let mut file_ctx2 = file_ctx.clone();
        let c2 = c.clone();
        let rename_prompt = loc2(&lang2, "Nowa nazwa", "New name");
        rsx! {
            div {
                class: "ctx-backdrop",
                onclick: {
                    let mut file_ctx = file_ctx2.clone();
                    move |_| file_ctx.set(None)
                },
            }
            div {
                class: "ctx-menu",
                style: "left:{c.x}px;top:{c.y}px;",
                onclick: move |e| e.stop_propagation(),
                // Round 4 (C5): a FOLDER gets only rename + delete — no
                // preview/edit/download/open entries. Files get the full
                // set, with Edit only for text-editable files (docx/xlsx/…
                // have no built-in editor).
                if !c.is_dir {
                    button {
                        class: "ctx-menu-item",
                        onclick: {
                            let mut file_ctx = file_ctx2.clone();
                            let c = c2.clone();
                            let state = state2.clone();
                            let mut tabs = tabs.clone();
                            let mut active = active.clone();
                            let mut next_id = next_id.clone();
                            let rid = rid2.clone();
                            let root = root2.clone();
                            move |_| {
                                file_ctx.set(None);
                                let c = c.clone();
                                let state = state.clone();
                                let rid = rid.clone();
                                let root = root.clone();
                                let mut tabs = tabs.clone();
                                let mut active = active.clone();
                                let mut next_id = next_id.clone();
                                open_file_tab(state, tabs, active, next_id, c.path.clone(), c.name.clone(), rid, root);
                            }
                        },
                        "👁 {loc2(&lang2, \"Podgląd\", \"Preview\")}"
                    }
                    if crate::fsgit::editable_name(&c.name) {
                        button {
                            class: "ctx-menu-item",
                            onclick: {
                                let mut file_ctx = file_ctx2.clone();
                                let c = c2.clone();
                                let state = state2.clone();
                                let mut tabs = tabs.clone();
                                let mut active = active.clone();
                                let mut next_id = next_id.clone();
                                let rid = rid2.clone();
                                let root = root2.clone();
                                move |_| {
                                    file_ctx.set(None);
                                    let c = c.clone();
                                    let state = state.clone();
                                    let rid = rid.clone();
                                    let root = root.clone();
                                    let mut tabs = tabs.clone();
                                    let mut active = active.clone();
                                    let mut next_id = next_id.clone();
                                    open_file_tab_edit(state, tabs, active, next_id, c.path.clone(), c.name.clone(), rid, root, true);
                                }
                            },
                            "✏️ {loc2(&lang2, \"Edytuj\", \"Edit\")}"
                        }
                    }
                }
                if !c.is_dir {
                    button {
                        class: "ctx-menu-item",
                        onclick: {
                            let mut file_ctx = file_ctx2.clone();
                            let c = c2.clone();
                            let state = state2.clone();
                            let rid = rid2.clone();
                            let root = root2.clone();
                            move |_| {
                                file_ctx.set(None);
                                let state = state.clone();
                                let c = c.clone();
                                let rid = rid.clone();
                                let root = root.clone();
                                spawn(async move {
                                    match crate::fsgit::api_download_base64(token(&state), c.path.clone(), rid, root).await {
                                        Ok(b64) => {
                                            let _ = dioxus::document::eval(&format!(
                                                "var a=document.createElement('a');a.href='data:application/octet-stream;base64,'+{};a.download={};document.body.appendChild(a);a.click();a.remove();",
                                                js_str(&b64), js_str(&c.name)
                                            )).await;
                                        }
                                        Err(_) => {}
                                    }
                                });
                            }
                        },
                        "⬇ {loc2(&lang2, \"Pobierz\", \"Download\")}"
                    }
                }
                if crate::state::is_desktop() && remote_id.is_none() {
                    button {
                        class: "ctx-menu-item",
                        onclick: {
                            let mut file_ctx = file_ctx2.clone();
                            let c = c2.clone();
                            let state = state2.clone();
                            move |_| {
                                file_ctx.set(None);
                                let state = state.clone();
                                let c = c.clone();
                                spawn(async move {
                                    let _ = crate::fsgit::api_open_externally(token(&state), c.path.clone()).await;
                                });
                            }
                        },
                        "↗ {loc2(&lang2, \"Otwórz zewnętrznie\", \"Open externally\")}"
                    }
                }
                button {
                    class: "ctx-menu-item",
                    onclick: {
                        let mut file_ctx = file_ctx2.clone();
                        let c = c2.clone();
                        let state = state2.clone();
                        let rid = rid2.clone();
                        let root = root2.clone();
                        move |_| {
                            file_ctx.set(None);
                            let state = state.clone();
                            let c = c.clone();
                            let rid = rid.clone();
                            let root = root.clone();
                            let rename_prompt = rename_prompt.clone();
                            spawn(async move {
                                let new_name = {
                                    #[cfg(any(feature = "web", feature = "desktop"))]
                                    {
                                        match dioxus::document::eval(&format!(
                                            "return window.prompt({}, {});",
                                            js_str(&rename_prompt),
                                            js_str(&c.name)
                                        ))
                                        .join::<String>()
                                        .await
                                        {
                                            Ok(v) => v,
                                            Err(_) => String::new(),
                                        }
                                    }
                                    #[cfg(not(any(feature = "web", feature = "desktop")))]
                                    { String::new() }
                                };
                                let new_name = new_name.trim().to_string();
                                if new_name.is_empty() || new_name == c.name { return; }
                                let _ = crate::fsgit::api_rename_path(token(&state), c.path.clone(), new_name, rid, root).await;
                            });
                        }
                    },
                    "📝 {loc2(&lang2, \"Zmień nazwę\", \"Rename\")}"
                }
                button {
                    class: "ctx-menu-item danger",
                    onclick: {
                        let mut file_ctx = file_ctx2.clone();
                        let c = c2.clone();
                        let state = state2.clone();
                        let rid = rid2.clone();
                        let root = root2.clone();
                        move |_| {
                            file_ctx.set(None);
                            let state = state.clone();
                            let c = c.clone();
                            let rid = rid.clone();
                            let root = root.clone();
                            spawn(async move {
                                let _ = crate::fsgit::api_delete_path(token(&state), c.path.clone(), rid, root).await;
                            });
                        }
                    },
                    "🗑 {loc2(&lang2, \"Usuń\", \"Delete\")}"
                }
            }
        }
    } else {
        VNode::empty()
    };

    let uploading_el: Element = if *uploading.read() {
        rsx! { div { class: "filetree-uploading", "⬆ …" } }
    } else {
        VNode::empty()
    };

    rsx! {
        div { class: "filetree",
            // hidden bridge: DROP_TREE_JS parks dropped files on
            // window.__vdDropTree and clicks this — the handler uploads
            // them into the hovered directory (local or over SSH)
            button {
                id: "vd-droptree-bridge",
                style: "display:none",
                onclick: {
                    let state = state.clone();
                    let mut uploading = uploading.clone();
                    let mut files_msg = files_msg.clone();
                    let entries = entries.clone();
                    let mut children = children.clone();
                    let cwd2 = cwd.clone();
                    let remote_id = remote_id.clone();
                    let root = root.clone();
                    let lang3 = lang.clone();
                    move |_| {
                        let state = state.clone();
                        let mut uploading = uploading.clone();
                        let mut files_msg = files_msg.clone();
                        let mut entries = entries.clone();
                        let mut children = children.clone();
                        let cwd2 = cwd2.clone();
                        let remote_id = remote_id.clone();
                        let root = root.clone();
                        let lang = lang3.clone();
                        spawn(async move {
                            // used only on web/desktop (the JS bridge below)
                            let _ = &uploading;
                            #[cfg(any(feature = "web", feature = "desktop"))]
                            {
                                let payload = dioxus::document::eval("return window.__vdDropTree || null;")
                                    .join::<serde_json::Value>()
                                    .await
                                    .ok()
                                    .unwrap_or(serde_json::Value::Null);
                                let _ = dioxus::document::eval("window.__vdDropTree = null;").await;
                                let dir = payload
                                    .get("dir")
                                    .and_then(|v| v.as_str())
                                    .unwrap_or("")
                                    .to_string();
                                let files = payload
                                    .get("files")
                                    .and_then(|v| v.as_array().cloned())
                                    .unwrap_or_default();
                                let total = files.len();
                                if total == 0 { return; }
                                uploading.set(true);
                                let tok = token(&state);
                                let mut failed = 0usize;
                                for f in files {
                                    let name = f.get("name").and_then(|v| v.as_str()).unwrap_or("file").to_string();
                                    let b64 = f.get("b64").and_then(|v| v.as_str()).unwrap_or("").to_string();
                                    if b64.is_empty() { failed += 1; continue; }
                                    let safe: String = name.chars().map(|c| if c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-') { c } else { '_' }).collect();
                                    let target = if dir.is_empty() { safe } else { format!("{dir}/{safe}") };
                                    if crate::fsgit::api_write_binary(tok.clone(), target, b64, remote_id.clone(), root.clone()).await.is_err() {
                                        failed += 1;
                                    }
                                }
                                // reload the target directory listing
                                match crate::fsgit::api_list_dir(tok.clone(), dir.clone(), remote_id.clone(), root.clone()).await {
                                    Ok(v) => {
                                        if dir.is_empty() || dir == *cwd2.read() {
                                            entries.set(v.clone());
                                        }
                                        children.write().insert(dir.clone(), v);
                                    }
                                    Err(e) => files_msg.set(e.to_string()),
                                }
                                uploading.set(false);
                                if failed > 0 {
                                    files_msg.set(loc2(&lang, &format!("Nie wgrano {failed}/{total} plików"), &format!("{failed}/{total} files not uploaded")));
                                }
                            }
                        });
                    }
                },
            }
            div { class: "filetree-root", style: "display:flex;gap:2px;flex-wrap:wrap;",
                // drop on the root crumb uploads into the displayed directory
                "data-path": "{cur}",
                {crumb_els.iter().cloned()}
            }
            {msg_el}
            div { class: "filetree-body",
                {row_els.iter().cloned()}
                {empty_el}
            }
            {uploading_el}
            {ctx_el}
        }
    }
}

#[allow(clippy::too_many_arguments)]
/// Right-click target in the dock file tree.
#[derive(Clone, Debug, PartialEq)]
pub struct FileCtx {
    pub x: f64,
    pub y: f64,
    pub path: String,
    pub name: String,
    pub is_dir: bool,
}

fn file_row(
    state: Signal<ClientState>,
    e: &FileEntry,
    depth: usize,
    remote_id: Option<String>,
    root: String,
    children: Signal<std::collections::HashMap<String, Vec<FileEntry>>>,
    expanded: Signal<Vec<String>>,
    files_msg: Signal<String>,
    tabs: Signal<Vec<PreviewTab>>,
    active: Signal<Option<u64>>,
    next_id: Signal<u64>,
    file_ctx: Signal<Option<FileCtx>>,
) -> Element {
    let name = e.name.clone();
    let path = e.path.clone();
    let is_dir = e.is_dir;
    let size = fmt_size(e.size);
    let is_open = expanded.read().iter().any(|p| p == &path);
    let icon = if is_dir {
        if is_open { "📂" } else { "📁" }
    } else {
        "📄"
    };
    let chevron = if is_dir {
        if is_open { "▾" } else { "▸" }
    } else {
        " "
    };
    rsx! {
        div {
            class: if is_open { "filetree-item open" } else { "filetree-item" },
            style: "padding-left:{depth * 12 + 4}px;",
            title: "{path}",
            // drop target for the file-tree drag & drop upload (DROP_TREE_JS)
            "data-path": "{path}",
            "data-isdir": if is_dir { "1" } else { "0" },
            oncontextmenu: {
                let mut file_ctx = file_ctx.clone();
                let path = path.clone();
                let name = name.clone();
                move |e| {
                    e.prevent_default();
                    let p = e.data().client_coordinates();
                    file_ctx.set(Some(FileCtx { x: p.x, y: p.y, path: path.clone(), name: name.clone(), is_dir }));
                }
            },
            onclick: {
                let state = state.clone();
                let mut expanded = expanded.clone();
                let tabs = tabs.clone();
                let active = active.clone();
                let next_id = next_id.clone();
                let name = name.clone();
                let path = path.clone();
                let remote_id = remote_id.clone();
                let root = root.clone();
                move |_| {
                    if is_dir {
                        // Z22: a directory expands in place — the tree is
                        // never replaced by a listing of the target folder.
                        if expanded.read().iter().any(|p| p == &path) {
                            expanded.write().retain(|p| p != &path);
                            return;
                        }
                        expanded.write().push(path.clone());
                        if children.read().contains_key(&path) {
                            return;
                        }
                        let dir = path.clone();
                        let key = path.clone();
                        let rid = remote_id.clone();
                        let root = root.clone();
                        let mut children = children;
                        let mut files_msg = files_msg.clone();
                        spawn(async move {
                            match api_list_dir(token(&state), dir, rid, root).await {
                                Ok(v) => {
                                    children.write().insert(key, v);
                                }
                                Err(e) => files_msg.set(e.to_string()),
                            }
                        });
                    } else {
                        open_file_tab(
                            state,
                            tabs,
                            active,
                            next_id,
                            path.clone(),
                            name.clone(),
                            remote_id.clone(),
                            root.clone(),
                        );
                    }
                }
            },
            span { class: "filetree-chevron", "{chevron}" }
            span { "{icon}" }
            span { "{name}" }
            span { style: "color:var(--text-dim);font-size:10px;margin-left:auto;",
                if is_dir { "" } else { "{size}" }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// File preview tab
// ---------------------------------------------------------------------------

#[component]
fn FilePreviewTab(
    state: Signal<ClientState>,
    lang: String,
    tab: PreviewTab,
    tabs: Signal<Vec<PreviewTab>>,
    active: Signal<Option<u64>>,
) -> Element {
    let name = tab.name.clone();
    let path_lbl = tab.path.clone();
    let download_lbl = if lang == "pl" {
        "Pobierz".to_string()
    } else {
        "Download".to_string()
    };
    let size = tab
        .data
        .as_ref()
        .map(|d| fmt_size(d.file_size))
        .unwrap_or_else(|| "…".to_string());

    // editor state — keyed per tab so each file keeps its own state
    let mut editing = use_signal(|| false);
    let mut content = use_signal(String::new);
    let saving = use_signal(|| false);
    let saved = use_signal(|| false);
    let save_err = use_signal(String::new);
    let active_sheet = use_signal(|| 0usize);
    // Round 4 (C5): one-shot kick when the tab was opened via "Edit".
    let mut edit_kick = use_signal(|| false);
    // Round 5 (R3): word-wrap toggle in the editor, persisted per install.
    // Round 6 (R1): the read must be one-shot — an unguarded spawn in the
    // component body re-rendered forever (signal set -> render -> spawn)
    // and froze the app the moment edit mode opened.
    let mut wrap = use_signal(|| false);
    // round 12: render caches — recomputing md html / syntax highlight on
    // every parent re-render made md previews slow and edit-mode entry freeze
    let md_html_cache = use_signal(|| (String::new(), String::new()));
    let code_html_cache = use_signal(|| (String::new(), String::new()));
    let mut wrap_init = use_signal(|| false);
    if !*wrap_init.read() {
        wrap_init.set(true);
        let mut w = wrap.clone();
        spawn(async move {
            #[cfg(any(feature = "web", feature = "desktop"))]
            {
                if let Ok(v) = dioxus::document::eval(
                    "return (localStorage.getItem('vdEditorWrap') || '0');",
                )
                .join::<String>()
                .await
                {
                    w.set(v == "1");
                }
            }
        });
    }

    let is_editing = *editing.read();
    let is_saving = *saving.read();
    let is_saved = *saved.read();
    let err_txt = save_err.read().clone();

    let body_el: Element = match tab.data.as_ref() {
        None => rsx! { div { class: "preview-loading", "{tr(&lang, \"previewLoading\")}" } },
        Some(data) => {
            // Round 4 (C5): "Edit" from the context menu opens the editor
            // immediately (once — never clobber later typing).
            if tab.start_edit && !*edit_kick.read() {
                edit_kick.set(true);
                editing.set(true);
                content.set(data.text.clone());
                let mut v = tabs.write();
                if let Some(t) = v.iter_mut().find(|t| t.id == tab.id) {
                    t.start_edit = false;
                }
            }
            let dirty = is_editing && *content.read() != data.text;
            let is_editing_c = is_editing;
            let data = data.clone();
            let inner: Element = if is_editing_c {
                rsx! {
                    div { class: "preview-edit",
                        CodeEditor { code: content, id: "pv".to_string(), file: Some(name.clone()), wrap: Some(*wrap.read()) }
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
                    "zip" => {
                        if data.zip_entries.is_empty() {
                            rsx! { div { class: "preview-loading", "…" } }
                        } else {
                            let zip_count = data.zip_entries.len();
                            let zip_els: Vec<Element> = data
                                .zip_entries
                                .iter()
                                .map(|e| {
                                    let title = e.name.clone();
                                    let base = e
                                        .name
                                        .rsplit('/')
                                        .next()
                                        .unwrap_or(&e.name)
                                        .to_string();
                                    let dir_part = match e.name.rfind('/') {
                                        Some(pos) => e.name[..pos].to_string(),
                                        None => String::new(),
                                    };
                                    let has_dir = !dir_part.is_empty();
                                    let size_kb = format!("{:.1} KB", e.size as f64 / 1024.0);
                                    rsx! {
                                        div { class: "zip-entry", title: "{title}",
                                            span { class: "zip-entry-name",
                                                "{base}"
                                                if has_dir {
                                                    span { class: "zip-entry-dir", " — {dir_part}" }
                                                }
                                            }
                                            span { class: "zip-entry-size", "{size_kb}" }
                                        }
                                    }
                                })
                                .collect();
                            rsx! {
                                div { class: "preview-zip",
                                    div { class: "zip-count", "{zip_count} entries" }
                                    {zip_els.iter().cloned()}
                                }
                            }
                        }
                    }

                    "markdown" => {
                        // round 12: cache the rendered html per source text —
                        // re-rendering the whole markdown on every parent
                        // render made opening/switching md previews crawl
                        let html = {
                            let mut cache = md_html_cache.clone();
                            let c = cache.read();
                            if c.0 == data.text {
                                c.1.clone()
                            } else {
                                drop(c);
                                let h = crate::md::to_html(&data.text);
                                cache.set((data.text.clone(), h.clone()));
                                h
                            }
                        };
                        rsx! { div { class: "preview-markdown", dangerous_inner_html: html } }
                    }
                    "csv" => sheet_table(&crate::fsgit::SheetData { name: String::new(), rows: parse_csv(&data.text), styles: Vec::new(), merges: Vec::new(), col_widths: Vec::new() }),
                    "xlsx" => {
                        if data.sheets.is_empty() {
                            rsx! { div { class: "preview-loading", "{tr(&lang, \"previewLoading\")}" } }
                        } else {
                            let idx = (*active_sheet.read()).min(data.sheets.len() - 1);
                            let tab_el_tabs: Vec<Element> = data
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
                            let sheet = data.sheets[idx].clone();
                            rsx! {
                                div { class: "preview-sheet",
                                    div { class: "sheet-tabs", {tab_el_tabs.iter().cloned()} }
                                    {sheet_table(&sheet)}
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
                        let html = {
                            let mut cache = code_html_cache.clone();
                            let c = cache.read();
                            if c.0 == data.text {
                                c.1.clone()
                            } else {
                                drop(c);
                                let h = crate::highlight::highlight_auto(&name, &data.text);
                                cache.set((data.text.clone(), h.clone()));
                                h
                            }
                        };
                        // round 7 (R1): the preview honours the same wrap toggle
                        let cv_cls = if *wrap.read() { "preview-text code-view wrap" } else { "preview-text code-view" };
                        rsx! { pre { class: "{cv_cls}", dangerous_inner_html: html } }
                    }
                }
            };

            // toolbar (buttons need `dirty`, computed here in Rust)
            let data_text = data.text.clone();
            let edit_btn: Element = if data.editable {
                if is_editing_c {
                    // Round 7 (R2): in edit mode the button becomes
                    // "back to preview" with its own icon.
                    let mut editing = editing.clone();
                    rsx! {
                        button {
                            class: "file-preview-btn",
                            title: "{tr(&lang, \"previewBack\")}",
                            onclick: move |_| editing.set(false),
                            svg {
                                width: "14", height: "14", view_box: "0 0 24 24",
                                fill: "none", stroke: "currentColor",
                                "stroke-width": "2", "stroke-linecap": "round", "stroke-linejoin": "round",
                                path { d: "M1 12s4-8 11-8 11 8 11 8-4 8-11 8-11-8-11-8z" }
                                circle { cx: "12", cy: "12", r: "3" }
                            }
                        }
                    }
                } else {
                    let mut editing = editing.clone();
                    let mut content = content.clone();
                    rsx! {
                        button {
                            class: "file-preview-btn",
                            title: "{tr(&lang, \"previewEdit\")}",
                            onclick: move |_| {
                                content.set(data_text.clone());
                                editing.set(true);
                            },
                            "✎"
                        }
                    }
                }
            } else {
                VNode::empty()
            };

            let save_btn: Element = if is_editing_c {
                rsx! {
                    button {
                        class: if dirty { "file-preview-btn primary" } else { "file-preview-btn" },
                        title: if is_saving { "{tr(&lang, \"previewSaving\")}" } else { "{tr(&lang, \"previewSave\")}" },
                        disabled: !dirty || is_saving,
                        onclick: {
                            let st = state.clone();
                            let p = tab.path.clone();
                            let tid = tab.id;
                            let content = content.clone();
                            let tabs2 = tabs.clone();
                            let rid = tab.remote_id.clone();
                            let root = tab.root.clone();
                            // FnMut body: re-clone per click before the move
                            // into the async task (E0507 otherwise)
                            move |_| {
                                let c = content.read().clone();
                                let rid = rid.clone();
                                let root = root.clone();
                                let mut saving = saving.clone();
                                saving.set(true);
                                let st = st.clone();
                                let p = p.clone();
                                let mut saved = saved.clone();
                                let mut save_err = save_err.clone();
                                let mut saving2 = saving.clone();
                                let mut tabs3 = tabs2.clone();
                                spawn(async move {
                                    match api_save_file(token(&st), p.clone(), c.clone(), rid, root).await {
                                        Ok(()) => {
                                            saved.set(true);
                                            save_err.set(String::new());
                                            let mut v = tabs3.write();
                                            if let Some(t) = v.iter_mut().find(|t| t.id == tid) {
                                                if let Some(d) = t.data.as_mut() {
                                                    d.text = c.clone();
                                                }
                                            }
                                        }
                                        Err(e) => {
                                            saved.set(false);
                                            save_err.set(e.to_string());
                                        }
                                    }
                                    saving2.set(false);
                                });
                            }
                        },
                        svg {
                            width: "14", height: "14", view_box: "0 0 24 24",
                            fill: "none", stroke: "currentColor",
                            "stroke-width": "2", "stroke-linecap": "round", "stroke-linejoin": "round",
                            path { d: "M19 21H5a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h11l5 5v11a2 2 0 0 1-2 2z" }
                            polyline { points: "17 21 17 13 7 13 7 21" }
                            polyline { points: "7 3 7 8 15 8" }
                        }
                    }
                }
            } else {
                VNode::empty()
            };

            let save_status: Element = if is_editing_c {
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

            let tabs_c = tabs.clone();
            let active_c = active.clone();
            let tid = tab.id;
            rsx! {
                div { class: "file-preview",
                    div { class: "file-preview-header",
                        div { class: "file-preview-title",
                            span { class: "file-preview-name", "{name}" }
                            span { class: "file-preview-size", "{size}" }
                        }
                        // Z21: the full path (stable layout) - the row is the
                        // title block, actions stay on the right.
                        span { class: "file-preview-path", title: "{path_lbl}", "{path_lbl}" }
                        div { class: "file-preview-actions",
                            {save_status}
                            // Round 12: the wrap toggle belongs to the EDITOR —
                            // hidden in plain preview mode.
                            if is_editing_c {
                            button {
                                class: if *wrap.read() { "file-preview-btn active" } else { "file-preview-btn" },
                                title: "{loc2(&lang, \"Zawijaj wiersze\", \"Word wrap\")}",
                                onclick: move |_| {
                                    let next = !*wrap.read();
                                    wrap.set(next);
                                    #[cfg(any(feature = "web", feature = "desktop"))]
                                    spawn(async move {
                                        let _ = dioxus::document::eval(&format!(
                                            "try{{localStorage.setItem('vdEditorWrap','{}');}}catch(e){{}}",
                                            if next { "1" } else { "0" }
                                        )).await;
                                        // round 8: re-position the gutter numbers
                                        // immediately after the wrap flip (the
                                        // editor effect only tracks content).
                                        let _ = dioxus::document::eval(
                                            "requestAnimationFrame(function(){window.vdEdSync && window.vdEdSync('pv');});",
                                        ).await;
                                    });
                                },
                                svg {
                                    width: "14", height: "14", view_box: "0 0 24 24",
                                    fill: "none", stroke: "currentColor",
                                    "stroke-width": "2", "stroke-linecap": "round", "stroke-linejoin": "round",
                                    path { d: "M4 6h16" }
                                    path { d: "M4 12h10" }
                                    path { d: "M4 18h7" }
                                    path { d: "M17 15l3 3-3 3" }
                                }
                            }
                            }
                            {edit_btn}
                            {save_btn}
                            // Z21: download a copy (works in the browser and
                            // in the desktop webview — blob URL + anchor).
                            {
                                let dl_path = path_lbl.clone();
                                let dl_rid = tab.remote_id.clone();
                                let dl_root = tab.root.clone();
                                let dl_name = name.clone();
                                let st_dl = state.clone();
                                rsx! {
                                    button {
                                        class: "file-preview-btn",
                                        title: "{download_lbl}",
                                        onclick: move |_| {
                                            let st = st_dl.clone();
                                            let p = dl_path.clone();
                                            let rid = dl_rid.clone();
                                            let root = dl_root.clone();
                                            let fname = dl_name.clone();
                                            spawn(async move {
                                                match crate::fsgit::api_download_base64(
                                                    token(&st), p, rid, root,
                                                )
                                                .await
                                                {
                                                    Ok(b64) => {
                                                        let js = format!(
                                                            "try{{var b=atob('{}');var u=new Uint8Array(b.length);for(var i=0;i<b.length;i++){{u[i]=b.charCodeAt(i);}}var a=document.createElement('a');a.href=URL.createObjectURL(new Blob([u]));a.download='{}';a.click();}}catch(e){{}}",
                                                            b64.replace('\'', ""),
                                                            fname.replace('\'', "")
                                                        );
                                                        dioxus::document::eval(js.as_str());
                                                    }
                                                    Err(_) => {}
                                                }
                                            });
                                        },
                                        "⬇"
                                    }
                                }
                            }
                            if crate::state::is_desktop() {
                                {
                                    let open_path = path_lbl.clone();
                                    let st_open = state.clone();
                                    rsx! {
                                        button {
                                            class: "file-preview-btn",
                                            title: "{tr(&lang, \"previewOpenExternally\")}",
                                            onclick: move |_| {
                                                let st = st_open.clone();
                                                let p = open_path.clone();
                                                spawn(async move {
                                                    let _ = crate::fsgit::api_open_externally(token(&st), p).await;
                                                });
                                            },
                                            "↗"
                                        }
                                    }
                                }
                            }
                            button {
                                class: "file-preview-btn",
                                onclick: move |_| close_tab(tabs_c, active_c, tid),
                                "✕"
                            }
                        }
                    }
                    div { class: "file-preview-body",
                        {inner}
                    }
                }
            }
        }
    };

    body_el
}

/// Render one sheet. `styles` (round 4, C4) carries per-cell inline CSS;
/// `merges`/`col_widths` (round 6, R11) keep merged header cells and the
/// real column widths.
fn sheet_table(sheet: &crate::fsgit::SheetData) -> Element {
    let rows = &sheet.rows;
    let styles = &sheet.styles;
    // covered-by-merge lookup: (row, col) -> anchor (r1, c1) or None
    let mut covered: std::collections::HashMap<(usize, usize), (usize, usize)> = std::collections::HashMap::new();
    let mut anchor_span: std::collections::HashMap<(usize, usize), (usize, usize)> = std::collections::HashMap::new();
    for m in &sheet.merges {
        let (r1, c1, r2, c2) = (m[0], m[1], m[2], m[3]);
        anchor_span.insert((r1, c1), (c2 - c1 + 1, r2 - r1 + 1));
        for r in r1..=r2 {
            for c in c1..=c2 {
                if (r, c) != (r1, c1) {
                    covered.insert((r, c), (r1, c1));
                }
            }
        }
    }
    let row_els: Vec<Element> = rows
        .iter()
        .enumerate()
        .map(|(ri, row)| {
            let cells: Vec<Element> = row
                .iter()
                .enumerate()
                .filter(|(ci, _)| !covered.contains_key(&(ri, *ci)))
                .map(|(ci, c)| {
                    let extra = styles
                        .get(ri)
                        .and_then(|r| r.get(ci))
                        .cloned()
                        .flatten()
                        .unwrap_or_default();
                    let (colspan, rowspan) = anchor_span.get(&(ri, ci)).copied().unwrap_or((1, 1));
                    if ri == 0 {
                        rsx! { th { class: "sheet-header-cell", colspan: "{colspan}", rowspan: "{rowspan}", style: "border:1px solid var(--border);padding:4px 8px;text-align:left;font-weight:600;{extra}", "{c}" } }
                    } else {
                        rsx! { td { colspan: "{colspan}", rowspan: "{rowspan}", style: "border:1px solid var(--border);padding:4px 8px;{extra}", "{c}" } }
                    }
                })
                .collect();
            let zebra = if ri % 2 == 0 { "sheet-row-even" } else { "sheet-row-odd" };
            rsx! { tr { class: "{zebra}", {cells.iter().cloned()} } }
        })
        .collect();
    let col_els: Vec<Element> = sheet
        .col_widths
        .iter()
        .enumerate()
        .filter(|(_, w)| **w > 0.0)
        .map(|(i, w)| {
            // character width ≈ 7px + padding
            let px = (*w * 7.0 + 16.0).round() as u32;
            rsx! { col { key: "c{i}", style: "width:{px}px;" } }
        })
        .collect();
    rsx! {
        div { class: "preview-sheet",
            div { class: "sheet-table-wrap",
                table { class: "sheet-table",
                    if !col_els.is_empty() {
                        colgroup { {col_els.iter().cloned()} }
                    }
                    {row_els.iter().cloned()}
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Diff preview tab
// ---------------------------------------------------------------------------

#[component]
fn DiffPanel(
    lang: String,
    tab: PreviewTab,
    tabs: Signal<Vec<PreviewTab>>,
    active: Signal<Option<u64>>,
) -> Element {
    if !tab.loaded {
        return rsx! { div { class: "preview-loading", "{tr(&lang, \"previewLoading\")}" } };
    }

    let line_els: Vec<Element> = tab
        .diff
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

    let tabs_c = tabs.clone();
    let active_c = active.clone();
    let tid = tab.id;
    let path = tab.path.clone();
    rsx! {
        div { class: "file-preview",
            div { class: "file-preview-header",
                div { class: "file-preview-title",
                    span { class: "file-preview-name", "{path}" }
                }
                button {
                    class: "file-preview-btn",
                    onclick: move |_| close_tab(tabs_c, active_c, tid),
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

// ---------------------------------------------------------------------------
// Git panel
// ---------------------------------------------------------------------------

#[allow(clippy::too_many_arguments)]
fn git_panel(
    state: Signal<ClientState>,
    lang: String,
    git: Signal<Option<GitStatus>>,
    git_log: Signal<Vec<GitCommit>>,
    commit_msg: Signal<String>,
    git_busy: Signal<bool>,
    git_error: Signal<String>,
    tabs: Signal<Vec<PreviewTab>>,
    active: Signal<Option<u64>>,
    next_id: Signal<u64>,
) -> Element {
    let st = git.read().clone();
    let log = git_log.read().clone();
    let busy = *git_busy.read();
    let msg_val = commit_msg.read().clone();
    let extra_err = git_error.read().clone();

    // Nexus: null status -> loading placeholder with a lone refresh button
    let Some(status) = st else {
        return rsx! {
            div { class: "git-panel",
                div { class: "git-toolbar",
                    button {
                        class: "git-refresh-btn",
                        title: "{tr(&lang, \"gitRefresh\")}",
                        onclick: move |_| {
                            spawn(git_refresh(state, git, git_log));
                        },
                        "⟳"
                    }
                }
                div { class: "dock-empty", "{tr(&lang, \"gitLoading\")}" }
            }
        };
    };

    // Nexus: !repo -> "Not a git repository" (no error banner)
    if !status.repo {
        return rsx! { div { class: "dock-empty", "{tr(&lang, \"notAGitRepo\")}" } };
    }

    let entries = status.entries.clone();
    let entries_len = entries.len();
    let has_unstaged = entries.iter().any(|e| !e.staged);

    // linked worktrees (workspace/git/worktrees/list) — click reveals in Files
    let worktrees: Vec<serde_json::Value> = state
        .read()
        .worktrees
        .as_ref()
        .and_then(|(_, j)| serde_json::from_str::<Vec<serde_json::Value>>(j).ok())
        .unwrap_or_default();
    let wt_els: Vec<Element> = worktrees
        .iter()
        .map(|w| {
            let state = state.clone();
            let name = w.get("name").and_then(|x| x.as_str()).unwrap_or("").to_string();
            let branch = w.get("branch").and_then(|x| x.as_str()).unwrap_or("").to_string();
            let wcwd = w.get("cwd").and_then(|x| x.as_str()).unwrap_or("").to_string();
            let changes = w.get("branchChanges").and_then(|c| {
                let a = c.get("additions").and_then(|x| x.as_i64()).unwrap_or(0);
                let d = c.get("deletions").and_then(|x| x.as_i64()).unwrap_or(0);
                if a + d > 0 { Some(format!("+{a}/-{d}")) } else { None }
            });
            rsx! {
                button {
                    class: "ctx-menu-item worktree-row",
                    title: "{wcwd}",
                    onclick: move |_| {
                        let state = state.clone();
                        let p = wcwd.clone();
                        spawn(async move {
                            crate::state::open_path(state, &p).await;
                        });
                    },
                    "🌿 {name} ({branch})"
                    if let Some(c) = &changes {
                        span { class: "session-badge", " {c}" }
                    }
                }
            }
        })
        .collect();

    // error banner: action error wins over the status error (Nexus
    // `{gitError ?? status.error}`)
    let shown_err = if extra_err.is_empty() {
        status.error.clone()
    } else {
        extra_err.clone()
    };
    let error_el: Element = if shown_err.is_empty() {
        VNode::empty()
    } else {
        rsx! { div { class: "git-error", "{shown_err}" } }
    };

    let ab_el: Element = if status.ahead > 0 || status.behind > 0 {
        rsx! { span { class: "git-ahead-behind", "↑{status.ahead} ↓{status.behind}" } }
    } else {
        VNode::empty()
    };

    let push_el: Element = if status.ahead > 0 {
        rsx! {
            button {
                class: "git-stage-all-btn",
                title: "{tr(&lang, \"pushHint\")}",
                disabled: busy,
                onclick: move |_| {
                    spawn(git_push_action(state, git, git_log, git_error, git_busy));
                },
                "{tr(&lang, \"pushLabel\")} ↑{status.ahead}"
            }
        }
    } else {
        VNode::empty()
    };

    let stage_all_el: Element = if has_unstaged {
        let unstaged: Vec<String> = entries
            .iter()
            .filter(|e| !e.staged)
            .map(|e| e.path.clone())
            .collect();
        rsx! {
            button {
                class: "git-stage-all-btn",
                title: "{tr(&lang, \"stageLabel\")}",
                onclick: move |_| {
                    spawn(git_stage_action(state, git, git_log, git_error, unstaged.clone(), false));
                },
                "+ {tr(&lang, \"stageLabel\")}"
            }
        }
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
            let btn_title = if staged {
                tr(&lang, "unstageLabel").to_string()
            } else {
                tr(&lang, "stageLabel").to_string()
            };
            // dedicated clones — move closures take ownership of `path`
            let row_title = path.clone();
            let diff_path = path.clone();
            let stage_path = path;
            rsx! {
                div { class: "git-file-row",
                    button {
                        class: "git-file",
                        title: "{row_title}",
                        onclick: move |_| {
                            open_diff_tab(state, tabs, active, next_id, diff_path.clone(), staged);
                        },
                        span { class: "{cls}", "{code}" }
                        span { class: "git-file-name", "{dir}" }
                        span { class: "git-file-dir", "{dir_part}" }
                    }
                    button {
                        class: "git-stage-btn",
                        title: "{btn_title}",
                        onclick: move |_| {
                            spawn(git_stage_action(
                                state,
                                git,
                                git_log,
                                git_error,
                                vec![stage_path.clone()],
                                staged,
                            ));
                        },
                        {if staged { "−" } else { "+" }}
                    }
                }
            }
        })
        .collect();

    let changes_el: Element = if entries.is_empty() {
        rsx! { div { class: "git-empty", "{tr(&lang, \"gitNoChanges\")}" } }
    } else {
        rsx! { {entry_els.iter().cloned()} }
    };

    let log_els: Vec<Element> = log
        .iter()
        .map(|c| {
            let author_first = c.author.split(' ').next().unwrap_or("").to_string();
            // ISO date -> "MM-DD" (Nexus toLocaleDateString 2-digit)
            let mmdd = if c.date.len() >= 10 {
                format!("{}-{}", &c.date[5..7], &c.date[8..10])
            } else {
                c.date.clone()
            };
            let row_title = format!("{}\n{}", c.hash, c.author);
            rsx! {
                div { class: "git-commit", title: "{row_title}",
                    span { class: "git-commit-hash", "{c.short_hash}" }
                    span { class: "git-commit-msg", "{c.message}" }
                    span { class: "git-commit-meta", "{author_first} · {mmdd}" }
                }
            }
        })
        .collect();

    let history_el: Element = if log.is_empty() {
        rsx! { div { class: "git-empty", "{tr(&lang, \"gitNoCommits\")}" } }
    } else {
        rsx! { {log_els.iter().cloned()} }
    };

    let commit_disabled = busy || msg_val.trim().is_empty();

    rsx! {
        div { class: "git-panel",
            div { class: "git-toolbar",
                span { class: "git-branch", title: "{status.branch}",
                    span { class: "git-branch-icon", "⎇" }
                    "{status.branch}"
                    {ab_el}
                }
                {push_el}
                button {
                    class: "git-refresh-btn",
                    title: "{tr(&lang, \"gitRefresh\")}",
                    onclick: move |_| {
                        spawn(git_refresh(state, git, git_log));
                    },
                    {if busy { "…" } else { "⟳" }}
                }
            }
            {error_el}
            div { class: "git-section-label",
                "{tr(&lang, \"gitChanges\")} ({entries_len})"
                {stage_all_el}
            }
            div { class: "git-files",
                {changes_el}
            }
            div { class: "git-commit-box",
                input {
                    class: "git-commit-input",
                    r#type: "text",
                    value: "{msg_val}",
                    placeholder: "{tr(&lang, \"commitMessagePlaceholder\")}",
                    oninput: {
                        let mut commit_msg = commit_msg.clone();
                        let mut git_error = git_error.clone();
                        move |e| {
                            commit_msg.set(e.value());
                            git_error.set(String::new());
                        }
                    },
                    onkeydown: move |e: Event<KeyboardData>| {
                        if e.key() == Key::Enter {
                            spawn(git_commit_action(state, git, git_log, git_error, commit_msg, git_busy));
                        }
                    },
                }
                button {
                    class: "git-commit-btn",
                    disabled: commit_disabled,
                    onclick: move |_| {
                        spawn(git_commit_action(state, git, git_log, git_error, commit_msg, git_busy));
                    },
                    "{tr(&lang, \"commitLabel\")}"
                }
            }
            div { class: "git-section-label", "{tr(&lang, \"gitHistory\")}" }
            div { class: "git-log",
                {history_el}
            }
            if !wt_els.is_empty() {
                div { class: "git-section-label", "Worktrees" }
                div { class: "git-log",
                    {wt_els.iter().cloned()}
                }
            }
        }
    }
}
