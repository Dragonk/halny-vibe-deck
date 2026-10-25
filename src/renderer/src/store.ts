import { create } from "zustand";
import type {
  Session,
  Message,
  AppView,
  AcpStatus,
  AppConfig,
  ToolCall,
  ThinkingBlock,
  ThemeMode,
  ConfigOption,
  UsageStats,
  DockTabKind,
  VibeSessionMessage,
  VibeSession,
  PreviewTab,
} from "./types";
import { defaultConfig } from "./types";
import type { RemoteConnection, RemoteStatus, RemoteSessionInfo } from "./types";

/** ACP history message (from SessionHistory event) */
export interface AcpHistoryToolCall {
  id: string;
  title: string;
  kind: string;
  status: "running" | "done" | "error";
  input?: Record<string, unknown>;
  output?: string;
  diffs?: Array<{ path: string; oldText: string | null; newText: string }>;
}

export interface AcpHistoryMessage {
  role: "user" | "ai";
  content: string;
  thinking?: Array<{ id: string; content: string }>;
  toolCalls?: AcpHistoryToolCall[];
}

function uuid(): string {
  return crypto.randomUUID();
}

const BS = String.fromCharCode(92);

/** Normalize a path to forward slashes (Windows paths arrive with backslashes). */
function normPath(p: string): string {
  return p.split(BS).join("/");
}

/** v0.7.0: file-tree key for a directory, namespaced per remote connection. */
export function fileTreeKey(dir: string, remoteId?: string | null): string {
  return remoteId ? "r:" + remoteId + ":" + dir : dir;
}

interface AppStore {
  // Config
  config: AppConfig;
  setConfig: (config: AppConfig) => void;
  addRecentFolder: (folder: string) => void;

  // Theme
  theme: ThemeMode;
  toggleTheme: () => void;

  // View
  currentView: AppView;
  setCurrentView: (view: AppView) => void;

  // Sessions
  sessions: Session[];
  currentSessionId: string | null;
  currentSession: () => Session | null;
  setCurrentSessionId: (id: string | null) => void;
  pendingNewSession: boolean;
  setPendingNewSession: (v: boolean) => void;
  sessionStarting: boolean;
  setSessionStarting: (v: boolean) => void;
  addSession: (workingDir?: string) => Session;
  updateSession: (id: string, updates: Partial<Session>) => void;
  /** ACP session currently loaded in the agent process */
  activeAcpSessionId: string | null;

  // ACP state
  acpStatus: AcpStatus;
  setAcpStatus: (status: AcpStatus) => void;
  acpError: string | null;
  setAcpError: (err: string | null) => void;

  streamingMessage: string | null;
  streamingSessions: string[];
  busySessions: string[];
  markSessionBusy: (sessionId: string) => void;
  markSessionIdle: (sessionId: string) => void;
  sessionLoadProgress: { acpSessionId: string; count: number } | null;
  setSessionLoadProgress: (progress: { acpSessionId: string; count: number } | null) => void;

  // Config options (model / mode / thinking) from ACP
  configOptions: ConfigOption[];
  setConfigOptions: (options: ConfigOption[]) => void;
  /** Update the stored currentValue of a single config option (keeps chips in sync). */
  patchConfigOption: (id: string, value: string) => void;
  /** Re-apply the model/mode/thinking recorded on a session after switching to it. */
  syncSessionChips: (localSessionId: string) => Promise<void>;
  currentModel: string;
  currentMode: string;
  currentThinking: string;
  setCurrentModel: (model: string) => void;
  setCurrentMode: (mode: string) => void;
  setCurrentThinking: (level: string) => void;
  availableModes: Array<{ id: string; name: string }>;
  setAvailableModes: (modes: Array<{ id: string; name: string }>) => void;
  availableCommands: Array<{ name: string; description: string; hint?: string }>;
  setAvailableCommands: (commands: Array<{ name: string; description: string; hint?: string }>) => void;

  // Live usage / cache stats per local session
  usage: Record<string, UsageStats>;
  setUsage: (sessionId: string, stats: UsageStats) => void;

  // Working directory
  workingDirectory: string;
  setWorkingDirectory: (dir: string) => void;

  sandboxEnabled: boolean;
  setSandboxEnabled: (v: boolean) => void;

  // Dock (right panel)
  dockOpen: boolean;
  setDockOpen: (v: boolean) => void;
  ensureDockOpen: () => void;
  /** v0.6.0: mobile (narrow viewport) sidebar drawer */
  mobileSidebarOpen: boolean;
  setMobileSidebarOpen: (v: boolean) => void;
  dockWidth: number;
  setDockWidth: (w: number) => void;
  dockTab: DockTabKind;
  setDockTab: (tab: DockTabKind) => void;
  previewTabs: PreviewTab[];
  activePreviewId: string | null;
  openFilePreview: (path: string, name?: string, remoteId?: string | null) => void;
  openDiffPreview: (name: string, path: string, diff: string, remoteId?: string | null) => void;
  closePreviewTab: (id: string) => void;
  setActivePreview: (id: string) => void;
  openFilePreviewTab: (path: string, name?: string, remoteId?: string | null) => void;
  // File-tree state lives in the store so expansion survives dock tab switches.
  // v0.7.0: keys are namespaced per remote connection (fileTreeKey).
  fileTreeExpanded: Set<string>;
  fileTreeEntries: Record<string, Array<{ name: string; isDir: boolean }>>;
  loadFileTreeDir: (dir: string, remoteId?: string | null) => void;
  toggleFileTreeDir: (dir: string, remoteId?: string | null) => void;
  revealInFileTree: (dir: string, remoteId?: string | null) => void;
  hideSession: (key: string) => void;
  hideProject: (workingDirectory: string) => void;
  unhideSessions: (keys: string[]) => void;
  unhideProject: (workingDirectory: string) => void;
  reactivateHistorySession: (vibe: VibeSession) => Promise<void>;

  // Projects (sidebar grouping)
  collapsedProjects: string[];
  toggleProjectCollapse: (project: string) => void;

  // Message queue
  messageQueue: Array<{ id: string; content: string; timestamp: number }>;
  addToMessageQueue: (content: string) => void;
  removeFromMessageQueue: (id: string) => void;
  updateQueueMessage: (id: string, content: string) => void;
  clearMessageQueue: () => void;
  getNextQueuedMessage: () => { id: string; content: string } | undefined;

  currentSessionWorkingDirectory: () => string;

  // Actions
  addUserMessage: (text: string, attachments?: { path: string; isDir: boolean; content: string }[]) => Message;
  startAiMessage: (sessionId: string) => string;
  appendChunk: (sessionId: string, messageId: string, chunk: string) => void;
  finalizeAiMessage: (sessionId: string, messageId: string) => void;
  updateToolCall: (sessionId: string, toolCall: ToolCall) => void;
  mergeToolCall: (sessionId: string, toolCallId: string, updates: Partial<ToolCall>) => void;
  startThinking: (sessionId: string, messageId: string, blockId: string) => void;
  appendThinkingChunk: (sessionId: string, messageId: string, blockId: string, chunk: string) => void;
  finalizeThinking: (sessionId: string, messageId: string, blockId: string) => void;
  linkAcpSession: (localSessionId: string, acpSessionId: string) => void;
  unlinkAcpSession: (localSessionId: string) => void;
  getSessionByAcpId: (acpSessionId: string) => Session | null;
  /** ACP session id we are switching to (guards against priming-session events) */
  pendingLoadSessionId: string | null;
  setPendingLoadSessionId: (id: string | null) => void;
  importAcpSessions: (
    acpSessions: Array<{ sessionId: string; title: string; cwd: string; updatedAt: string }>,
  ) => void;
  /** Real resume: replace the session transcript with the replayed history */
  applySessionHistory: (
    acpSessionId: string,
    history: AcpHistoryMessage[],
    title?: string,
  ) => void;
  loadAcpSession: (localSessionId: string) => Promise<void>;
  loadVibeSessionMessages: (localSessionId: string) => Promise<void>;
  fetchContextSummary: (localSessionId: string) => Promise<void>;
  // ---- v0.6.0/v0.7.0: remote SSH connections ----
  /** Latest reachability status per connection id */
  remoteStatuses: Record<string, RemoteStatus>;
  /**
   * v0.7.0: idempotently make sure the ACP worker for a connection (null =
   * the local agent) is running. Never primes a session - resume paths rely
   * on this so replaying a session does not create garbage sessions.
   */
  ensureAcpWorker: (remoteId: string | null, cwd?: string) => Promise<void>;
  /** Poll status + session lists for all enabled remote connections. */
  refreshRemoteData: () => Promise<void>;
  startRemoteSession: (remoteId: string, cwd: string) => Promise<void>;
  loadRemoteSession: (remoteId: string, info: RemoteSessionInfo) => Promise<void>;
  importRemoteSessions: (remoteId: string, infos: RemoteSessionInfo[]) => void;
}

export const useStore = create<AppStore>((set, get) => ({
  config: defaultConfig,
  setConfig: (config) => set({ config, theme: config.theme, dockOpen: config.dockOpen, dockWidth: config.dockWidth }),
  addRecentFolder: (folder) => {
    const cfg = get().config;
    const updated = [folder, ...cfg.recentFolders.filter((f) => f !== folder)].slice(0, 12);
    set({ config: { ...cfg, recentFolders: updated } });
    window.api.config.save({ ...cfg, recentFolders: updated }).catch(console.error);
  },

  theme: "auto",
  toggleTheme: () =>
    set((s) => {
      const order: ThemeMode[] = ["auto", "dark", "light"];
      const idx = order.indexOf(s.theme);
      const newTheme = order[(idx + 1) % order.length];
      const cfg = { ...s.config, theme: newTheme };
      window.api.config.save(cfg).catch(console.error);
      return { theme: newTheme, config: cfg };
    }),

  currentView: "chat",
  setCurrentView: (currentView) => set({ currentView }),

  sessions: [],
  currentSessionId: null,
  currentSession: () => {
    const { sessions, currentSessionId } = get();
    return sessions.find((s) => s.id === currentSessionId) ?? null;
  },
  setCurrentSessionId: (id) => set({ currentSessionId: id }),
  pendingNewSession: false,
  setPendingNewSession: (pendingNewSession) => set({ pendingNewSession }),
  sessionStarting: false,
  setSessionStarting: (sessionStarting) => set({ sessionStarting }),
  activeAcpSessionId: null,
  addSession: (workingDir) => {
    const { sessions, workingDirectory } = get();
    const session: Session = {
      id: uuid(),
      name: "Session " + (sessions.length + 1),
      messages: [],
      createdAt: Date.now(),
      lastActive: Date.now(),
      workingDirectory: workingDir || workingDirectory || undefined,
    };
    set((s) => ({
      sessions: [...s.sessions, session],
      currentSessionId: session.id,
    }));
    return session;
  },
  updateSession: (id, updates) => {
    set((s) => ({
      sessions: s.sessions.map((sess) => (sess.id === id ? { ...sess, ...updates } : sess)),
    }));
  },

  acpStatus: "disconnected",
  setAcpStatus: (acpStatus) => set({ acpStatus }),
  acpError: null,
  setAcpError: (acpError) => set({ acpError }),

  streamingMessage: null,
  streamingSessions: [],
  busySessions: [],
  mobileSidebarOpen: false,
  setMobileSidebarOpen: (v) => set({ mobileSidebarOpen: v }),
  markSessionBusy: (sessionId) =>
    set((s) => ({
      busySessions: s.busySessions.includes(sessionId) ? s.busySessions : [...s.busySessions, sessionId],
    })),
  markSessionIdle: (sessionId) =>
    set((s) => ({ busySessions: s.busySessions.filter((id) => id !== sessionId) })),
  sessionLoadProgress: null,
  setSessionLoadProgress: (progress) => set({ sessionLoadProgress: progress }),


  configOptions: [],
  setConfigOptions: (options) =>
    set(() => {
      const pick = (id: string): string => {
        const o = options.find((x) => x.id === id);
        return o && o.currentValue !== undefined && o.currentValue !== null
          ? String(o.currentValue)
          : "";
      };
      return {
        configOptions: options,
        currentModel: pick("model"),
        currentMode: pick("mode"),
        currentThinking: pick("thinking"),
      };
    }),

  patchConfigOption: (id, value) =>
    set((s) => ({
      configOptions: s.configOptions.map((o) => (o.id === id ? { ...o, currentValue: value } : o)),
    })),

  syncSessionChips: async (localSessionId) => {
    const session = get().sessions.find((s) => s.id === localSessionId);
    if (!session?.acpSessionId) return;
    const acpSessionId = session.acpSessionId;
    const chipRid = session.remoteId ?? null;
    let model = session.lastModel;
    let mode = session.lastMode;
    let thinking = session.lastThinking;
    if (!model || !mode || !thinking) {
      try {
        const last = await window.api.stats.vibeSessionLastConfig(acpSessionId);
        const patch: { lastModel?: string; lastMode?: string; lastThinking?: string } = {};
        if (!model && last?.model) {
          model = last.model;
          patch.lastModel = last.model;
        }
        if (!mode && last?.mode) {
          mode = last.mode;
          patch.lastMode = last.mode;
        }
        if (!thinking && last?.thinking) {
          thinking = last.thinking;
          patch.lastThinking = last.thinking;
        }
        if (Object.keys(patch).length > 0) get().updateSession(localSessionId, patch);
      } catch {
        /* vibe logs unavailable — keep the CLI-reported chips */
      }
    }
    const apply = (id: string, value: string | undefined, cur: string): void => {
      if (!value || value === cur) return;
      // A fallback value that is not part of the option list (e.g. a
      // custom agent name) must not corrupt the chip dropdown.
      const opt = get().configOptions.find((o) => o.id === id);
      if (opt && opt.options && opt.options.length > 0 && !opt.options.some((op) => op.value === value)) return;
      if (id === "model") get().setCurrentModel(value);
      if (id === "mode") get().setCurrentMode(value);
      if (id === "thinking") get().setCurrentThinking(value);
      get().patchConfigOption(id, value);
      window.api.acp.setConfigOption(id, value, acpSessionId, chipRid ?? undefined).catch(console.error);
    };
    apply("model", model, get().currentModel);
    apply("mode", mode, get().currentMode);
    apply("thinking", thinking, get().currentThinking);
  },

  currentModel: "",
  currentMode: "",
  currentThinking: "",
  setCurrentModel: (currentModel) => set({ currentModel }),
  setCurrentMode: (currentMode) => set({ currentMode }),
  setCurrentThinking: (currentThinking) => set({ currentThinking }),

  availableModes: [],
  setAvailableModes: (availableModes) => set({ availableModes }),
  availableCommands: [],
  setAvailableCommands: (availableCommands) => set({ availableCommands }),

  usage: {},
  setUsage: (sessionId, stats) => set((s) => ({ usage: { ...s.usage, [sessionId]: stats } })),

  workingDirectory: "",
  setWorkingDirectory: (workingDirectory) => {
    const cfg = get().config;
    set({ workingDirectory, config: { ...cfg, workingDirectory } });
    window.api.config.save({ ...cfg, workingDirectory }).catch(console.error);
  },

  sandboxEnabled: false,
  setSandboxEnabled: (sandboxEnabled) => {
    const cfg = get().config;
    set({ sandboxEnabled, config: { ...cfg, sandboxEnabled } });
    window.api.config.save({ ...cfg, sandboxEnabled }).catch(console.error);
  },

  dockOpen: defaultConfig.dockOpen,
  setDockOpen: (dockOpen) => {
    const cfg = get().config;
    set({ dockOpen, config: { ...cfg, dockOpen } });
    window.api.config.save({ ...cfg, dockOpen }).catch(console.error);
  },
  ensureDockOpen: () => {
    if (get().dockOpen) return;
    const cfg = get().config;
    set({ dockOpen: true, config: { ...cfg, dockOpen: true } });
    window.api.config.save({ ...cfg, dockOpen: true }).catch(console.error);
  },
  dockWidth: defaultConfig.dockWidth,
  setDockWidth: (dockWidth) => set({ dockWidth }),
  dockTab: "files",
  setDockTab: (dockTab) => set({ dockTab }),

  previewTabs: [],
  activePreviewId: null,
  openFilePreview: (path, name, remoteId) => {
    const rid = remoteId ?? null;
    // Directories reveal in the Files tab instead of opening a preview tab
    window.api.fs
      .stat(path, rid ?? undefined)
      .then((s) => {
        if (s.exists && s.isDir) get().revealInFileTree(path, rid);
        else if (!s.exists) {
          // stat can miss unusual paths; try listing before deciding it is a file
          window.api.fs
            .listDir(path, rid ?? undefined)
            .then((entries) => {
              if ((entries as Array<{ name: string; isDir: boolean }>).length > 0)
                get().revealInFileTree(path, rid);
              else get().openFilePreviewTab(path, name, rid);
            })
            .catch(() => get().openFilePreviewTab(path, name, rid));
        } else get().openFilePreviewTab(path, name, rid);
      })
      .catch(() => get().openFilePreviewTab(path, name, rid));
  },
  openFilePreviewTab: (path, name, remoteId) => {
    const rid = remoteId ?? null;
    get().ensureDockOpen();
    set((s) => {
      const existing = s.previewTabs.find(
        (t) => t.kind === "file" && t.path === path && (t.remoteId ?? null) === rid,
      );
      if (existing) {
        return { activePreviewId: existing.id };
      }
      const tab: PreviewTab = {
        id: uuid(),
        kind: "file",
        name: name || path.split(/[\\/]/).pop() || path,
        path,
        remoteId: rid,
      };
      return { previewTabs: [...s.previewTabs, tab], activePreviewId: tab.id };
    });
  },
  openDiffPreview: (name, path, diff, remoteId) => {
    const rid = remoteId ?? null;
    get().ensureDockOpen();
    set((s) => {
      const existing = s.previewTabs.find(
        (t) => t.kind === "diff" && t.path === path && (t.remoteId ?? null) === rid,
      );
      if (existing) {
        return { activePreviewId: existing.id };
      }
      const tab: PreviewTab = {
        id: uuid(),
        kind: "diff",
        name: name || path.split(/[\\/]/).pop() || path,
        path,
        diff,
        remoteId: rid,
      };
      return { previewTabs: [...s.previewTabs, tab], activePreviewId: tab.id };
    });
  },
  closePreviewTab: (id) => {
    set((s) => {
      const previewTabs = s.previewTabs.filter((t) => t.id !== id);
      const activePreviewId =
        s.activePreviewId === id ? null : s.activePreviewId;
      return { previewTabs, activePreviewId };
    });
  },
  setActivePreview: (id) => set({ activePreviewId: id || null }),

  fileTreeExpanded: new Set<string>(),
  fileTreeEntries: {},
  loadFileTreeDir: (rawDir, remoteId) => {
    const dir = normPath(rawDir);
    const key = fileTreeKey(dir, remoteId ?? null);
    window.api.fs
      .listDir(dir, remoteId ?? undefined)
      .then((entries) => {
        const sorted = [...(entries as Array<{ name: string; isDir: boolean }>)].sort((a, b) => {
          if (a.isDir !== b.isDir) return a.isDir ? -1 : 1;
          return a.name.localeCompare(b.name);
        });
        set((s) => ({ fileTreeEntries: { ...s.fileTreeEntries, [key]: sorted } }));
      })
      .catch(() => {
        set((s) => ({ fileTreeEntries: { ...s.fileTreeEntries, [key]: [] } }));
      });
  },
  toggleFileTreeDir: (rawDir, remoteId) => {
    const dir = normPath(rawDir);
    const key = fileTreeKey(dir, remoteId ?? null);
    set((s) => {
      const next = new Set(s.fileTreeExpanded);
      if (next.has(key)) next.delete(key);
      else {
        next.add(key);
        get().loadFileTreeDir(dir, remoteId ?? null);
      }
      return { fileTreeExpanded: next };
    });
  },
  revealInFileTree: (rawDir, remoteId) => {
    const rid = remoteId ?? null;
    const dir = normPath(rawDir);
    const root = normPath(get().currentSessionWorkingDirectory() || get().workingDirectory || "");
    const inside = root !== "" && (dir === root || dir.startsWith(root + "/"));
    if (!inside) {
      // Outside the session folder — fall back to a preview tab
      get().openFilePreviewTab(dir, dir.split(/[\\\\/]/).pop() || dir, rid);
      return;
    }
    const cfg = get().config;
    set({
      dockOpen: true,
      dockTab: "files",
      activePreviewId: null,
      config: { ...cfg, dockOpen: true },
    });
    window.api.config.save({ ...cfg, dockOpen: true }).catch(console.error);
    // Expand every ancestor from root to dir and load their entries
    const toExpand: string[] = [];
    let cur = dir;
    while (cur.startsWith(root + "/") && cur !== root) {
      toExpand.push(cur);
      const idx = cur.lastIndexOf("/");
      if (idx <= 0) break;
      cur = cur.slice(0, idx);
    }
    toExpand.push(root);
    const expandKeys = toExpand.map((d) => fileTreeKey(d, rid));
    set((s) => ({ fileTreeExpanded: new Set([...s.fileTreeExpanded, ...expandKeys]) }));
    for (const d of toExpand) get().loadFileTreeDir(d, rid);
  },
  hideSession: (key) => {
    const cfg = get().config;
    const next = [...(cfg.hiddenSessions ?? []), key];
    set({ config: { ...cfg, hiddenSessions: next } });
    window.api.config.save({ ...cfg, hiddenSessions: next }).catch(console.error);
  },
  hideProject: (workingDirectory) => {
    const cfg = get().config;
    const wd = workingDirectory ?? "";
    const keys = get().sessions
      .filter((s) => (s.workingDirectory ?? "") === wd)
      .map((s) => s.acpSessionId ?? s.id);
    const merged = [...new Set([...(cfg.hiddenSessions ?? []), ...keys])];
    const mergedProjects = wd !== "" ? [...new Set([...(cfg.hiddenProjects ?? []), wd])] : (cfg.hiddenProjects ?? []);
    set({ config: { ...cfg, hiddenSessions: merged, hiddenProjects: mergedProjects } });
    window.api.config.save({ ...cfg, hiddenSessions: merged, hiddenProjects: mergedProjects }).catch(console.error);
  },
  unhideSessions: (keys) => {
    const cfg = get().config;
    const next = (cfg.hiddenSessions ?? []).filter((k) => !keys.includes(k));
    set({ config: { ...cfg, hiddenSessions: next } });
    window.api.config.save({ ...cfg, hiddenSessions: next }).catch(console.error);
  },
  unhideProject: (workingDirectory) => {
    const cfg = get().config;
    const next = (cfg.hiddenProjects ?? []).filter((wd) => wd !== workingDirectory);
    set({ config: { ...cfg, hiddenProjects: next } });
    window.api.config.save({ ...cfg, hiddenProjects: next }).catch(console.error);
  },
  reactivateHistorySession: async (vibe) => {
    const existing = get().sessions.find((s) => s.acpSessionId === vibe.sessionId);
    let localId = existing?.id;
    if (!localId) {
      const created = get().addSession(vibe.environment?.workingDirectory || "");
      localId = created.id;
      get().updateSession(localId, {
        acpSessionId: vibe.sessionId,
        name: vibe.title ?? created.name,
        workingDirectory: vibe.environment?.workingDirectory || created.workingDirectory,
      });
    }
    get().setCurrentView("chat");
    await get().loadAcpSession(localId);
  },

  collapsedProjects: [],
  toggleProjectCollapse: (project) => {
    const cfg = get().config;
    const current = get().collapsedProjects;
    const next = current.includes(project)
      ? current.filter((p) => p !== project)
      : [...current, project];
    set({ collapsedProjects: next, config: { ...cfg, collapsedProjects: next } });
    window.api.config.save({ ...cfg, collapsedProjects: next }).catch(console.error);
  },

  messageQueue: [],
  addToMessageQueue: (content) => {
    const msg = { id: uuid(), content, timestamp: Date.now() };
    set((s) => ({ messageQueue: [...s.messageQueue, msg] }));
  },
  removeFromMessageQueue: (id) => {
    set((s) => ({ messageQueue: s.messageQueue.filter((m) => m.id !== id) }));
  },
  updateQueueMessage: (id, content) => {
    set((s) => ({
      messageQueue: s.messageQueue.map((m) => (m.id === id ? { ...m, content } : m)),
    }));
  },
  clearMessageQueue: () => set({ messageQueue: [] }),
  getNextQueuedMessage: () => {
    const { messageQueue } = get();
    return messageQueue[0];
  },

  currentSessionWorkingDirectory: () => {
    const { sessions, currentSessionId, workingDirectory } = get();
    const session = sessions.find((s) => s.id === currentSessionId);
    return session?.workingDirectory || workingDirectory || "";
  },

  addUserMessage: (text, attachments) => {
    const { currentSessionId, updateSession, sessions } = get();
    const msg: Message = {
      id: uuid(),
      sender: "user",
      content: text,
      isComplete: true,
      timestamp: Date.now(),
    };
    if (attachments && attachments.length > 0) {
      msg.attachments = attachments;
    }
    if (currentSessionId) {
      const sess = sessions.find((s) => s.id === currentSessionId);
      if (sess) {
        const name =
          sess.messages.length === 0
            ? text.slice(0, 40) + (text.length > 40 ? "\u2026" : "")
            : sess.name;
        updateSession(currentSessionId, {
          messages: [...sess.messages, msg],
          lastActive: Date.now(),
          updatedAt: Date.now(),
          name,
        });
      }
    }
    return msg;
  },

  startAiMessage: (sessionId) => {
    const id = uuid();
    const msg: Message = {
      id,
      sender: "ai",
      content: "",
      isComplete: false,
      timestamp: Date.now(),
    };
    const sess = get().sessions.find((s) => s.id === sessionId);
    if (sess) {
      get().updateSession(sessionId, { messages: [...sess.messages, msg] });
    }
    set((s) => ({
      streamingMessage: "",
      streamingSessions: s.streamingSessions.includes(sessionId)
        ? s.streamingSessions
        : [...s.streamingSessions, sessionId],
    }));
    return id;
  },

  appendChunk: (sessionId, messageId, chunk) => {
    set((s) => {
      const sessions = s.sessions.map((sess) => {
        if (sess.id !== sessionId) return sess;
        const messages = sess.messages.map((m) =>
          m.id === messageId ? { ...m, content: m.content + chunk } : m,
        );
        return { ...sess, messages };
      });
      return { sessions, streamingMessage: (s.streamingMessage ?? "") + chunk };
    });
  },

  finalizeAiMessage: (sessionId, messageId) => {
    set((s) => {
      const sessions = s.sessions.map((sess) => {
        if (sess.id !== sessionId) return sess;
        const messages = sess.messages.map((m) =>
          m.id === messageId ? { ...m, isComplete: true } : m,
        );
        return { ...sess, messages, lastActive: Date.now(), updatedAt: Date.now() };
      });
      return {
        sessions,
        streamingMessage: null,
        streamingSessions: s.streamingSessions.filter((id) => id !== sessionId),
      };
    });
  },

  updateToolCall: (sessionId, toolCall) => {
    set((s) => {
      const sessions = s.sessions.map((sess) => {
        if (sess.id !== sessionId) return sess;
        const messages = [...sess.messages];
        const lastAiIdx = messages.map((m) => m.sender).lastIndexOf("ai");
        if (lastAiIdx >= 0) {
          const msg = { ...messages[lastAiIdx] };
          const existing = (msg.toolCalls ?? []).findIndex((tc) => tc.id === toolCall.id);
          if (existing >= 0) {
            msg.toolCalls = (msg.toolCalls ?? []).map((tc) =>
              tc.id === toolCall.id ? toolCall : tc,
            );
          } else {
            msg.toolCalls = [...(msg.toolCalls ?? []), toolCall];
          }
          messages[lastAiIdx] = msg;
        }
        return { ...sess, messages };
      });
      return { sessions };
    });
  },

  mergeToolCall: (sessionId, toolCallId, updates) => {
    set((s) => {
      const sessions = s.sessions.map((sess) => {
        if (sess.id !== sessionId) return sess;
        const messages = sess.messages.map((m) => {
          if (m.sender !== "ai") return m;
          const existing = (m.toolCalls ?? []).findIndex((tc) => tc.id === toolCallId);
          if (existing < 0) return m;
          const toolCalls = (m.toolCalls ?? []).map((tc) =>
            tc.id === toolCallId ? { ...tc, ...updates } : tc,
          );
          return { ...m, toolCalls };
        });
        return { ...sess, messages };
      });
      return { sessions };
    });
  },

  startThinking: (sessionId, messageId, blockId) => {
    const block: ThinkingBlock = {
      id: blockId,
      content: "",
      isComplete: false,
      startedAt: Date.now(),
    };
    set((s) => ({
      sessions: s.sessions.map((sess) => {
        if (sess.id !== sessionId) return sess;
        const messages = sess.messages.map((m) => {
          if (m.id !== messageId) return m;
          return { ...m, thinking: [...(m.thinking ?? []), block] };
        });
        return { ...sess, messages };
      }),
    }));
  },

  appendThinkingChunk: (sessionId, messageId, blockId, chunk) => {
    set((s) => ({
      sessions: s.sessions.map((sess) => {
        if (sess.id !== sessionId) return sess;
        const messages = sess.messages.map((m) => {
          if (m.id !== messageId) return m;
          const thinking = (m.thinking ?? []).map((b) =>
            b.id === blockId ? { ...b, content: b.content + chunk } : b,
          );
          return { ...m, thinking };
        });
        return { ...sess, messages };
      }),
    }));
  },

  finalizeThinking: (sessionId, messageId, blockId) => {
    set((s) => ({
      sessions: s.sessions.map((sess) => {
        if (sess.id !== sessionId) return sess;
        const messages = sess.messages.map((m) => {
          if (m.id !== messageId) return m;
          const thinking = (m.thinking ?? []).map((b) =>
            b.id === blockId ? { ...b, isComplete: true, completedAt: Date.now() } : b,
          );
          return { ...m, thinking };
        });
        return { ...sess, messages };
      }),
    }));
  },

  linkAcpSession: (localSessionId, acpSessionId) => {
    set((s) => ({
      sessions: s.sessions.map((sess) =>
        sess.id === localSessionId ? { ...sess, acpSessionId } : sess,
      ),
      activeAcpSessionId: acpSessionId,
    }));
  },

  unlinkAcpSession: (localSessionId) => {
    set((s) => {
      const sess = s.sessions.find((x) => x.id === localSessionId);
      const activeAcpSessionId =
        sess && sess.acpSessionId === s.activeAcpSessionId ? null : s.activeAcpSessionId;
      return {
        sessions: s.sessions.map((x) =>
          x.id === localSessionId ? { ...x, acpSessionId: undefined } : x,
        ),
        activeAcpSessionId,
      };
    });
  },

  getSessionByAcpId: (acpSessionId) => {
    return get().sessions.find((s) => s.acpSessionId === acpSessionId) ?? null;
  },

  pendingLoadSessionId: null,
  setPendingLoadSessionId: (pendingLoadSessionId) => set({ pendingLoadSessionId }),

  importAcpSessions: (acpSessions) => {
    set((s) => {
      const now = Date.now();
      const sessions = [...s.sessions];
      for (const a of acpSessions) {
        if (!a.sessionId) continue;
        const ts = a.updatedAt ? new Date(a.updatedAt).getTime() : now;
        const idx = sessions.findIndex((sess) => sess.acpSessionId === a.sessionId);
        if (idx >= 0) {
          const sess = sessions[idx];
          // Refresh timestamp; keep local name/messages intact
          sessions[idx] = { ...sess, updatedAt: Math.max(sess.updatedAt ?? 0, ts) };
        } else {
          sessions.push({
            id: uuid(),
            name: a.title || a.sessionId.slice(0, 16) + "\u2026",
            acpSessionId: a.sessionId,
            messages: [],
            createdAt: ts,
            lastActive: ts,
            updatedAt: ts,
            workingDirectory: a.cwd || undefined,
          });
        }
      }
      // Newest first, always
      sessions.sort((a, b) => (b.updatedAt ?? b.lastActive) - (a.updatedAt ?? a.lastActive));
      return { sessions };
    });
  },

  applySessionHistory: (acpSessionId, history, title) => {
    set({ sessionLoadProgress: null });
    const local = get().getSessionByAcpId(acpSessionId);
    if (!local) return;
    const messages: Message[] = [];
    history.forEach((h, idx) => {
      const baseId = acpSessionId + "-" + idx;
      const thinking: ThinkingBlock[] | undefined =
        h.thinking && h.thinking.length > 0
          ? h.thinking.map((b, bi) => ({
              id: baseId + "-t" + bi,
              content: b.content,
              isComplete: true,
              startedAt: Date.now(),
              completedAt: Date.now(),
            }))
          : undefined;
      const toolCalls: ToolCall[] | undefined =
        h.toolCalls && h.toolCalls.length > 0
          ? h.toolCalls.map((tc) => ({
              id: tc.id,
              title: tc.title,
              kind: tc.kind,
              status: tc.status === "error" ? ("error" as const) : ("done" as const),
              input: tc.input,
              output: tc.output,
              diffs: tc.diffs,
            }))
          : undefined;
      messages.push({
        id: baseId,
        sender: h.role,
        content: h.content,
        isComplete: true,
        timestamp: Date.now(),
        thinking,
        toolCalls,
      });
    });
    const updates: Partial<Session> = {
      messages,
      lastActive: Date.now(),
      updatedAt: Date.now(),
      loading: false,
    };
    if (title && local.name !== title) updates.name = title;
    get().updateSession(local.id, updates);
    const st = get();
    if (st.pendingLoadSessionId === acpSessionId) st.setPendingLoadSessionId(null);
  },

  loadAcpSession: async (localSessionId) => {
    const session = get().sessions.find((s) => s.id === localSessionId);
    if (!session?.acpSessionId) return;
    const acpSessionId = session.acpSessionId;
    get().updateSession(localSessionId, { loading: true });
    get().setPendingLoadSessionId(acpSessionId);
    const rid = session.remoteId ?? null;
    try {
      const cfg = get().config;
      const cwd = session.workingDirectory || cfg.workingDirectory || ".";
      // v0.7.0: workers persist per connection - resuming only makes sure
      // the right worker is alive (no prime, no restart churn).
      await get().ensureAcpWorker(rid, cwd);
      // Real resume: session/load replays the full history via events
      await window.api.acp.loadSession(acpSessionId, cwd, rid ?? undefined);
      set({ activeAcpSessionId: acpSessionId });
      // Per-session isolation: restore this session’s recorded model/mode/thinking.
      void get().syncSessionChips(localSessionId);
      // Safety net: clear flags if no history event arrives (empty session)
      window.setTimeout(() => {
        const st = useStore.getState();
        if (st.pendingLoadSessionId === acpSessionId) st.setPendingLoadSessionId(null);
        const sess = st.sessions.find((x) => x.id === localSessionId);
        if (sess?.loading) st.updateSession(localSessionId, { loading: false });
      }, 2500);
    } catch (err) {
      console.error("[store] ACP session/load failed:", err);
      get().setPendingLoadSessionId(null);
      get().updateSession(localSessionId, { loading: false });
      // Local sessions can still fall back to the on-disk vibe logs;
      // remote ones have no local logs - just clear the flags.
      if (!rid) await get().loadVibeSessionMessages(localSessionId);
    }
  },

  loadVibeSessionMessages: async (localSessionId) => {
    const session = get().sessions.find((s) => s.id === localSessionId);
    if (!session?.acpSessionId) return;

    try {
      const vibeMessages = (await window.api.stats.vibeSessionMessages(
        session.acpSessionId,
      )) as VibeSessionMessage[];

      const messages: Message[] = [];
      const pendingToolCalls = new Map<string, ToolCall>();

      for (const vm of vibeMessages) {
        if (vm.role === "user") {
          // Skip injected system context messages
          if (vm.injected) continue;
          messages.push({
            id: vm.message_id || uuid(),
            sender: "user",
            content: vm.content ?? "",
            isComplete: true,
            timestamp: Date.now(),
          });
        } else if (vm.role === "assistant") {
          const toolCalls: ToolCall[] = (vm.tool_calls ?? []).map((tc) => {
            let input: Record<string, unknown> = {};
            try {
              input = JSON.parse(tc.function.arguments) as Record<string, unknown>;
            } catch {
              input = {};
            }
            const toolCall: ToolCall = {
              id: tc.id,
              title: tc.function.name,
              kind: "execute",
              status: "done",
              input,
            };
            pendingToolCalls.set(tc.id, toolCall);
            return toolCall;
          });

          const thinking = vm.reasoning_content
            ? [{
                id: vm.reasoning_message_id ?? "thinking",
                content: vm.reasoning_content,
                isComplete: true,
                startedAt: Date.now(),
                completedAt: Date.now(),
              }]
            : undefined;

          const content = vm.content ?? "";
          if (content || toolCalls.length > 0 || thinking) {
            messages.push({
              id: vm.message_id || uuid(),
              sender: "ai",
              content,
              isComplete: true,
              timestamp: Date.now(),
              toolCalls: toolCalls.length > 0 ? toolCalls : undefined,
              thinking,
            });
          }
        } else if (vm.role === "tool") {
          const toolCallId = vm.tool_call_id;
          const rawContent = vm.content ?? "";

          // Parse ACP JSON content-event array into plain text + error detection
          let parsedContent = rawContent;
          let hasErrorItem = false;
          try {
            const parsed = JSON.parse(rawContent) as Array<Record<string, unknown>>;
            if (Array.isArray(parsed)) {
              hasErrorItem = parsed.some(
                (e) => e.type === "tool_error" || e.is_error === true,
              );
              const texts = parsed
                .filter(
                  (e) =>
                    (e.type === "content" || e.type === "tool_error") &&
                    typeof (e.content as Record<string, unknown> | undefined)?.text === "string",
                )
                .map(
                  (e) => (e.content as Record<string, unknown>).text as string,
                );
              if (texts.length > 0) parsedContent = texts.join("\n");
            }
          } catch {
            /* not JSON, use rawContent as-is */
          }

          const isError =
            hasErrorItem || parsedContent.includes("<tool_error>");

          if (toolCallId && pendingToolCalls.has(toolCallId)) {
            const toolCall = pendingToolCalls.get(toolCallId)!;
            toolCall.output = parsedContent;
            toolCall.status = isError ? "error" : "done";
            pendingToolCalls.delete(toolCallId);
          } else if (isError && vm.name) {
            // Fallback: match by tool name for error cases
            for (const [id, tc] of pendingToolCalls) {
              if (tc.title === vm.name) {
                tc.output = parsedContent;
                tc.status = "error";
                pendingToolCalls.delete(id);
                break;
              }
            }
          }
        }
      }

      // Mark any remaining tool calls with error output as failed
      for (const [, tc] of pendingToolCalls) {
        if (tc.output?.includes("<tool_error>")) {
          tc.status = "error";
        }
      }

      get().updateSession(localSessionId, {
        messages,
        lastActive: Date.now(),
        updatedAt: Date.now(),
        loading: false,
      });
    } catch (err) {
      console.error("Failed to load vibe session messages:", err);
      get().updateSession(localSessionId, { loading: false });
    }
  },

  fetchContextSummary: async (localSessionId) => {
    const session = get().sessions.find((s) => s.id === localSessionId);
    if (!session || session.messages.length === 0) return;

    const vibeMessages = session.messages.map((m) => ({
      role: m.sender,
      content: m.content,
    }));

    try {
      const rid = session.remoteId ?? null;
      await get().ensureAcpWorker(rid, session.workingDirectory || get().config.workingDirectory || ".");

      const summary = await window.api.acp.getContextSummary(vibeMessages, rid ?? undefined);

      if (summary && summary.trim()) {
        const contextMessage: Message = {
          id: uuid(),
          sender: "system",
          content: "Previous conversation summary:\n" + summary.trim(),
          isComplete: true,
          timestamp: Date.now(),
          isContext: true,
        };

        get().updateSession(localSessionId, {
          contextSummary: summary.trim(),
          messages: [contextMessage, ...session.messages],
        });
      }
    } catch (err) {
      console.error("Failed to fetch context summary:", err);
    }
  },

  // ---- v0.6.0/v0.7.0: remote SSH connections ----
  remoteStatuses: {},

  ensureAcpWorker: async (remoteId, cwd) => {
    const cfg = get().config;
    const rid = remoteId ?? undefined;
    if (await window.api.acp.isRunning(rid)) return;
    if (!rid && !cfg.acpPath) return;
    get().setAcpStatus("connecting");
    await window.api.acp.ensureWorker(cfg.acpPath || "vibe-acp", cwd || cfg.workingDirectory || ".", rid);
  },

  refreshRemoteData: async () => {
    const conns = (get().config.remoteConnections ?? []).filter((c) => c.enabled);
    if (conns.length === 0) {
      if (Object.keys(get().remoteStatuses).length > 0) set({ remoteStatuses: {} });
      return;
    }
    try {
      const statuses = (await window.api.remote.statusNow(
        conns.map((c) => c.id),
      )) as Record<string, RemoteStatus>;
      set({ remoteStatuses: statuses });
      for (const c of conns) {
        if (!statuses[c.id]?.ok) continue;
        try {
          const infos = (await window.api.remote.sessions(c.id)) as RemoteSessionInfo[];
          get().importRemoteSessions(c.id, infos);
        } catch (err) {
          console.error("[store] remote sessions failed:", c.name, err);
        }
      }
    } catch (err) {
      console.error("[store] remote status failed:", err);
    }
  },

  startRemoteSession: async (remoteId, cwd) => {
    const conn = (get().config.remoteConnections ?? []).find(
      (c: RemoteConnection) => c.id === remoteId,
    );
    if (!conn) return;
    // v0.7.0: create the draft BEFORE starting the worker so the
    // SessionInitialized event can always link to it (multi-worker safe).
    const session = get().addSession(cwd);
    get().updateSession(session.id, { remoteId });
    get().setSessionStarting(true);
    get().setAcpStatus("connecting");
    get().setAcpError(null);
    get().setCurrentView("chat");
    try {
      const cfg = get().config;
      await window.api.acp.start(cfg.acpPath || "vibe-acp", cwd, remoteId);
    } catch (err) {
      get().setSessionStarting(false);
      get().setAcpStatus("error");
      get().setAcpError((err as Error).message);
      return;
    }
    get().setSessionStarting(false);
  },

  loadRemoteSession: async (remoteId, info) => {
    let local = get().sessions.find(
      (s) => s.remoteId === remoteId && s.acpSessionId === info.sessionId,
    );
    if (!local) {
      const session = get().addSession(info.cwd || ".");
      get().updateSession(session.id, {
        remoteId,
        acpSessionId: info.sessionId,
        name: info.title || info.sessionId.slice(0, 16) + "…",
        updatedAt: info.updatedAt ? new Date(info.updatedAt).getTime() : undefined,
      });
      local = get().sessions.find((s) => s.id === session.id)!;
    }
    set({ pendingNewSession: false });
    get().setCurrentSessionId(local.id);
    get().setCurrentView("chat");
    await get().loadAcpSession(local.id);
  },

  importRemoteSessions: (remoteId, infos) => {
    set((s) => {
      const now = Date.now();
      const sessions = [...s.sessions];
      for (const a of infos) {
        if (!a.sessionId) continue;
        const ts = a.updatedAt ? new Date(a.updatedAt).getTime() : now;
        const idx = sessions.findIndex(
          (x) => x.acpSessionId === a.sessionId && x.remoteId === remoteId,
        );
        if (idx >= 0) {
          sessions[idx] = { ...sessions[idx], updatedAt: Math.max(sessions[idx].updatedAt ?? 0, ts) };
        } else {
          sessions.push({
            id: uuid(),
            name: a.title || a.sessionId.slice(0, 16) + "…",
            acpSessionId: a.sessionId,
            messages: [],
            createdAt: ts,
            lastActive: ts,
            updatedAt: ts,
            workingDirectory: a.cwd || undefined,
            remoteId,
          });
        }
      }
      sessions.sort((a, b) => (b.updatedAt ?? b.lastActive) - (a.updatedAt ?? a.lastActive));
      return { sessions };
    });
  },
}));
