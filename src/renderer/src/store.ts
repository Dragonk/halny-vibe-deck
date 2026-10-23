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
  PreviewTab,
} from "./types";
import { defaultConfig } from "./types";

/** ACP history message (from SessionHistory event) */
export interface AcpHistoryToolCall {
  id: string;
  title: string;
  kind: string;
  status: "running" | "done" | "error";
  input?: Record<string, unknown>;
  output?: string;
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

  // Config options (model / mode / thinking) from ACP
  configOptions: ConfigOption[];
  setConfigOptions: (options: ConfigOption[]) => void;
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
  dockWidth: number;
  setDockWidth: (w: number) => void;
  dockTab: DockTabKind;
  setDockTab: (tab: DockTabKind) => void;
  previewTabs: PreviewTab[];
  activePreviewId: string | null;
  openFilePreview: (path: string, name?: string) => void;
  openDiffPreview: (name: string, path: string, diff: string) => void;
  closePreviewTab: (id: string) => void;
  setActivePreview: (id: string) => void;

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
  markSessionBusy: (sessionId) =>
    set((s) => ({
      busySessions: s.busySessions.includes(sessionId) ? s.busySessions : [...s.busySessions, sessionId],
    })),
  markSessionIdle: (sessionId) =>
    set((s) => ({ busySessions: s.busySessions.filter((id) => id !== sessionId) })),


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
  dockWidth: defaultConfig.dockWidth,
  setDockWidth: (dockWidth) => set({ dockWidth }),
  dockTab: "files",
  setDockTab: (dockTab) => set({ dockTab }),

  previewTabs: [],
  activePreviewId: null,
  openFilePreview: (path, name) => {
    set((s) => {
      const existing = s.previewTabs.find(
        (t) => t.kind === "file" && t.path === path,
      );
      if (existing) {
        return { activePreviewId: existing.id };
      }
      const tab: PreviewTab = {
        id: uuid(),
        kind: "file",
        name: name || path.split(/[\\/]/).pop() || path,
        path,
      };
      return { previewTabs: [...s.previewTabs, tab], activePreviewId: tab.id };
    });
  },
  openDiffPreview: (name, path, diff) => {
    set((s) => {
      const existing = s.previewTabs.find(
        (t) => t.kind === "diff" && t.path === path,
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
    try {
      const cfg = get().config;
      const cwd = session.workingDirectory || cfg.workingDirectory || ".";
      const running = await window.api.acp.isRunning();
      if (!running) {
        if (!cfg.acpPath) throw new Error("ACP path not configured. Go to Settings.");
        await window.api.acp.start(cfg.acpPath, cwd);
      }
      // Real resume: session/load replays the full history via events
      await window.api.acp.loadSession(acpSessionId, cwd);
      set({ activeAcpSessionId: acpSessionId });
      // Safety net: clear flags if no history event arrives (empty session)
      window.setTimeout(() => {
        const st = useStore.getState();
        if (st.pendingLoadSessionId === acpSessionId) st.setPendingLoadSessionId(null);
        const sess = st.sessions.find((x) => x.id === localSessionId);
        if (sess?.loading) st.updateSession(localSessionId, { loading: false });
      }, 2500);
    } catch (err) {
      console.error("[store] ACP session/load failed, falling back to vibe logs:", err);
      get().setPendingLoadSessionId(null);
      get().updateSession(localSessionId, { loading: false });
      await get().loadVibeSessionMessages(localSessionId);
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
      const isRunning = await window.api.acp.isRunning();
      if (!isRunning) {
        const cfg = get().config;
        if (!cfg.acpPath) return;
        const cwd = session.workingDirectory || cfg.workingDirectory || ".";
        await window.api.acp.start(cfg.acpPath, cwd);
      }

      const summary = await window.api.acp.getContextSummary(vibeMessages);

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
}));
