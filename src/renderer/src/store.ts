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
} from "./types";

import { defaultConfig } from "./types";

function uuid(): string {
  return crypto.randomUUID();
}

function makeSessionName(index: number): string {
  return `Session ${index}`;
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

  // ACP state
  acpStatus: AcpStatus;
  setAcpStatus: (status: AcpStatus) => void;
  acpError: string | null;
  setAcpError: (err: string | null) => void;

  streamingMessage: string | null;
  // set of local session IDs currently receiving AI chunks
  streamingSessions: string[];
  // set of local session IDs where agent is actively working (tool calls, thinking, streaming)
  busySessions: string[];
  markSessionBusy: (sessionId: string) => void;
  markSessionIdle: (sessionId: string) => void;

  currentModel: string;
  availableModels: string[];
  currentMode: string;
  availableModes: Array<{ id: string; name: string }>;
  availableCommands: Array<{ name: string; description: string; hint?: string }>;

  setCurrentModel: (model: string) => void;
  setAvailableModels: (models: string[]) => void;
  setCurrentMode: (mode: string) => void;
  setAvailableModes: (modes: Array<{ id: string; name: string }>) => void;
  setAvailableCommands: (commands: Array<{ name: string; description: string; hint?: string }>) => void;

  workingDirectory: string;
  setWorkingDirectory: (dir: string) => void;

  sandboxEnabled: boolean;
  setSandboxEnabled: (v: boolean) => void;

  // Pickers
  showModePicker: boolean;
  showModelPicker: boolean;
  setShowModePicker: (v: boolean) => void;
  setShowModelPicker: (v: boolean) => void;

  // Message queue for pending messages while agent is working
  messageQueue: Array<{ id: string; content: string; timestamp: number }>;
  addToMessageQueue: (content: string) => void;
  removeFromMessageQueue: (id: string) => void;
  updateQueueMessage: (id: string, content: string) => void;
  clearMessageQueue: () => void;
  getNextQueuedMessage: () => { id: string; content: string } | undefined;

  // Session-specific working directory
  currentSessionWorkingDirectory: () => string;

  // Actions
  addUserMessage: (text: string, attachments?: { path: string; isDir: boolean; content: string }[]) => Message;
  startAiMessage: (sessionId: string) => string;
  appendChunk: (sessionId: string, messageId: string, chunk: string) => void;
  finalizeAiMessage: (sessionId: string, messageId: string) => void;
  updateToolCall: (sessionId: string, toolCall: ToolCall) => void;
  mergeToolCall: (sessionId: string, toolCallId: string, updates: Partial<ToolCall>) => void;
  startThinking: (
    sessionId: string,
    messageId: string,
    blockId: string,
  ) => void;
  appendThinkingChunk: (
    sessionId: string,
    messageId: string,
    blockId: string,
    chunk: string,
  ) => void;
  finalizeThinking: (
    sessionId: string,
    messageId: string,
    blockId: string,
  ) => void;
  linkAcpSession: (localSessionId: string, acpSessionId: string) => void;
  getSessionByAcpId: (acpSessionId: string) => Session | null;
  importAcpSessions: (
    acpSessions: Array<{
      sessionId: string;
      name: string;
      cwd: string;
      startTime: string;
    }>,
  ) => void;
  loadVibeSessionMessages: (localSessionId: string) => Promise<void>;
  fetchContextSummary: (localSessionId: string) => Promise<void>;
}

export const useStore = create<AppStore>((set, get) => ({
  config: defaultConfig,
  setConfig: (config) => set({ config, theme: config.theme }),
  addRecentFolder: (folder: string) => {
    const cfg = get().config;
    const updated = [
      folder,
      ...cfg.recentFolders.filter((f) => f !== folder),
    ].slice(0, 5);
    set({ config: { ...cfg, recentFolders: updated } });
  },

  theme: 'auto',
  toggleTheme: () => set((s) => {
    const order: ThemeMode[] = ['auto', 'dark', 'light']
    const idx = order.indexOf(s.theme)
    const newTheme = order[(idx + 1) % order.length]
    return {
      theme: newTheme,
      config: { ...s.config, theme: newTheme }
    }
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
  addSession: (workingDir?: string) => {
    const { sessions, workingDirectory } = get();
    const session: Session = {
      id: uuid(),
      name: makeSessionName(sessions.length + 1),
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
      sessions: s.sessions.map((sess) =>
        sess.id === id ? { ...sess, ...updates } : sess,
      ),
    }));
  },

  acpStatus: "disconnected",
  setAcpStatus: (acpStatus) => set({ acpStatus }),
  acpError: null,
  setAcpError: (acpError) => set({ acpError }),

  streamingMessage: null,
  streamingSessions: [],
  busySessions: [],
  markSessionBusy: (sessionId) => set((s) => ({
    busySessions: s.busySessions.includes(sessionId) ? s.busySessions : [...s.busySessions, sessionId],
  })),
  markSessionIdle: (sessionId) => set((s) => ({
    busySessions: s.busySessions.filter((id) => id !== sessionId),
  })),

  currentModel: "",
  availableModels: [],
  currentMode: "default",
  availableModes: [],
  availableCommands: [],

  setCurrentModel: (currentModel) => set({ currentModel }),
  setAvailableModels: (availableModels) => set({ availableModels }),
  setCurrentMode: (currentMode) => set({ currentMode }),
  setAvailableModes: (availableModes) => set({ availableModes }),
  setAvailableCommands: (availableCommands) => set({ availableCommands }),

  workingDirectory: "",
  setWorkingDirectory: (workingDirectory) => {
    const cfg = get().config;
    set({ workingDirectory, config: { ...cfg, workingDirectory } });
    window.api.config.save({ ...cfg, workingDirectory }).catch(console.error);
  },

  sandboxEnabled: false,
  setSandboxEnabled: (sandboxEnabled) => set({ sandboxEnabled }),

  showModePicker: false,
  showModelPicker: false,
  setShowModePicker: (v) => set({ showModePicker: v }),
  setShowModelPicker: (v) => set({ showModelPicker: v }),

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
      messageQueue: s.messageQueue.map((m) =>
        m.id === id ? { ...m, content } : m
      ),
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
        // Auto-name session from first user message
        const name =
          sess.messages.length === 0
            ? text.slice(0, 40) + (text.length > 40 ? "…" : "")
            : sess.name;
        updateSession(currentSessionId, {
          messages: [...sess.messages, msg],
          lastActive: Date.now(),
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
        return { ...sess, messages, lastActive: Date.now() };
      });
      return {
        sessions,
        streamingMessage: null,
        streamingSessions: s.streamingSessions.filter((id) => id !== sessionId),
      };
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
    }));
  },

  getSessionByAcpId: (acpSessionId) => {
    return get().sessions.find((s) => s.acpSessionId === acpSessionId) ?? null;
  },

  importAcpSessions: (acpSessions) => {
    set((s) => {
      const existing = new Set(
        s.sessions.map((sess) => sess.acpSessionId).filter(Boolean),
      );
      const toAdd: Session[] = acpSessions
        .filter((a) => a.sessionId && !existing.has(a.sessionId))
        .map((a) => {
          const startTimeMs = a.startTime ? new Date(a.startTime).getTime() : Date.now()
          return {
            id: uuid(),
            name: a.name || a.sessionId.slice(0, 16) + "…",
            acpSessionId: a.sessionId,
            messages: [],
            createdAt: startTimeMs,
            lastActive: startTimeMs,
            startTime: startTimeMs,
            workingDirectory: a.cwd || undefined,
          }
        })
      if (toAdd.length === 0) return {};
      // Merge and sort by lastActive desc so newest is first
      const merged = [...s.sessions, ...toAdd].sort(
        (a, b) => b.lastActive - a.lastActive,
      );
      return { sessions: merged };
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
          const existing = (msg.toolCalls ?? []).findIndex(
            (tc) => tc.id === toolCall.id,
          );
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
          if (m.sender !== 'ai') return m;
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

  loadVibeSessionMessages: async (localSessionId) => {
    const session = get().sessions.find((s) => s.id === localSessionId);
    if (!session?.acpSessionId) return;

    try {
      const vibeMessages = await window.api.stats.vibeSessionMessages(session.acpSessionId);

      const messages: Message[] = [];
      let pendingToolCalls: Map<string, ToolCall> = new Map();

      // Parse attachments from historical user message content.
      // Handles multiple formats produced by different vibe-acp versions:
      // Formats supported (newest first):
      //   XML (new):  <vibe_attach_file><name>x</name><location>path</location><content>...</content></vibe_attach_file>
      //               <vibe_attach_directory><name>x</name><location>path</location><content>...</content></vibe_attach_directory>
      //   Format A:   @path\n\nFile: path\nContent:\n```\ncontent\n```
      //   Format B:   path\n\nFile: path\nContent:\n```\ncontent\n```
      //   Format C:   path \n\nfile:///full_path\n```\ncontent\n```
      const parseHistoricalAttachments = (
        content: string
      ): { text: string; attachments: { path: string; isDir: boolean; content: string }[] } => {
        const attachments: { path: string; isDir: boolean; content: string }[] = []
        const escRx = (s: string): string => s.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')

        let text = content
        const markers: Array<{ marker: string; path: string; isDir: boolean }> = []
        let idx = 0
        const mkMarker = (): string => `\x00ATT${idx++}\x00`

        // --- New XML format (highest priority) ---
        // <vibe_attach_file><name>…</name><location>…</location><content>…</content></vibe_attach_file>
        text = text.replace(
          /<vibe_attach_file><name>([^<]*)<\/name><location>([^<]*)<\/location><content>([\s\S]*?)<\/content><\/vibe_attach_file>/g,
          (_match, name: string, location: string, fileContent: string) => {
            const p = location.trim() || name.trim()
            const marker = mkMarker()
            attachments.push({ path: p, isDir: false, content: fileContent })
            markers.push({ marker, path: p, isDir: false })
            return marker
          }
        )
        // <vibe_attach_directory>…</vibe_attach_directory>
        text = text.replace(
          /<vibe_attach_directory><name>([^<]*)<\/name><location>([^<]*)<\/location><content>([\s\S]*?)<\/content><\/vibe_attach_directory>/g,
          (_match, name: string, location: string, dirContent: string) => {
            const p = location.trim() || name.trim()
            const marker = mkMarker()
            attachments.push({ path: p, isDir: true, content: dirContent })
            markers.push({ marker, path: p, isDir: true })
            return marker
          }
        )

        // --- Legacy formats (backwards compatibility) ---
        // File blocks: "File: path\nContent:\n```\ncontent\n```"
        text = text.replace(
          /File: ([^\n]+)\nContent:\n```[^\n]*\n([\s\S]*?)```/g,
          (_match, filePath: string, fileContent: string) => {
            const p = filePath.trim()
            const marker = mkMarker()
            attachments.push({ path: p, isDir: false, content: fileContent })
            markers.push({ marker, path: p, isDir: false })
            return marker
          }
        )
        // Directory blocks: "Directory: path\nContents:\n```\ncontent\n```"
        text = text.replace(
          /Directory: ([^\n]+)\nContents:\n```[^\n]*\n([\s\S]*?)```/g,
          (_match, dirPath: string, dirContent: string) => {
            const p = dirPath.trim()
            const marker = mkMarker()
            attachments.push({ path: p, isDir: true, content: dirContent })
            markers.push({ marker, path: p, isDir: true })
            return marker
          }
        )
        // file:/// URI blocks: "file:///path\n```\ncontent\n```"
        text = text.replace(
          /file:\/\/\/([^\n]+)\n```[^\n]*\n([\s\S]*?)```/g,
          (_match, uriPath: string, fileContent: string) => {
            const p = uriPath.trim().split(/[\\/]/).pop() ?? uriPath.trim()
            const marker = mkMarker()
            attachments.push({ path: p, isDir: false, content: fileContent })
            markers.push({ marker, path: p, isDir: false })
            return marker
          }
        )

        // Remove orphaned header lines that preceded legacy blocks
        for (const { marker, path } of markers) {
          const pe = escRx(path)
          const me = escRx(marker)
          text = text.replace(
            new RegExp(`@?[ \\t]*${pe}[ \\t]*\\n\\n(?=${me})`, 'g'),
            ''
          )
        }

        // Replace all markers with @path references
        for (const { marker, path } of markers) {
          const me = escRx(marker)
          text = text.replace(new RegExp(me, 'g'), `@${path}`)
        }

        return { text: text.trim(), attachments }
      }

      for (const vm of vibeMessages) {
        if (vm.role === 'user') {
          // Skip injected system context messages
          if (vm.injected) continue
          const raw = vm.content ?? ''
          const { text, attachments } = parseHistoricalAttachments(raw)
          messages.push({
            id: vm.message_id,
            sender: 'user' as const,
            content: text,
            isComplete: true,
            timestamp: Date.now(),
            attachments: attachments.length > 0 ? attachments : undefined,
          });
        } else if (vm.role === 'assistant') {
          const toolCalls: ToolCall[] = (vm.tool_calls ?? []).map((tc) => {
            const toolCall: ToolCall = {
              id: tc.id,
              title: tc.function.name,
              kind: 'execute',
              status: 'done' as const,
              input: (() => {
                try {
                  return JSON.parse(tc.function.arguments);
                } catch {
                  return {};
                }
              })(),
            };
            pendingToolCalls.set(tc.id, toolCall);
            return toolCall;
          });

          const thinking = vm.reasoning_content
            ? [{
                id: vm.reasoning_message_id ?? 'thinking',
                content: vm.reasoning_content,
                isComplete: true,
                startedAt: Date.now(),
                completedAt: Date.now(),
              }]
            : undefined;

          const content = vm.content ?? '';
          if (content || toolCalls.length > 0 || thinking) {
            messages.push({
              id: vm.message_id,
              sender: 'ai' as const,
              content,
              isComplete: true,
              timestamp: Date.now(),
              toolCalls: toolCalls.length > 0 ? toolCalls : undefined,
              thinking,
            });
          }
        } else if (vm.role === 'tool') {
          const toolCallId = vm.tool_call_id;
          const rawContent = vm.content ?? '';

          // Parse ACP JSON content-event array into plain text for display and error detection
          let parsedContent = rawContent;
          let hasErrorItem = false;
          try {
            const parsed = JSON.parse(rawContent);
            if (Array.isArray(parsed)) {
              hasErrorItem = parsed.some((e: Record<string, unknown>) =>
                e.type === 'tool_error' || e.is_error === true
              );
              const texts = parsed
                .filter((e: Record<string, unknown>) =>
                  (e.type === 'content' || e.type === 'tool_error') &&
                  typeof (e.content as Record<string, unknown>)?.text === 'string'
                )
                .map((e: Record<string, unknown>) => (e.content as Record<string, unknown>).text as string);
              if (texts.length > 0) parsedContent = texts.join('\n');
            }
          } catch { /* not JSON, use rawContent as-is */ }

          const isError = hasErrorItem || parsedContent.includes('<tool_error>');

          if (toolCallId && pendingToolCalls.has(toolCallId)) {
            // Standard matching by tool_call_id
            const toolCall = pendingToolCalls.get(toolCallId)!;
            toolCall.output = parsedContent;
            toolCall.status = isError ? 'error' : 'done';
            pendingToolCalls.delete(toolCallId);
          } else if (isError && vm.name) {
            // Fallback: try to match by tool name for error cases
            // This handles historical sessions where IDs might not match
            for (const [id, tc] of pendingToolCalls) {
              if (tc.title === vm.name) {
                tc.output = parsedContent;
                tc.status = 'error';
                pendingToolCalls.delete(id);
                break;
              }
            }
          }
        }
      }

      // Mark any remaining pending tool calls with error output as failed
      for (const [, tc] of pendingToolCalls) {
        if (tc.output?.includes('<tool_error>')) {
          tc.status = 'error';
        }
      }

      get().updateSession(localSessionId, { messages });
    } catch (err) {
      console.error('Failed to load vibe session messages:', err);
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
        const cwd = session.workingDirectory || cfg.workingDirectory || '';
        await window.api.acp.start(cfg.acpPath, cwd, get().currentModel || '');
      }

      const summary = await window.api.acp.getContextSummary(vibeMessages);

      if (summary && summary.trim()) {
        const contextMessage: Message = {
          id: uuid(),
          sender: 'system',
          content: `Previous conversation summary:\n${summary.trim()}`,
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
      console.error('Failed to fetch context summary:', err);
    }
  },
}));
