import React, { useState, useRef, useCallback, useEffect } from "react";
import { useStore } from "../store";
import { useTranslation } from "../i18n";
import type { AppConfig } from "../types";

interface Props {
  config: AppConfig;
}

interface DirEntry {
  name: string;
  isDir: boolean;
}

interface SlashItem {
  id: string;
  label: string;
  kind: 'mode' | 'skill' | 'prompt' | 'builtin' | 'command';
  content?: string; // for skill/prompt insertion
  description?: string;
}

const SendIcon = (): React.JSX.Element => (
  <svg
    xmlns="http://www.w3.org/2000/svg"
    width="14"
    height="14"
    viewBox="0 0 24 24"
    fill="currentColor"
    style={{ transform: "rotate(-90deg)" }}
  >
    <path d="M12 18v4h4v-4h-4ZM16 14v4h4v-4h-4ZM20 10v4h4v-4h-4ZM16 6v4h4V6h-4ZM12 2v4h4V2h-4ZM12 10v4h4v-4h-4ZM8 10v4h4v-4H8ZM4 10v4h4v-4H4ZM0 10v4h4v-4H0Z" />
  </svg>
);

const StopIcon = (): React.JSX.Element => (
  <svg xmlns="http://www.w3.org/2000/svg" width="12" height="12" viewBox="0 0 24 24" fill="currentColor">
    <rect x="4" y="4" width="16" height="16" rx="2" />
  </svg>
);

export default function InputBar({ config }: Props): React.JSX.Element {
  const {
    acpStatus,
    currentModel,
    availableModels,
    currentMode,
    availableModes,
    availableCommands,
    workingDirectory,
    setWorkingDirectory,
    sandboxEnabled,
    setSandboxEnabled,
    showModePicker,
    setShowModePicker,
    setCurrentMode,
    setCurrentModel,
    addUserMessage,
    setAcpStatus,
    setAcpError,
    currentSession,
    currentSessionWorkingDirectory,
    addSession,
    sessions,
    currentSessionId,
    updateSession,
    pendingNewSession,
    setPendingNewSession,
    sessionStarting,
    setSessionStarting,
    addRecentFolder,
    streamingSessions,
    busySessions,
    markSessionBusy,
    addToMessageQueue,
    messageQueue,
  } = useStore();

  const tr = useTranslation(config.language);
  const storeConfig = config;

  const [inputText, setInputText] = useState("");
  const [isStarting, setIsStarting] = useState(false);

  // @ file picker
  const [atQuery, setAtQuery] = useState("");
  const [atFiles, setAtFiles] = useState<Array<{ path: string; isDir: boolean }>>([]);
  const [showAtPicker, setShowAtPicker] = useState(false);
  const [atPickerIdx, setAtPickerIdx] = useState(0);

  // / slash picker (modes + skills + prompts)
  const [slashQuery, setSlashQuery] = useState("");
  const [showSlashPicker, setShowSlashPicker] = useState(false);
  const [slashPickerIdx, setSlashPickerIdx] = useState(0);
  const [slashSkills, setSlashSkills] = useState<{ name: string; content: string }[]>([]);
  const [slashPrompts, setSlashPrompts] = useState<{ name: string; content: string }[]>([]);

  // Attached images (pasted)
  const [pastedImages, setPastedImages] = useState<
    { name: string; url: string }[]
  >([]);
  const [showFolderPicker, setShowFolderPicker] = useState(false);

  const editorRef = useRef<HTMLDivElement>(null);
  const modePickerRef = useRef<HTMLDivElement>(null);
  const atPickerRef = useRef<HTMLDivElement>(null);
  const folderPickerRef = useRef<HTMLDivElement>(null);

  // Close mode picker on outside click
  useEffect(() => {
    const handler = (e: MouseEvent): void => {
      if (
        modePickerRef.current &&
        !modePickerRef.current.contains(e.target as Node)
      ) {
        setShowModePicker(false);
      }
      if (
        atPickerRef.current &&
        !atPickerRef.current.contains(e.target as Node)
      ) {
        setShowAtPicker(false);
        setShowSlashPicker(false);
      }
      if (
        folderPickerRef.current &&
        !folderPickerRef.current.contains(e.target as Node)
      ) {
        setShowFolderPicker(false);
      }
    };
    document.addEventListener("mousedown", handler);
    return () => document.removeEventListener("mousedown", handler);
  }, [setShowModePicker]);

  // Auto-resize textarea
  useEffect(() => {
    if (editorRef.current) {
      editorRef.current.style.height = "auto";
      editorRef.current.style.height = `${Math.min(editorRef.current.scrollHeight, 160)}px`;
    }
  }, [inputText]);

  // Load files when @ picker is active (uses recursive search)
  useEffect(() => {
    if (!showAtPicker) return;
    const dir =
      currentSessionWorkingDirectory() || config.workingDirectory || "";
    if (!dir) {
      setAtFiles([]);
      return;
    }
    window.api.fs.searchFiles(dir, atQuery).then((entries) => {
      setAtFiles(entries);
      setAtPickerIdx(0);
    });
  }, [showAtPicker, atQuery, currentSessionWorkingDirectory, config.workingDirectory]);

  // Load skills and prompts once when slash picker first opens
  useEffect(() => {
    if (!showSlashPicker) return;
    Promise.all([
      window.api.vibe.getSkills().catch(() => []),
      window.api.vibe.getPrompts().catch(() => []),
    ]).then(([skills, prompts]) => {
      setSlashSkills(skills);
      setSlashPrompts(prompts);
    });
  }, [showSlashPicker]);

  const filteredAtFiles = atFiles;

  // / picker shows available commands from ACP, skills and prompts (modes are selected via the toolbar mode button)
  const allSlashItems: SlashItem[] = [
    ...availableCommands.map((c) => ({ id: `cmd:${c.name}`, label: c.name, kind: 'command' as const, content: `/${c.name}${c.hint ? ' ' + c.hint : ''}`, description: c.description })),
    ...slashSkills.map((s) => ({ id: `skill:${s.name}`, label: s.name, kind: 'skill' as const, content: s.content })),
    ...slashPrompts.map((p) => ({ id: `prompt:${p.name}`, label: p.name, kind: 'prompt' as const, content: p.content })),
  ];

  const filteredSlashItems = allSlashItems.filter(
    (item) =>
      item.label.toLowerCase().includes(slashQuery.toLowerCase()),
  );

  // Insert file/folder reference at cursor position in contenteditable (just the @path marker, no content)
  const insertAtFile = useCallback(
    async (entry: { path: string; isDir: boolean }) => {
      const editor = editorRef.current;
      if (!editor) return;

      const selection = window.getSelection();
      if (!selection || selection.rangeCount === 0) return;

      const range = selection.getRangeAt(0);

      // Collect all text before cursor
      const allTextBefore = getTextBeforeCursor(editor, range);
      const atIdx = allTextBefore.lastIndexOf('@');
      if (atIdx === -1) return;

      setShowAtPicker(false);

      // Build the @path marker - trailing slash for directories
      const pathSuffix = entry.isDir && !entry.path.endsWith('/') ? '/' : '';
      const insertText = `@${entry.path}${pathSuffix}`;

      // Find and delete from @ to cursor, then insert the marker
      let charCount = 0;
      let atNode: Text | null = null;
      let atNodeOffset = 0;

      const walker = document.createTreeWalker(editor, NodeFilter.SHOW_TEXT, null);
      let node: Node | null = walker.nextNode();
      while (node) {
        const nodeText = node.textContent ?? '';
        const nextCount = charCount + nodeText.length;
        if (charCount <= atIdx && atIdx < nextCount) {
          atNode = node as Text;
          atNodeOffset = atIdx - charCount;
          break;
        }
        charCount = nextCount;
        node = walker.nextNode();
      }

      if (!atNode) return;

      // Create range to delete from @ to cursor
      const deleteRange = document.createRange();
      const cursorTextNode = range.startContainer.nodeType === Node.TEXT_NODE ? range.startContainer as Text : null;

      if (atNode === cursorTextNode) {
        deleteRange.setStart(atNode, atNodeOffset);
        deleteRange.setEnd(cursorTextNode!, range.startOffset);
      } else {
        deleteRange.setStart(atNode, atNodeOffset);
        deleteRange.setEnd(range.startContainer, range.startOffset);
      }

      deleteRange.deleteContents();

      // Insert the marker text
      const textNode = document.createTextNode(insertText);
      range.insertNode(textNode);

      // Move cursor after the inserted text
      range.setStartAfter(textNode);
      range.collapse(true);
      selection.removeAllRanges();
      selection.addRange(range);

      // Update inputText state
      setInputText(editor.textContent ?? '');
    },
    [editorRef],
  );

  // Update inputText state from editor content, preserving file link paths
  const updateInputTextFromEditor = useCallback(() => {
    const editor = editorRef.current;
    if (!editor) return;

    // Get all text, but for file links use their data-path instead of display text
    const result: string[] = [];
    const walker = document.createTreeWalker(editor, NodeFilter.SHOW_ELEMENT, null);
    
    while (walker.nextNode()) {
      const node = walker.currentNode as HTMLElement;
      if (node.classList?.contains('file-link')) {
        const path = node.dataset.path;
        result.push(path ?? node.textContent ?? '');
      }
    }
    
    // Also get text from non-element nodes (direct text children)
    for (const child of Array.from(editor.childNodes)) {
      if (child.nodeType === Node.TEXT_NODE) {
        result.push(child.textContent ?? '');
      } else if (child.nodeType === Node.ELEMENT_NODE) {
        const el = child as HTMLElement;
        if (!el.classList?.contains('file-link')) {
          // For non-file-link elements, recursively get text
          const tempDiv = document.createElement('div');
          tempDiv.appendChild(child.cloneNode(true));
          result.push(tempDiv.textContent ?? '');
        }
      }
    }
    
    setInputText(result.join(''));
  }, [editorRef]);

  // Helper to get all text content before a node
  const getTextNodesBefore = (root: Node, targetNode: Node, targetOffset: number): string => {
    let result = '';
    const walker = document.createTreeWalker(root, NodeFilter.SHOW_TEXT, null);
    let node: Node | null = walker.nextNode();
    while (node) {
      if (node === targetNode) {
        result += (node.textContent ?? '').slice(0, targetOffset);
        break;
      }
      result += node.textContent ?? '';
      node = walker.nextNode();
    }
    return result;
  };

  // Replace the current /token with selected mode
  const insertSlashMode = useCallback(
    (mode: { id: string; name: string }) => {
      const editor = editorRef.current;
      if (!editor) return;

      // Find and remove /token before cursor
      const textContent = editor.textContent ?? '';
      const selection = window.getSelection();
      if (!selection || selection.rangeCount === 0) return;

      const range = selection.getRangeAt(0);
      const beforeCursor = getTextBeforeCursor(editor, range);

      const slashIdx = beforeCursor.lastIndexOf("/");
      if (slashIdx === -1) return;

      // Delete the /token
      const deleteRange = document.createRange();
      // We need to find the range from slashIdx to cursor
      const beforeSlash = textContent.slice(0, slashIdx);
      const afterSlash = textContent.slice(beforeCursor.length);
      
      editor.textContent = beforeSlash + afterSlash;
      
      // Set cursor position
      const newRange = document.createRange();
      const textNode = editor.firstChild;
      if (textNode) {
        newRange.setStart(textNode, slashIdx);
        newRange.collapse(true);
        selection.removeAllRanges();
        selection.addRange(newRange);
      }

      setShowSlashPicker(false);
      setCurrentMode(mode.id);
      if (acpStatus === "connected")
        window.api.acp.setMode(mode.id).catch(console.error);
    },
    [acpStatus, setCurrentMode],
  );

  const insertSlashItem = useCallback(
    (item: SlashItem) => {
      const editor = editorRef.current;
      if (!editor) return;

      const selection = window.getSelection();
      if (!selection || selection.rangeCount === 0) return;

      const range = selection.getRangeAt(0);
      const beforeCursor = getTextBeforeCursor(editor, range);
      const slashIdx = beforeCursor.lastIndexOf("/");
      if (slashIdx === -1) return;

      const content = item.content ?? '';
      const textContent = editor.textContent ?? '';
      const afterCursor = textContent.slice(beforeCursor.length);

      editor.textContent = textContent.slice(0, slashIdx) + content + (content.endsWith('\n') ? '' : '\n') + afterCursor;

      // Move cursor after inserted content
      const newPos = slashIdx + content.length + 1;
      const newRange = document.createRange();
      const textNode = editor.firstChild;
      if (textNode && textNode.textContent && newPos <= textNode.textContent.length) {
        newRange.setStart(textNode, newPos);
        newRange.collapse(true);
        selection.removeAllRanges();
        selection.addRange(newRange);
      }

      setShowSlashPicker(false);
    },
    [],
  );

  // Helper to get text before cursor in contenteditable
  const getTextBeforeCursor = (editor: Node, range: Range): string => {
    const textNodes: string[] = [];
    const walker = document.createTreeWalker(editor, NodeFilter.SHOW_TEXT, null);
    let node: Node | null = walker.nextNode();
    while (node) {
      if (node === range.startContainer) {
        textNodes.push((node.textContent ?? '').slice(0, range.startOffset));
        break;
      }
      textNodes.push(node.textContent ?? '');
      node = walker.nextNode();
    }
    return textNodes.join('');
  };

  // Handle input in contenteditable
  const handleEditorInput = (): void => {
    const editor = editorRef.current;
    if (!editor) return;

    const value = editor.textContent ?? '';
    setInputText(value);

    // Get cursor position
    const selection = window.getSelection();
    if (!selection || selection.rangeCount === 0) return;

    const range = selection.getRangeAt(0);
    const beforeCursor = getTextBeforeCursor(editor, range);

    // @ file picker detection
    const atMatch = beforeCursor.match(/@([^\s@]*)$/);
    if (atMatch) {
      setAtQuery(atMatch[1]);
      setShowAtPicker(true);
      setShowSlashPicker(false);
    } else {
      setShowAtPicker(false);
    }

    // / slash picker detection (at start or after space/newline)
    const slashMatch = beforeCursor.match(/(?:^|[\s\n])\/([^\s/]*)$/);
    if (slashMatch) {
      setSlashQuery(slashMatch[1]);
      setShowSlashPicker(true);
      setShowAtPicker(false);
    } else {
      setShowSlashPicker(false);
    }
  };

  const handleSend = useCallback(async () => {
    const editor = editorRef.current;
    let textToSend = inputText.trim();
    
    if (editor) {
      const clone = editor.cloneNode(true) as HTMLElement;
      const fileLinks = clone.querySelectorAll('.file-link');
      fileLinks.forEach((link) => {
        const el = link as HTMLElement;
        const path = el.dataset.path ?? el.textContent ?? '';
        const textNode = document.createTextNode('@' + path);
        link.replaceWith(textNode);
      });
      textToSend = clone.textContent ?? '';
    }
    
    if (!textToSend && pastedImages.length === 0) return;

    // Extract @path markers and fetch file/folder contents for ACP
    const atPathRegex = /@([^\s]+)/g;
    const cwd = currentSessionWorkingDirectory() || config.workingDirectory || '';
    const pathMatches = [...textToSend.matchAll(atPathRegex)];
    let expandedText = textToSend;
    const attachments: { path: string; isDir: boolean; content: string }[] = [];

    if (pathMatches.length > 0) {
      const fetchPromises = pathMatches.map(async (match) => {
        const rawPath = match[1];
        const isDir = rawPath.endsWith('/');
        const path = isDir ? rawPath.slice(0, -1) : rawPath;
        
        const fullPath = path.startsWith('/') || path.match(/^[A-Za-z]:/)
          ? path
          : `${cwd}/${path}`.replace(/\\/g, '/');

        try {
          const result = await window.api.fs.readFile(fullPath);
          return { original: match[0], result, isDir, path: rawPath };
        } catch {
          return { original: match[0], result: { type: 'error' as const, content: '' }, isDir, path: rawPath };
        }
      });

      const results = await Promise.all(fetchPromises);
      
      for (const { original, result, isDir, path: rawPath } of results) {
        const name = rawPath.replace(/\/$/, '').split(/[/\\]/).pop() ?? rawPath
        const location = isDir ? rawPath : rawPath
        if (result.type === 'file') {
          attachments.push({ path: rawPath, isDir: false, content: result.content });
          expandedText = expandedText.replace(
            original,
            `<vibe_attach_file><name>${name}</name><location>${location}</location><content>${result.content}</content></vibe_attach_file>`
          );
        } else if (result.type === 'directory') {
          attachments.push({ path: rawPath, isDir: true, content: result.content });
          expandedText = expandedText.replace(
            original,
            `<vibe_attach_directory><name>${name}</name><location>${location}</location><content>${result.content}</content></vibe_attach_directory>`
          );
        }
      }
    }

    if (pendingNewSession || sessionStarting) {
      // First message on a draft session — create ACP session now
      setPendingNewSession(false);
      setSessionStarting(true);
      setIsStarting(true);
      try {
        if (acpStatus === "connected") {
          // ACP running — just open a new session
          setAcpStatus("connecting");
          await window.api.acp.newSession(cwd, currentModel || "");
        } else {
          // ACP not running — start it (start() calls newSession() internally)
          if (!config.acpPath) {
            setAcpError("ACP path not configured. Go to Settings.");
            setIsStarting(false);
            setPendingNewSession(true);
            return;
          }
          setAcpStatus("connecting");
          await window.api.acp.start(config.acpPath, cwd, currentModel || "");
        }
      } catch (err) {
        setAcpStatus("error");
        setAcpError((err as Error).message);
        setIsStarting(false);
        setPendingNewSession(true);
        return;
      }
      setIsStarting(false);
      // SessionInitialized has fired by now (sent before IPC resolve) → currentSessionId is set
      setInputText("");
      if (editorRef.current) editorRef.current.textContent = "";
      setPastedImages([]);
      addUserMessage(textToSend, attachments);
      try {
        await window.api.acp.sendPrompt(expandedText);
      } catch (err) {
        setAcpError((err as Error).message);
      }
      return;
    }

    // Existing session - continue in same ACP session
    const sessionWd =
      currentSessionWorkingDirectory() || config.workingDirectory || "";

    const currentSess = currentSession();
    const acpSessId = currentSess?.acpSessionId;

    if (acpStatus === "disconnected" || acpStatus === "error") {
      if (!config.acpPath) {
        setAcpError("ACP path not configured. Go to Settings.");
        return;
      }
      setIsStarting(true);
      setAcpStatus("connecting");
      try {
        await window.api.acp.start(
          config.acpPath,
          sessionWd,
          currentModel || "",
        );
      } catch (err) {
        setAcpStatus("error");
        setAcpError((err as Error).message);
        setIsStarting(false);
        return;
      }
      setIsStarting(false);
    }

    setInputText("");
    if (editorRef.current) editorRef.current.textContent = "";
    setPastedImages([]);

    // If agent is currently busy (streaming or tool calls), add to queue instead of sending
    const isCurrentSessionBusy =
      currentSessionId != null && busySessions.includes(currentSessionId);
    if (isCurrentSessionBusy) {
      addToMessageQueue(expandedText);
      return;
    }

    addUserMessage(textToSend, attachments);
    const promptText = currentSess?.contextSummary
      ? `${currentSess.contextSummary}\n\nUser: ${expandedText}`
      : expandedText;
    try {
      // Pass explicit ACP session ID so the message goes to the correct session
      // even when user has switched between sessions in the sidebar
      await window.api.acp.sendPrompt(promptText, acpSessId);
    } catch (err) {
      const errMsg = (err as Error).message;
      // Session not in ACP memory (e.g. app restarted) — try to load from disk first
      const sessionExpired =
        errMsg.includes("No active session") ||
        errMsg.includes("Session not found");
      if (sessionExpired) {
        try {
          setSessionStarting(true);
          if (acpSessId) {
            // Try to load the historical session from disk (preserves full conversation context)
            try {
              await window.api.acp.loadSession(acpSessId, sessionWd, currentModel || "");
              // SessionInitialized fired — session is now in ACP memory with full history
              await window.api.acp.sendPrompt(promptText, acpSessId);
            } catch (loadErr) {
              const loadMsg = (loadErr as Error).message;
              // session/load failed (session log missing) → fall back to new session
              console.log("[InputBar] loadSession failed:", loadMsg, "— creating new session");
              await window.api.acp.newSession(sessionWd, currentModel || "");
              await window.api.acp.sendPrompt(promptText);
            }
          } else {
            await window.api.acp.newSession(sessionWd, currentModel || "");
            await window.api.acp.sendPrompt(promptText);
          }
          setSessionStarting(false);
        } catch (err2) {
          setSessionStarting(false);
          setAcpError((err2 as Error).message);
        }
      } else {
        setAcpError(errMsg);
      }
    }
  }, [
    inputText,
    pastedImages,
    acpStatus,
    config,
    currentSessionWorkingDirectory,
    currentModel,
    currentSession,
    addSession,
    addUserMessage,
    setAcpStatus,
    setAcpError,
    pendingNewSession,
    setPendingNewSession,
  ]);

  const handleKeyDown = (e: React.KeyboardEvent<HTMLDivElement>): void => {
    // Navigate pickers with arrow keys
    if (showAtPicker && filteredAtFiles.length > 0) {
      if (e.key === "ArrowDown") {
        e.preventDefault();
        setAtPickerIdx((i) => Math.min(i + 1, filteredAtFiles.length - 1));
        return;
      }
      if (e.key === "ArrowUp") {
        e.preventDefault();
        setAtPickerIdx((i) => Math.max(i - 1, 0));
        return;
      }
      if (e.key === "Enter" || e.key === "Tab") {
        e.preventDefault();
        insertAtFile(filteredAtFiles[atPickerIdx]);
        return;
      }
      if (e.key === "Escape") {
        setShowAtPicker(false);
        return;
      }
    }
    if (showSlashPicker && filteredSlashItems.length > 0) {
      if (e.key === "ArrowDown") {
        e.preventDefault();
        setSlashPickerIdx((i) => Math.min(i + 1, filteredSlashItems.length - 1));
        return;
      }
      if (e.key === "ArrowUp") {
        e.preventDefault();
        setSlashPickerIdx((i) => Math.max(i - 1, 0));
        return;
      }
      if (e.key === "Enter" || e.key === "Tab") {
        e.preventDefault();
        insertSlashItem(filteredSlashItems[slashPickerIdx]);
        return;
      }
      if (e.key === "Escape") {
        setShowSlashPicker(false);
        return;
      }
    }
    if (e.key === "Enter" && !e.shiftKey) {
      e.preventDefault();
      handleSend();
    }
  };

  const handlePaste = (e: React.ClipboardEvent<HTMLDivElement>): void => {
    const items = Array.from(e.clipboardData.items);
    const imageItem = items.find((item) => item.type.startsWith("image/"));
    if (!imageItem) return;
    e.preventDefault();
    const file = imageItem.getAsFile();
    if (!file) return;
    const url = URL.createObjectURL(file);
    const name = `image_${Date.now()}.png`;
    setPastedImages((prev) => [...prev, { name, url }]);
    // Insert @name reference at cursor
    const editor = editorRef.current;
    if (editor) {
      const textNode = document.createTextNode("@" + name + " ");
      const selection = window.getSelection();
      if (selection && selection.rangeCount > 0) {
        const range = selection.getRangeAt(0);
        range.deleteContents();
        range.insertNode(textNode);
        range.setStartAfter(textNode);
        range.collapse(true);
        selection.removeAllRanges();
        selection.addRange(range);
      } else {
        editor.appendChild(textNode);
      }
      setInputText(editor.textContent ?? '');
    }
  };

  const handleFolderPick = async (): Promise<void> => {
    const folder = await window.api.dialog.openFolder();
    if (folder) {
      // Set working directory for current session
      const session = sessions.find((s) => s.id === currentSessionId);
      if (session && !session.acpSessionId) {
        updateSession(session.id, { workingDirectory: folder });
      } else if (!session) {
        setWorkingDirectory(folder);
      }
      addRecentFolder(folder);
      setShowFolderPicker(false);
    }
  };

  const handleFolderSelect = (folder: string): void => {
    // Set working directory for current session - only for sessions that haven't started yet
    const session = sessions.find((s) => s.id === currentSessionId);
    if (session && !session.acpSessionId) {
      updateSession(session.id, { workingDirectory: folder });
    } else if (!session) {
      setWorkingDirectory(folder);
    }
    addRecentFolder(folder);
    setShowFolderPicker(false);
  };

  const handleModeSelect = async (mode: string): Promise<void> => {
    setCurrentMode(mode);
    setShowModePicker(false);
    if (acpStatus === "connected") {
      const acpSessId = currentSession()?.acpSessionId;
      await window.api.acp.setMode(mode, acpSessId).catch(console.error);
    }
  };

  const handleModelChange = async (model: string): Promise<void> => {
    setCurrentModel(model);
    if (acpStatus === "connected") {
      const acpSessId = currentSession()?.acpSessionId;
      await window.api.acp.setModel(model, acpSessId).catch(console.error);
    }
  };

  const isSending = acpStatus === "connecting" || isStarting;
  const isCurrentSessionStreaming =
    currentSessionId != null && streamingSessions.includes(currentSessionId);
  const isCurrentSessionBusy =
    currentSessionId != null && busySessions.includes(currentSessionId);
  const canSend =
    (inputText.trim().length > 0 || pastedImages.length > 0) && !isSending && !isCurrentSessionBusy;
  const currentModeName =
    availableModes.find((m) => m.id === currentMode)?.name ??
    currentMode ??
    "Default";

  const handleStop = useCallback(() => {
    const acpSessId = currentSession()?.acpSessionId;
    if (acpSessId) {
      window.api.acp.cancelSession(acpSessId).catch(console.error);
    }
  }, [currentSession]);

  return (
    <div className="input-card-wrap">
      <div className="input-card">
        {/* Pasted image previews */}
        {pastedImages.length > 0 && (
          <div className="input-attachments">
            {pastedImages.map((img, i) => (
              <div key={i} className="attachment-chip">
                <img
                  src={img.url}
                  alt={img.name}
                  className="attachment-thumb"
                />
                <span className="attachment-name">{img.name}</span>
                <button
                  className="attachment-remove"
                  onClick={() => {
                    URL.revokeObjectURL(img.url);
                    setPastedImages((prev) => prev.filter((_, j) => j !== i));
                    setInputText((prev) =>
                      prev
                        .replace("@" + img.name + " ", "")
                        .replace("@" + img.name, ""),
                    );
                  }}
                >
                  ✕
                </button>
              </div>
            ))}
          </div>
        )}

        {/* Contenteditable input */}
        <div className="input-card-textarea-wrap" ref={atPickerRef}>
          <div
            ref={editorRef}
            className="input-card-editor"
            contentEditable
            onInput={handleEditorInput}
            onKeyDown={handleKeyDown}
            onPaste={handlePaste}
            data-placeholder={tr.typeMessage}
            suppressContentEditableWarning
          />

          {/* @ file picker dropdown */}
          {showAtPicker && filteredAtFiles.length > 0 && (
            <div className="input-picker-dropdown">
              {filteredAtFiles.slice(0, 12).map((f, i) => (
                <button
                  key={f.path}
                  className={`input-picker-item ${i === atPickerIdx ? "active" : ""}`}
                  onMouseDown={(e) => {
                    e.preventDefault();
                    insertAtFile(f);
                  }}
                >
                  <span className="picker-item-icon">
                    {f.isDir ? "📁" : "📄"}
                  </span>
                  {f.path}
                </button>
              ))}
            </div>
          )}

          {/* / slash picker dropdown (modes + skills + prompts) */}
          {showSlashPicker && filteredSlashItems.length > 0 && (
            <div className="input-picker-dropdown">
              {filteredSlashItems.map((item, i) => (
                <button
                  key={item.id}
                  className={`input-picker-item ${i === slashPickerIdx ? "active" : ""}`}
                  onMouseDown={(e) => {
                    e.preventDefault();
                    insertSlashItem(item);
                  }}
                >
                  <span className="picker-item-icon">/</span>
                  <span>{item.label}</span>
                  <span className={`picker-item-kind picker-item-kind-${item.kind}`}>{item.kind}</span>
                </button>
              ))}
            </div>
          )}
        </div>

        {/* Bottom toolbar */}
        <div className="input-card-toolbar">
          {/* Left actions */}
          <div className="input-card-left" ref={folderPickerRef}>
            <button
              className={`ic-btn ${currentSession()?.acpSessionId ? 'ic-btn-disabled' : ''}`}
              onClick={() => {
                if (!currentSession()?.acpSessionId) {
                  setShowFolderPicker(!showFolderPicker)
                }
              }}
              title={currentSession()?.acpSessionId ? tr.folderChangeDisabled : tr.workingDirectory}
              disabled={!!currentSession()?.acpSessionId}
            >
              <svg
                width="15"
                height="15"
                viewBox="0 0 24 24"
                fill="none"
                stroke="currentColor"
                strokeWidth="2"
                strokeLinecap="round"
                strokeLinejoin="round"
              >
                <path d="M22 19a2 2 0 0 1-2 2H4a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h5l2 3h9a2 2 0 0 1 2 2z" />
              </svg>
            </button>
            {showFolderPicker && (
              <div className="folder-picker-dropdown">
                <button
                  className="folder-picker-item"
                  onClick={handleFolderPick}
                >
                  <span>📁 {tr.addNew}</span>
                </button>
                {storeConfig.recentFolders.map((folder, idx) => (
                  <button
                    key={folder}
                    className="folder-picker-item"
                    onClick={() => handleFolderSelect(folder)}
                  >
                    <span>📁 {folder.split(/[/\\]/).pop()}</span>
                  </button>
                ))}
              </div>
            )}
            <button
              className={`ic-btn ${sandboxEnabled ? "ic-btn-active" : ""}`}
              onClick={() => setSandboxEnabled(!sandboxEnabled)}
              title={sandboxEnabled ? tr.sandboxOn : tr.sandboxOff}
            >
              <svg
                width="15"
                height="15"
                viewBox="0 0 24 24"
                fill="none"
                stroke="currentColor"
                strokeWidth="2"
                strokeLinecap="round"
                strokeLinejoin="round"
              >
                <rect x="3" y="11" width="18" height="11" rx="2" ry="2" />
                <path d="M7 11V7a5 5 0 0 1 10 0v4" />
              </svg>
            </button>
          </div>

          {/* Right actions */}
          <div className="input-card-right">
            {/* Model select */}
            {availableModels.length > 0 && (
              <select
                className="ic-model-select"
                value={currentModel}
                onChange={(e) => handleModelChange(e.target.value)}
                title={tr.aiModel}
              >
                {availableModels.map((m) => (
                  <option key={m} value={m}>
                    {m}
                  </option>
                ))}
              </select>
            )}

            {/* Mode picker */}
            <div className="ic-mode-wrap" ref={modePickerRef}>
              <button
                className="ic-mode-btn"
                onClick={() => setShowModePicker(!showModePicker)}
                title={tr.agentMode}
              >
                {currentModeName}
                <svg
                  width="10"
                  height="10"
                  viewBox="0 0 10 10"
                  fill="currentColor"
                >
                  <path d="M1 3l4 4 4-4" />
                </svg>
              </button>
              {showModePicker && availableModes.length > 0 && (
                <div className="ic-mode-dropdown">
                  {availableModes.map((m) => (
                    <button
                      key={m.id}
                      className={`ic-mode-item ${m.id === currentMode ? "active" : ""}`}
                      onClick={() => handleModeSelect(m.id)}
                    >
                      {m.id === currentMode && <span>✓ </span>}
                      {m.name}
                    </button>
                  ))}
                </div>
              )}
            </div>

            {/* Stop / Send button */}
            {isCurrentSessionBusy ? (
              <button
                className="ic-stop-btn"
                onClick={handleStop}
                title="Stop"
              >
                <StopIcon />
              </button>
            ) : (
              <button
                className={`ic-send-btn ${canSend ? "enabled" : ""}`}
                onClick={handleSend}
                disabled={!canSend}
                title={tr.sendEnter}
              >
                {isSending ? <span className="spinner">⟳</span> : <SendIcon />}
              </button>
            )}
          </div>
        </div>
      </div>

      {/* Status bar below card */}
      <div className="input-status-bar">
        <div className={`acp-status ${acpStatus}`}>
          <span className="status-dot" />
          <span className="status-label">
            {acpStatus === "connected"
              ? tr.connected
              : acpStatus === "connecting"
                ? tr.connecting
                : acpStatus === "error"
                  ? tr.error
                  : tr.disconnected}
          </span>
        </div>
        {currentSessionWorkingDirectory() && (
          <span className="status-cwd" title={currentSessionWorkingDirectory()}>
            {currentSessionWorkingDirectory().split(/[/\\]/).pop()}
          </span>
        )}
      </div>
    </div>
  );
}
