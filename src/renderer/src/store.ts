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
  SessionPreviewData,
  PlanEntry,
} from "./types";
import { defaultConfig } from "./types";
import type { RemoteConnection, RemoteStatus, RemoteSessionInfo, AppUpdateInfo, CliUpdateInfo } from "./types";

/** Minimal shapes used to rebuild chip option lists for historical sessions. */
interface HistModelHolder {
  models?: HistModelCfg[];
}
interface HistModelCfg {
  name: string;
  alias?: string;
  display_name?: string;
  thinking?: string;
  thinking_levels?: string[];
  supports_images?: boolean;
}
const keyOf = (m: HistModelCfg): string => m.alias || m.name;
interface HistAgentCfg {
  name: string;
  agent_type?: string;
  display_name?: string;
  description?: string;
}

/** ACP history message (from SessionHistory event) */
export interface AcpHistoryToolCall {
  id: string;
  title: string;
  kind: string;
  status: "running" | "done" | "error";
  input?: Record<string, unknown>;
  output?: string;
  diffs?: Array<{ path: string; oldText: string | null; newText: string; startLine?: number | null; newStartLine?: number | null }>;
  previousContent?: string | null;
  toolName?: string | null;
  rawOutput?: Record<string, unknown> | null;
  subagent?: { agent: string | null; agentName?: string | null; task: string; childSessionId: string | null } | null;
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
  /** v0.10.5: sessions with a pending stop request (session/cancel in flight). */
  stoppingSessions: string[];
  markSessionStopping: (sessionId: string) => void;
  clearSessionStopping: (sessionId: string) => void;
  /** v0.9.0: local session ids with a live ACP session in a worker (runtime only). */
  liveSessions: string[];
  markSessionLive: (sessionId: string) => void;
  /** Last activity timestamp per local session id (drives idle auto-close). */
  sessionActivity: Record<string, number>;
  /** Turn a live session off (frees the worker when it was the last one). */
  closeSession: (sessionId: string) => void;
  /** v0.9.0: lazy resume - read the history from disk without starting ACP. */
  previewSession: (localSessionId: string) => Promise<void>;
  /** v0.9.0: update system state (app + CLI, banner dismissal). */
  updateInfo: {
    app: AppUpdateInfo | null;
    cli: CliUpdateInfo | null;
    checking: boolean;
    dismissed: boolean;
  };
  checkAppUpdate: () => Promise<void>;
  checkCliUpdate: () => Promise<void>;
  /** v0.11.0: app self-update download/install progress (main event). */
  updateProgress: { phase: string; received: number; total: number | null; percent: number | null } | null;
  setUpdateProgress: (p: { phase: string; received: number; total: number | null; percent: number | null } | null) => void;
  checkRemoteCliUpdate: (remoteId: string) => Promise<CliUpdateInfo>;
  dismissUpdateBanner: () => void;
  /** Upgrade the CLI (local or remote); stops all workers and restores live sessions. */
  updateCliNow: (remoteId?: string) => Promise<{ ok: boolean; version: string | null; error: string | null }>;
  updateAppNow: () => Promise<{ ok: boolean; error: string | null }>;
  sessionLoadProgress: { acpSessionId: string; count: number } | null;
  setSessionLoadProgress: (progress: { acpSessionId: string; count: number } | null) => void;

  // Config options (model / mode / thinking) from ACP
  configOptions: ConfigOption[];
  setConfigOptions: (options: ConfigOption[]) => void;
  /** Update the stored currentValue of a single config option (keeps chips in sync). */
  patchConfigOption: (id: string, value: string) => void;
  /** Re-apply the model/mode/thinking recorded on a session after switching to it. */
  syncSessionChips: (localSessionId: string) => Promise<void>;
  /** v0.11.1: restore chips AND the recorded option lists when switching to a session. */
  restoreSessionChips: (localSessionId: string) => Promise<void>;
  /** v0.13.14: recompute the thinking level list for a freshly picked model
   * (option lists are model specific and the CLI is not consulted for a
   * session that is not live yet). */
  refreshThinkingLevels: (modelValue: string) => Promise<void>;
  /** v1.0.7: chips from the vibe config BEFORE any ACP connection. */
  loadDefaultChips: () => Promise<void>;
  /** v1.0.13: mark sessions that run a turn right now as busy + live. */
  syncBusySessions: () => Promise<void>;
  /** v0.11.1: last known configOptions per ACP session (option lists are model specific). */
  sessionConfigOptions: Record<string, ConfigOption[]>;
  setSessionConfigOptions: (acpSessionId: string, options: ConfigOption[]) => void;
  /** v0.11.1: current agent plan (todo checklist) per ACP session. */
  sessionPlans: Record<string, PlanEntry[]>;
  setSessionPlan: (acpSessionId: string, entries: PlanEntry[]) => void;
  currentModel: string;
  /** v1.0.8: models seen in the vibe config with their vision capability. */
  vibeModels: Array<{ value: string; supportsImages: boolean }>;
  setVibeModels: (models: Array<{ value: string; supportsImages: boolean }>) => void;
  /** True when the active model can receive images (unknown = assume yes). */
  activeModelSupportsImages: () => boolean;
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
  /** v1.0.9: open the preview and go straight into edit mode. */
  openFileEditor: (path: string, name?: string, remoteId?: string | null) => void;
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
  reactivateRemoteSession: (remoteId: string, acpSessionId: string, cwd: string, title: string | null) => Promise<void>;

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
  sendQueuedNow: (id: string) => void;

  currentSessionWorkingDirectory: () => string;
  currentSessionRemoteId: () => string | undefined;

  // Actions
  addUserMessage: (text: string, attachments?: { path: string; isDir: boolean; content: string }[], id?: string) => Message;
  startAiMessage: (sessionId: string) => string;
  appendChunk: (sessionId: string, messageId: string, chunk: string) => void;
  finalizeAiMessage: (sessionId: string, messageId: string) => void;
  updateToolCall: (sessionId: string, toolCall: ToolCall) => void;
  mergeToolCall: (sessionId: string, toolCallId: string, updates: Partial<ToolCall>) => void;
  startThinking: (sessionId: string, messageId: string, blockId: string) => void;
  appendThinkingChunk: (sessionId: string, messageId: string, blockId: string, chunk: string) => void;
  finalizeThinking: (sessionId: string, messageId: string, blockId: string) => void;
  /** v0.10.2: the CLI never emits agent_thought_end - close every open thinking block. */
  finalizeAllThinking: (sessionId: string, messageId: string) => void;
  /** v0.10.2: close tool calls still marked running when the turn ends. */
  completeToolCalls: (sessionId: string, messageId: string) => void;
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
  /** v1.0.11: sessions whose history was loaded from disk (per client). */
  historyLoadedSessions: string[];
  loadAcpSession: (localSessionId: string, internalAttempt?: number) => Promise<void>;
  loadVibeSessionMessages: (localSessionId: string) => Promise<void>;
  fetchContextSummary: (localSessionId: string) => Promise<void>;
  // ---- v0.6.0/v0.7.0: remote SSH connections ----
  /** Latest reachability status per connection id */
  remoteStatuses: Record<string, RemoteStatus>;
  /** v1.0.9: layers the system back gesture pops (previews, edit, views). */
  backStack: Array<{ id: number; run: () => void }>;
  pushBack: (run: () => void) => number;
  removeBack: (id: number) => void;
  popBack: () => void;
  /** v1.0.9: connections whose session list is currently being fetched. */
  remoteLoading: Record<string, boolean>;
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


// v1.0.14: panel back-layers (mobile). A panel opened on the phone pushes
// a "close it" layer onto the back stack; closing it by ANY path removes
// that layer, so the LIFO order of previews/panels always matches what
// the user sees. Module-level ids - not reactive state.
const panelLayers: { sidebar?: number; dock?: number } = {}
const isMobileViewport = (): boolean =>
  typeof window !== 'undefined' && window.matchMedia('(max-width: 768px)').matches

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
      // v1.0.7: carry the model/mode/thinking picked on the chips BEFORE
      // any ACP connection (populated from the vibe config at boot) into
      // the draft - the first prompt applies them inside session/new.
      lastModel: get().currentModel || undefined,
      lastMode: get().currentMode || undefined,
      lastThinking: get().currentThinking || undefined,
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
  setMobileSidebarOpen: (v) => {
    const st = get();
    if (v === st.mobileSidebarOpen) return;
    set({ mobileSidebarOpen: v });
    if (!isMobileViewport()) return;
    // v1.0.14: the panel is a back-stack layer (opened -> closable by the
    // back gesture; closed -> the layer is gone).
    if (v && panelLayers.sidebar === undefined) {
      panelLayers.sidebar = get().pushBack(() => {
        panelLayers.sidebar = undefined;
        get().setMobileSidebarOpen(false);
      });
    } else if (!v && panelLayers.sidebar !== undefined) {
      get().removeBack(panelLayers.sidebar);
      panelLayers.sidebar = undefined;
    }
  },
  markSessionBusy: (sessionId) =>
    set((s) => ({
      busySessions: s.busySessions.includes(sessionId) ? s.busySessions : [...s.busySessions, sessionId],
      // v0.10.5: a new turn clears any stale stop request for the session.
      stoppingSessions: s.stoppingSessions.filter((id) => id !== sessionId),
      sessionActivity: { ...s.sessionActivity, [sessionId]: Date.now() },
    })),

  markSessionIdle: (sessionId) => {
    set((s) => ({
      busySessions: s.busySessions.filter((id) => id !== sessionId),
      // v0.10.5: the turn really ended - clear the stopping marker too.
      stoppingSessions: s.stoppingSessions.filter((id) => id !== sessionId),
      sessionActivity: { ...s.sessionActivity, [sessionId]: Date.now() },
    }))
    // v0.11.1: config changes picked while busy (or right before the session
    // was interrupted) are re-applied once the turn is over.
    const sess = get().sessions.find((x) => x.id === sessionId)
    if (sess?.configPendingApply) {
      get().updateSession(sessionId, { configPendingApply: false })
      void get().syncSessionChips(sessionId)
    }
  },

  stoppingSessions: [],
  markSessionStopping: (sessionId) =>
    set((s) => ({
      stoppingSessions: s.stoppingSessions.includes(sessionId)
        ? s.stoppingSessions
        : [...s.stoppingSessions, sessionId],
    })),
  clearSessionStopping: (sessionId) =>
    set((s) => ({ stoppingSessions: s.stoppingSessions.filter((id) => id !== sessionId) })),

  liveSessions: [],
  sessionActivity: {},

  markSessionLive: (sessionId) =>
    set((s) => ({
      liveSessions: s.liveSessions.includes(sessionId) ? s.liveSessions : [...s.liveSessions, sessionId],
      sessionActivity: { ...s.sessionActivity, [sessionId]: Date.now() },
    })),

  closeSession: (sessionId) => {
    const st = get();
    const sess = st.sessions.find((x) => x.id === sessionId);
    if (!sess) return;
    const liveSessions = st.liveSessions.filter((id) => id !== sessionId);
    set({
      liveSessions,
      busySessions: st.busySessions.filter((id) => id !== sessionId),
    });
    // v0.9.0: free the worker when its last live session went away.
    if (sess.remoteId) {
      const stillLive = liveSessions.some((id) => {
        const other = get().sessions.find((x) => x.id === id);
        return other?.remoteId === sess.remoteId;
      });
      if (!stillLive) {
        window.api.acp.stopWorker(sess.remoteId).catch(console.error);
      }
    } else {
      const stillLive = liveSessions.some((id) => {
        const other = get().sessions.find((x) => x.id === id);
        return !other?.remoteId;
      });
      if (!stillLive) {
        window.api.acp.stop().catch(console.error);
      }
    }
  },

  updateInfo: { app: null, cli: null, checking: false, dismissed: false },
  updateProgress: null,

  checkAppUpdate: async () => {
    set((s) => ({ updateInfo: { ...s.updateInfo, checking: true } }));
    try {
      const info = (await window.api.updates.checkApp()) as AppUpdateInfo;
      set((s) => ({ updateInfo: { ...s.updateInfo, app: info, checking: false } }));
    } catch (err) {
      console.error(err);
      set((s) => ({ updateInfo: { ...s.updateInfo, checking: false } }));
    }
  },

  checkCliUpdate: async () => {
    set((s) => ({ updateInfo: { ...s.updateInfo, checking: true } }));
    try {
      const info = (await window.api.updates.checkCli()) as CliUpdateInfo;
      set((s) => ({ updateInfo: { ...s.updateInfo, cli: info, checking: false } }));
    } catch (err) {
      console.error(err);
      set((s) => ({ updateInfo: { ...s.updateInfo, checking: false } }));
    }
  },

  checkRemoteCliUpdate: async (remoteId) => {
    return (await window.api.updates.checkCliRemote(remoteId)) as CliUpdateInfo;
  },

  dismissUpdateBanner: () => set((s) => ({ updateInfo: { ...s.updateInfo, dismissed: true } })),

  setUpdateProgress: (p) => set({ updateProgress: p }),

  updateCliNow: async (remoteId) => {
    // Main stops every vibe-acp worker first; afterwards bring all previously
    // live sessions back (worker restart + real session/load resume).
    const liveBefore = [...get().liveSessions];
    const currentId = get().currentSessionId;
    try {
      const res = (await window.api.updates.updateCli(remoteId)) as {
        version: string | null;
        error: string | null;
      };
      if (res.error) return { ok: false, version: null, error: res.error };
      for (const id of liveBefore) {
        if (id !== currentId) await get().loadAcpSession(id).catch(console.error);
      }
      if (currentId && liveBefore.includes(currentId)) {
        await get().loadAcpSession(currentId).catch(console.error);
      }
      await get().checkCliUpdate().catch(console.error);
      return { ok: true, version: res.version, error: null };
    } catch (err) {
      return { ok: false, version: null, error: (err as Error).message };
    }
  },

  updateAppNow: async () => {
    try {
      return (await window.api.updates.updateApp()) as { ok: boolean; error: string | null };
    } catch (err) {
      return { ok: false, error: (err as Error).message };
    }
  },

  previewSession: async (localSessionId) => {
    const session = get().sessions.find((s) => s.id === localSessionId);
    if (!session?.acpSessionId) return;
    get().updateSession(localSessionId, { loading: true });
    try {
      const rid = session.remoteId ?? null;
      const preview = rid
        ? ((await window.api.stats.remoteSessionPreview(rid, session.acpSessionId)) as SessionPreviewData | null)
        : ((await window.api.stats.sessionPreview(session.acpSessionId)) as SessionPreviewData | null);
      if (preview && preview.messages && preview.messages.length > 0) {
        get().applySessionHistory(
          session.acpSessionId,
          preview.messages as AcpHistoryMessage[],
          preview.title ?? undefined,
        );
      } else {
        if (preview?.title) get().updateSession(localSessionId, { name: preview.title });
        get().updateSession(localSessionId, { loading: false });
      }
      if (preview) {
        // v0.11.1: lazy preview carries the session plan (todo snapshot)
        if (preview.plan !== null && preview.plan !== undefined) {
          get().setSessionPlan(session.acpSessionId, preview.plan as PlanEntry[])
        }
        if (preview.usage) {
          get().setUsage(localSessionId, {
            contextUsed: preview.usage.contextUsed,
            contextSize: 0,
            promptTokens: preview.usage.promptTokens,
            completionTokens: preview.usage.completionTokens,
            cachedTokens: preview.usage.cachedTokens,
            totalTokens: preview.usage.totalTokens,
            cost: preview.usage.cost,
          });
        }
        const patch: { lastModel?: string; lastMode?: string; lastThinking?: string } = {};
        if (preview.config?.model) patch.lastModel = preview.config.model;
        if (preview.config?.mode) patch.lastMode = preview.config.mode;
        if (preview.config?.thinking) patch.lastThinking = preview.config.thinking;
        if (Object.keys(patch).length > 0) get().updateSession(localSessionId, patch);
        // Display-only chip move: the session is not live yet, so no ACP
        // call happens here (the real sync runs after the session resumes).
        const st2 = get();
        if (st2.currentSessionId === localSessionId) {
          if (patch.lastModel) {
            st2.setCurrentModel(patch.lastModel);
            st2.patchConfigOption("model", patch.lastModel);
          }
          if (patch.lastMode) {
            st2.setCurrentMode(patch.lastMode);
            st2.patchConfigOption("mode", patch.lastMode);
          }
          if (patch.lastThinking) {
            st2.setCurrentThinking(patch.lastThinking);
            st2.patchConfigOption("thinking", patch.lastThinking);
          }
        }
      }
    } catch (err) {
      console.error("[store] session preview failed:", err);
      get().updateSession(localSessionId, { loading: false });
    }
  },
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
      // v0.10.2: never send config changes mid-turn - a set_config_option
      // racing an in-flight prompt hangs the CLI turn. Display-only here;
      // the main process applies recorded config inside session/new|load.
      // v0.11.1: remember the deferred change - the idle transition re-applies
      // it, so a session interrupted mid-turn keeps the config the user picked.
      if (get().busySessions.includes(localSessionId)) {
        get().updateSession(localSessionId, { configPendingApply: true });
        return;
      }
      // v0.11.1: lazy (not live) sessions have no ACP session to sync - the
      // main process applies the recorded config inside session/new|load.
      if (!get().liveSessions.includes(localSessionId)) return;
      window.api.acp.setConfigOption(id, value, acpSessionId, chipRid ?? undefined).catch(console.error);
    };
    apply("model", model, get().currentModel);
    apply("mode", mode, get().currentMode);
    apply("thinking", thinking, get().currentThinking);
  },

  restoreSessionChips: async (localSessionId) => {
    const session = get().sessions.find((s) => s.id === localSessionId);
    if (!session?.acpSessionId) return;
    // Active/live session (or one the ACP reported this run): restore
    // exactly the option lists the ACP sent for it.
    const recorded = get().sessionConfigOptions[session.acpSessionId];
    if (recorded && recorded.length > 0) {
      get().setConfigOptions(recorded);
      await get().syncSessionChips(localSessionId);
      return;
    }
    // v0.12: historical session - never inherit the option lists of the
    // previously viewed session. Build display-only lists from the vibe
    // config of the session's cwd (global + project, local or remote).
    const rid = session.remoteId ?? null;
    const cwd = session.workingDirectory || get().workingDirectory || "";
    try {
      const [gcfgRaw, pcfgRaw, gagentsRaw, pagentsRaw] = await Promise.all([
        window.api.vibe.getConfig(rid ? "remote" : "global", "", rid ?? undefined) as Promise<HistModelHolder | null>,
        window.api.vibe.getConfig("project", cwd, rid ?? undefined) as Promise<HistModelHolder | null>,
        window.api.vibe.getAgents(rid ? "remote" : "global", "", rid ?? undefined) as Promise<HistAgentCfg[]>,
        window.api.vibe.getAgents("project", cwd, rid ?? undefined) as Promise<HistAgentCfg[]>,
      ]);
      const gmodels = (gcfgRaw && gcfgRaw.models) || [];
      const pmodels = (pcfgRaw && pcfgRaw.models) || [];
      const merged: HistModelCfg[] = [...gmodels];
      for (const pm of pmodels) {
        const i = merged.findIndex((m) => keyOf(m) === keyOf(pm));
        if (i >= 0) merged[i] = pm;
        else merged.push(pm);
      }
      const lastModel = session.lastModel || "";
      const cur = lastModel
        ? merged.find((m) => keyOf(m) === lastModel || m.name === lastModel)
        : undefined;
      // CLI defaults: the five levels; an explicitly empty set means "off".
      const levels =
        cur && cur.thinking_levels && cur.thinking_levels.length > 0
          ? cur.thinking_levels
          : ["off", "low", "medium", "high", "max"];
      let thinkingCurrent = session.lastThinking || "";
      if (!levels.includes(thinkingCurrent)) {
        const dl = cur?.thinking;
        thinkingCurrent =
          dl && levels.includes(dl)
            ? dl
            : levels.includes("high")
              ? "high"
              : levels[levels.length - 1];
      }
      const agents = [
        ...(gagentsRaw || []),
        ...(pagentsRaw || []),
      ].filter((a) => (a.agent_type ?? "agent") === "agent");
      const agentNames = new Set<string>();
      const agentChoices = agents
        .filter((a) => (agentNames.has(a.name) ? false : (agentNames.add(a.name), true)))
        .map((a) => ({ value: a.name, name: a.display_name || a.name, description: a.description || undefined }));
      const options: ConfigOption[] = [
        {
          id: "mode",
          name: "Mode",
          type: "select",
          currentValue: session.lastMode || "accept-edits",
          options: [
            { value: "ask", name: "Ask" },
            { value: "accept-edits", name: "Accept Edits" },
            { value: "smart-approve", name: "Smart Approve" },
            { value: "auto-approve", name: "Auto Approve" },
            ...agentChoices,
          ],
        },
        {
          id: "model",
          name: "Model",
          type: "select",
          currentValue: lastModel,
          options: merged.map((m) => ({
            value: keyOf(m),
            name: m.display_name || m.name,
            description: m.name,
          })),
        },
        {
          id: "thinking",
          name: "Thinking",
          type: "select",
          currentValue: thinkingCurrent,
          options: levels.map((l) => ({ value: l, name: l.charAt(0).toUpperCase() + l.slice(1) })),
        },
      ];
      get().setConfigOptions(options);
    } catch {
      // vibe config unavailable (e.g. remote host down) - keep current chips
    }
    await get().syncSessionChips(localSessionId);
  },

  // v0.13.14: the thinking option list is model specific - when the user
  // picks a model before a session is live (no ACP confirmation round
  // trip), rebuild the level list from the vibe config of the model that
  // was just selected and clamp the current level to it.
  refreshThinkingLevels: async (modelValue) => {
    const session = get().currentSession();
    // v0.13.15: for a LIVE session the CLI itself confirms the new model's
    // option set (set_config_option response -> ConfigOptionsUpdate). The
    // local rebuild below raced that confirmation and clobbered the correct
    // thinking levels with config-file defaults, so the picker showed levels
    // the freshly chosen model does not have.
    if (session && get().liveSessions.includes(session.id)) return;
    const rid = session?.remoteId ?? null;
    const cwd = session?.workingDirectory || get().workingDirectory || "";
    try {
      const [gcfgRaw, pcfgRaw] = await Promise.all([
        window.api.vibe.getConfig(rid ? "remote" : "global", "", rid ?? undefined) as Promise<HistModelHolder | null>,
        window.api.vibe.getConfig("project", cwd, rid ?? undefined) as Promise<HistModelHolder | null>,
      ]);
      const gmodels = (gcfgRaw && gcfgRaw.models) || [];
      const pmodels = (pcfgRaw && pcfgRaw.models) || [];
      const merged: HistModelCfg[] = [...gmodels];
      for (const pm of pmodels) {
        const i = merged.findIndex((m) => keyOf(m) === keyOf(pm));
        if (i >= 0) merged[i] = pm;
        else merged.push(pm);
      }
      const chosen = merged.find(
        (m) =>
          keyOf(m) === modelValue ||
          m.name === modelValue ||
          m.alias === modelValue ||
          m.display_name === modelValue,
      );
      if (!chosen) return;
      // Only an explicitly declared level list may override the dropdown.
      // Without the declaration the CLI decides at resume time (the list
      // arrives with SessionInitialized) - guessing defaults here produced
      // levels the freshly picked model does not offer.
      const levels = chosen.thinking_levels;
      if (!levels || levels.length === 0) return;
      // v1.0.8: refresh the vision capability for the freshly chosen model.
      get().setVibeModels(
        merged.map((m) => ({ value: keyOf(m), supportsImages: m.supports_images !== false })),
      );
      set((s) => ({
        configOptions: s.configOptions.map((o) =>
          o.id === "thinking"
            ? {
                ...o,
                currentValue:
                  typeof o.currentValue === "string" && levels.includes(o.currentValue)
                    ? o.currentValue
                    : levels[levels.length - 1],
                options: levels.map((l) => ({ value: l, name: l.charAt(0).toUpperCase() + l.slice(1) })),
              }
            : o,
        ),
      }));
      if (!levels.includes(get().currentThinking)) {
        const clamped = levels[levels.length - 1];
        get().setCurrentThinking(clamped);
        get().patchConfigOption("thinking", clamped);
        // Record the clamped level so the resume applies a value the new
        // model actually accepts.
        const cur = get().currentSession();
        if (cur) get().updateSession(cur.id, { lastThinking: clamped });
      }
    } catch {
      /* vibe config unavailable - keep the current level list */
    }
  },

  // v1.0.13: a freshly booted client (phone PWA reopened, browser F5)
  // learns from the main process which sessions run a turn RIGHT NOW and
  // marks them busy + live - without this the session looked idle
  // ("stalled") on the new client while the turn kept running server-side.
  syncBusySessions: async () => {
    try {
      const busy = (await window.api.acp.busySessions()) as Array<{
        acpSessionId: string;
        remoteId: string | null;
      }>;
      for (const b of busy) {
        const local = get().sessions.find(
          (s) => s.acpSessionId === b.acpSessionId && (s.remoteId ?? null) === (b.remoteId ?? null),
        );
        if (!local) continue;
        get().markSessionLive(local.id);
        get().markSessionBusy(local.id);
      }
      // A busy turn also means the worker knows the session - loading the
      // history from disk for it is safe and completes the client state.
      const current = get().currentSession();
      if (
        current?.acpSessionId &&
        busy.some((b) => b.acpSessionId === current.acpSessionId) &&
        !get().historyLoadedSessions.includes(current.id) &&
        !current.loading
      ) {
        await get().previewSession(current.id);
      }
    } catch (err) {
      console.error("[store] busy sessions sync failed:", err);
    }
  },

  // v1.0.7: no session is opened at boot anymore, so the chips must come
  // from the vibe config alone (models, built-in CLI modes, the active
  // model's thinking levels) BEFORE any ACP connection - the user picks
  // model/mode/thinking here and addSession carries the picks into the
  // draft, where the first prompt applies them inside session/new.
  loadDefaultChips: async () => {
    if (get().currentSession()) return; // a session owns the chips
    try {
      const [cfgRaw, agentsRaw] = await Promise.all([
        window.api.vibe.getConfig("global", "", undefined) as Promise<HistModelHolder | null>,
        window.api.vibe.getAgents("global", "", undefined) as Promise<HistAgentCfg[]>,
      ]);
      const models = (cfgRaw && cfgRaw.models) || [];
      const activeAlias = (cfgRaw as { active_model?: string } | null)?.active_model || "";
      const wanted = get().currentModel;
      const cur =
        models.find((m) => keyOf(m) === wanted || m.name === wanted) ||
        models.find((m) => keyOf(m) === activeAlias || m.name === activeAlias) ||
        models[0];
      // CLI defaults: the five levels; an explicitly empty set means "off".
      const levels =
        cur && cur.thinking_levels && cur.thinking_levels.length > 0
          ? cur.thinking_levels
          : ["off", "low", "medium", "high", "max"];
      const pickedThinking = get().currentThinking;
      const thinkingCurrent = levels.includes(pickedThinking)
        ? pickedThinking
        : cur?.thinking && levels.includes(cur.thinking)
          ? cur.thinking
          : levels.includes("high")
            ? "high"
            : levels[levels.length - 1];
      const agentNames = new Set<string>();
      const agentChoices = (agentsRaw || [])
        .filter((a) => (a.agent_type ?? "agent") === "agent")
        .filter((a) => (agentNames.has(a.name) ? false : (agentNames.add(a.name), true)))
        .map((a) => ({ value: a.name, name: a.display_name || a.name, description: a.description || undefined }));
      const options: ConfigOption[] = [
        {
          id: "mode",
          name: "Mode",
          type: "select",
          // The CLI's default initial agent (BuiltinAgentName.ACCEPT_EDITS).
          currentValue: get().currentMode || "accept-edits",
          options: [
            { value: "ask", name: "Ask" },
            { value: "accept-edits", name: "Accept Edits" },
            { value: "smart-approve", name: "Smart Approve" },
            { value: "auto-approve", name: "Auto Approve" },
            ...agentChoices,
          ],
        },
        {
          id: "model",
          name: "Model",
          type: "select",
          currentValue: cur ? keyOf(cur) : get().currentModel || "",
          options: models.map((m) => ({
            value: keyOf(m),
            name: m.display_name || m.name,
            description: m.name,
          })),
        },
        {
          id: "thinking",
          name: "Thinking",
          type: "select",
          currentValue: thinkingCurrent,
          options: levels.map((l) => ({ value: l, name: l.charAt(0).toUpperCase() + l.slice(1) })),
        },
      ];
      get().setConfigOptions(options);
      if (cur) get().setCurrentModel(keyOf(cur));
      if (!get().currentMode) get().setCurrentMode("accept-edits");
      get().setCurrentThinking(thinkingCurrent);
      // v1.0.8: remember vision capability per model for image gating.
      get().setVibeModels(
        models.map((m) => ({ value: keyOf(m), supportsImages: m.supports_images !== false })),
      );
    } catch {
      /* vibe config unavailable - chips populate on the first ACP session */
    }
  },

  sessionConfigOptions: {},
  setSessionConfigOptions: (acpSessionId, options) =>
    set((s) => ({ sessionConfigOptions: { ...s.sessionConfigOptions, [acpSessionId]: options } })),

  sessionPlans: {},
  setSessionPlan: (acpSessionId, entries) =>
    set((s) => ({ sessionPlans: { ...s.sessionPlans, [acpSessionId]: entries } })),

  currentModel: "",
  vibeModels: [],
  setVibeModels: (vibeModels) => set({ vibeModels }),
  activeModelSupportsImages: () => {
    const wanted = get().currentModel;
    if (!wanted) return true;
    const m = get().vibeModels.find((x) => x.value === wanted);
    return m ? m.supportsImages : true;
  },
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

  dockOpen: defaultConfig.dockOpen,
  setDockOpen: (dockOpen) => {
    const cfg = get().config;
    set({ dockOpen, config: { ...cfg, dockOpen } });
    if (isMobileViewport() && !dockOpen && panelLayers.dock !== undefined) {
      get().removeBack(panelLayers.dock);
      panelLayers.dock = undefined;
    }
    window.api.config.save({ ...cfg, dockOpen }).catch(console.error);
  },
  ensureDockOpen: () => {
    if (get().dockOpen) return;
    const cfg = get().config;
    set({ dockOpen: true, config: { ...cfg, dockOpen: true } });
    if (isMobileViewport() && panelLayers.dock === undefined) {
      // v1.0.14: the dock is a back-stack layer on mobile; the LIFO
      // order is automatic - a preview opened later pushed its layer
      // after this one, so back pops the preview first, then this.
      panelLayers.dock = get().pushBack(() => {
        panelLayers.dock = undefined;
        get().setDockOpen(false);
      });
    }
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
    // v1.0.9: back gesture returns to the file list from a preview tab.
    get().pushBack(() => useStore.setState({ activePreviewId: null }));
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
  openFileEditor: (path, name, remoteId) => {
    const rid = remoteId ?? null;
    // v1.0.9: back gesture returns to the file list from the editor.
    get().pushBack(() => useStore.setState({ activePreviewId: null }));
    get().ensureDockOpen();
    set((s) => {
      const existing = s.previewTabs.find(
        (t) => t.kind === "file" && t.path === path && (t.remoteId ?? null) === rid,
      );
      let tabs = s.previewTabs;
      let activeId = existing?.id;
      if (!existing) {
        const tab: PreviewTab = {
          id: uuid(),
          kind: "file",
          name: name || path.split(/[\\/]/).pop() || path,
          path,
          remoteId: rid,
          startEdit: true,
        };
        tabs = [...tabs, tab];
        activeId = tab.id;
      }
      return { previewTabs: tabs, activePreviewId: activeId };
    });
  },
  openDiffPreview: (name, path, diff, remoteId) => {
    const rid = remoteId ?? null;
    // v1.0.9: back gesture returns to the file list from a diff tab.
    get().pushBack(() => useStore.setState({ activePreviewId: null }));
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
    // v0.13.12: make the reactivated session current BEFORE the load, like
    // loadRemoteSession already did. Without this the chat view kept showing
    // the PREVIOUS session during the replay: no Resuming indicator, no
    // send guard for the session the user clicked, and an Enter went to the
    // old session instead (E2E: the prompt raced the load of another session).
    set({ pendingNewSession: false });
    get().setCurrentSessionId(localId);
    get().setCurrentView("chat");
    await get().loadAcpSession(localId);
  },

  reactivateRemoteSession: async (remoteId, acpSessionId, cwd, title) => {
    const existing = get().sessions.find(
      (s) => s.acpSessionId === acpSessionId && s.remoteId === remoteId,
    );
    let localId = existing?.id;
    if (!localId) {
      const created = get().addSession(cwd || "");
      localId = created.id;
      get().updateSession(localId, {
        acpSessionId,
        remoteId,
        name: title ?? created.name,
        workingDirectory: cwd || created.workingDirectory,
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

  // Jump the queue. When a turn is really running, move the picked message
  // to the front and break the turn - the turn-end flush in App then sends
  // it as the very next prompt. When nothing is actually running (idle
  // session, or a busy flag left over from an interrupted resume) there is
  // no turn-end flush that would ever pick the message up - it would sit in
  // the queue forever, so it is sent right away instead.
  sendQueuedNow: (id) => {
    const { messageQueue } = get();
    const msg = messageQueue.find((m) => m.id === id);
    if (!msg) return;
    const sessionId = get().currentSessionId;
    const session = sessionId ? get().sessions.find((s) => s.id === sessionId) : undefined;
    if (!session || !session.acpSessionId) return;
    const isBusy = sessionId != null && get().busySessions.includes(sessionId);
    if (!isBusy) {
      // Force send: no turn to break, so deliver the message immediately.
      set({ messageQueue: messageQueue.filter((m) => m.id !== id) });
      const acpSessionId = useStore.getState().activeAcpSessionId ?? session.acpSessionId;
      const promptText = session.contextSummary
        ? `${session.contextSummary}\n\nUser: ${msg.content}`
        : msg.content;
      get().addUserMessage(msg.content);
      get().markSessionBusy(session.id);
      window.api.acp
        .sendPrompt(promptText, acpSessionId, session.remoteId ?? undefined)
        .catch((err: Error) => {
          get().markSessionIdle(session.id);
          set({ acpError: err.message });
        });
      return;
    }
    set({ messageQueue: [msg, ...messageQueue.filter((m) => m.id !== id)] });
    window.api.acp.cancelSession(session.acpSessionId, session.remoteId ?? undefined).catch(console.error);
  },

  currentSessionWorkingDirectory: () => {
    const { sessions, currentSessionId, workingDirectory } = get();
    const session = sessions.find((s) => s.id === currentSessionId);
    return session?.workingDirectory || workingDirectory || "";
  },

  currentSessionRemoteId: () => {
    const { sessions, currentSessionId } = get();
    const session = sessions.find((s) => s.id === currentSessionId);
    return session?.remoteId;
  },

  addUserMessage: (text, attachments, id) => {
    const { currentSessionId, updateSession, sessions } = get();
    const msg: Message = {
      // v1.0.10: a deterministic id (when given) lets the UserMessage echo
      // from main be recognised and never duplicated across clients.
      id: id ?? uuid(),
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
            msg.toolCalls = [...(msg.toolCalls ?? []), { ...toolCall, startedAt: toolCall.startedAt ?? Date.now() }];
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
    // v0.10.2: guard against duplicate blocks when the CLI resends a chunk
    // with the same block id (re-sync after subagent handoffs).
    const msg0 = get()
      .sessions.find((s) => s.id === sessionId)
      ?.messages.find((m) => m.id === messageId)
    if (msg0?.thinking?.some((b) => b.id === blockId)) return
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

  // v0.10.2: the CLI never finalizes thinking blocks, so the renderer closes
  // them when the answer text / tool calls / turn end arrive.
  finalizeAllThinking: (sessionId, messageId) => {
    set((s) => ({
      sessions: s.sessions.map((sess) => {
        if (sess.id !== sessionId) return sess;
        const messages = sess.messages.map((m) => {
          if (m.id !== messageId) return m;
          const thinking = (m.thinking ?? []).map((b) =>
            b.isComplete ? b : { ...b, isComplete: true, completedAt: Date.now() },
          );
          return { ...m, thinking };
        });
        return { ...sess, messages };
      }),
    }));
  },

  // v0.10.2: a turn can end while a tool call is still "running" (lost
  // update, cancel) - close them so the steps timeline has no spinners.
  completeToolCalls: (sessionId, messageId) => {
    set((s) => ({
      sessions: s.sessions.map((sess) => {
        if (sess.id !== sessionId) return sess;
        const messages = sess.messages.map((m) => {
          if (m.id !== messageId) return m;
          const toolCalls = (m.toolCalls ?? []).map((tc) =>
            tc.status === "running" ? { ...tc, status: "done" as const } : tc,
          );
          return { ...m, toolCalls };
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
    // v1.0.11: this client now has the full on-disk history for the
    // session - a re-open does not need to re-read it.
    set((st) => ({
      historyLoadedSessions: st.historyLoadedSessions.includes(local.id)
        ? st.historyLoadedSessions
        : [...st.historyLoadedSessions, local.id],
    }));
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
              previousContent: tc.previousContent,
              toolName: tc.toolName,
              rawOutput: tc.rawOutput,
              subagent: tc.subagent,
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

  loadAcpSession: async (localSessionId, attempt) => {
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
      // v0.10.2: recorded config is applied inside session/load (main),
      // deterministically before any prompt can race it.
      const applyCfg = {
        model: session.lastModel,
        mode: session.lastMode,
        thinking: session.lastThinking
      };
      const hasApplyCfg = !!(applyCfg.model || applyCfg.mode || applyCfg.thinking);
      // Real resume: session/load replays the full history via events
      await window.api.acp.loadSession(acpSessionId, cwd, rid ?? undefined, hasApplyCfg ? applyCfg : undefined);
      set({ activeAcpSessionId: acpSessionId });
      get().markSessionLive(localSessionId);
      // v0.13.12: the load has SETTLED - clear the replay-guard flags here,
      // deterministically. The old fixed safety net (2.5 s local / 15 s
      // remote) fired while big sessions were still replaying: the Resuming
      // indicator vanished, the chat looked empty and sending became
      // possible again while session/load was still in flight, so the
      // user's prompt raced the load and SessionHistory then replaced the
      // whole message array, swallowing the just-sent message from the
      // UI. Main always emits SessionHistory before this promise resolves
      // and rejects within its 600 s timeout; the timer below only guards
      // a lost IPC reply and sits beyond that timeout.
      const settled = get();
      if (settled.pendingLoadSessionId === acpSessionId) settled.setPendingLoadSessionId(null);
      const sessAfter = settled.sessions.find((x) => x.id === localSessionId);
      if (sessAfter?.loading) settled.updateSession(localSessionId, { loading: false });
      // Per-session isolation: restore this session’s recorded model/mode/thinking.
      void get().syncSessionChips(localSessionId);
      // v0.13.14: restore the todo plan snapshot on resume - the CLI does
      // not replay plan updates during session/load, so without this the
      // plan card stayed empty after an app restart or a resume.
      const hasPlan = (get().sessionPlans[acpSessionId]?.length ?? 0) > 0;
      if (!hasPlan) {
        void (async () => {
          try {
            const preview = rid
              ? await window.api.stats.remoteSessionPreview(rid, acpSessionId)
              : await window.api.stats.sessionPreview(acpSessionId);
            const plan = (preview as { plan?: PlanEntry[] | null } | null)?.plan;
            // A live Plan event may have arrived while the preview was in
            // flight - never overwrite a fresher plan with the snapshot.
            if (plan && plan.length > 0 && (get().sessionPlans[acpSessionId]?.length ?? 0) === 0)
              get().setSessionPlan(acpSessionId, plan);
          } catch (err) {
            console.error("[store] plan restore failed:", err);
          }
        })();
      }
      window.setTimeout(() => {
        const st = useStore.getState();
        if (st.pendingLoadSessionId === acpSessionId) st.setPendingLoadSessionId(null);
        const sess = st.sessions.find((x) => x.id === localSessionId);
        if (sess?.loading) st.updateSession(localSessionId, { loading: false });
      }, 620_000);
    } catch (err) {
      // v1.0.16: a transport hiccup (server restart while resuming, WS
      // blip) rejects the load with "Connection closed" - RETRY instead
      // of a dead-end "Resume failed" banner. The web bridge reconnects
      // within ~3 s, so a few short attempts cover the restart window.
      const msg = String((err as Error)?.message ?? err);
      const transportHiccup =
        /Connection closed|app server was lost|Request failed/.test(msg);
      const attemptN = attempt ?? 1;
      if (transportHiccup && attemptN < 4) {
        console.error("[store] ACP session/load transport hiccup, retrying:", msg);
        await new Promise((r) => setTimeout(r, 3000));
        return get().loadAcpSession(localSessionId, attemptN + 1);
      }
      console.error("[store] ACP session/load failed:", err);
      get().setPendingLoadSessionId(null);
      get().updateSession(localSessionId, { loading: false });
      // Local sessions can still fall back to the on-disk vibe logs;
      // remote ones have no local logs - surface the failure so the user
      // sees why the history did not render instead of an empty chat.
      if (!rid) await get().loadVibeSessionMessages(localSessionId);
      else {
        get().setAcpStatus("error");
        get().setAcpError("Resume failed: " + msg);
      }
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
  remoteLoading: {},
  backStack: [],
  historyLoadedSessions: [],
  pushBack: (run) => {
    const id = Date.now() + Math.random();
    set((s) => ({ backStack: [...s.backStack.slice(-31), { id, run }] }));
    return id;
  },
  removeBack: (id) => set((s) => ({ backStack: s.backStack.filter((e) => e.id !== id) })),
  popBack: () => {
    const st = get();
    const top = st.backStack[st.backStack.length - 1];
    if (!top) return;
    set({ backStack: st.backStack.slice(0, -1) });
    try {
      top.run();
    } catch {
      /* a stale layer must never break the back gesture */
    }
  },

  ensureAcpWorker: async (remoteId, cwd) => {
    const cfg = get().config;
    const rid = remoteId ?? undefined;
    const running = await window.api.acp.isRunning(rid);
    // v0.13.12: never short-circuit on a live process handle. A frozen or
    // hung worker (SIGSTOP, stuck CLI, dead transport) "is running" too, and
    // resuming against it used to hang until the 600 s session/load timeout
    // with the Resuming spinner up and sending blocked. Hand EVERY resume
    // to the main ensure-worker probe: it pings (skipped mid-turn) and
    // restarts the worker when the probe fails. Only a real spawn flips
    // the status to connecting.
    if (!running) {
      if (!rid && !cfg.acpPath) return;
      get().setAcpStatus("connecting");
    }
    await window.api.acp.ensureWorker(cfg.acpPath || "vibe-acp", cwd || cfg.workingDirectory || ".", rid);
  },

  refreshRemoteData: async () => {
    const conns = (get().config.remoteConnections ?? []).filter((c) => c.enabled);
    if (conns.length === 0) {
      if (Object.keys(get().remoteStatuses).length > 0) set({ remoteStatuses: {} });
      return;
    }
    // v1.0.9: per-connection loading flag - the sidebar shows a spinner
    // instead of the "no sessions" look while the list is being fetched.
    set({ remoteLoading: Object.fromEntries(conns.map((c) => [c.id, true])) });
    try {
      const statuses = (await window.api.remote.statusNow(
        conns.map((c) => c.id),
      )) as Record<string, RemoteStatus>;
      set({ remoteStatuses: statuses });
      // v1.0.9: fetch every connection's session list CONCURRENTLY - the
      // old sequential loop stacked whole SSH+ACP handshakes (25 s timeout
      // each), so the first launch could sit for minutes without a list.
      await Promise.allSettled(
        conns.map(async (c) => {
          if (!statuses[c.id]?.ok) return;
          try {
            const infos = (await window.api.remote.sessions(c.id)) as RemoteSessionInfo[];
            get().importRemoteSessions(c.id, infos);
          } catch (err) {
            console.error("[store] remote sessions failed:", c.name, err);
          }
        }),
      );
    } catch (err) {
      console.error("[store] remote status failed:", err);
    } finally {
      set({ remoteLoading: {} });
    }
  },

  startRemoteSession: async (remoteId, cwd) => {
    const conn = (get().config.remoteConnections ?? []).find(
      (c: RemoteConnection) => c.id === remoteId,
    );
    if (!conn) return;
    // v1.0.15: entering a remote session is the same navigation as a
    // local one - the back gesture returns to the session list.
    if (
      typeof window !== "undefined" &&
      window.matchMedia("(max-width: 768px)").matches
    ) {
      get().pushBack(() => get().setMobileSidebarOpen(true));
      get().setMobileSidebarOpen(false);
    }
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
