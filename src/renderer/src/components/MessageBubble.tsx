import React, { useEffect, useRef, useMemo, useState } from "react";
import ReactMarkdown from "react-markdown";
import remarkGfm from "remark-gfm";
import mermaid from "mermaid";
import { useStore } from "../store";
import { useTranslation, t } from "../i18n";
import type { TranslationKeys } from "../i18n";
import type { Message, ToolCall, ThinkingBlock } from "../types";

interface Props {
  message: Message;
  isStreaming?: boolean;
  isAgentBusy?: boolean;
  acpSessionId?: string;
}

let mermaidInitialized = false;

async function initMermaid(): Promise<void> {
  if (mermaidInitialized) return;
  mermaid.initialize({
    startOnLoad: false,
    theme: "neutral",
    fontFamily: "var(--font-sans)",
  });
  mermaidInitialized = true;
}

function MermaidDiagram({ code }: { code: string }): React.JSX.Element {
  const [svg, setSvg] = useState<string>("");
  const [error, setError] = useState<string>("");
  const [isLoading, setIsLoading] = useState(true);
  const containerRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    let cancelled = false;

    const render = async () => {
      try {
        await initMermaid();
        if (cancelled) return;

        const id = `mermaid-${Math.random().toString(36).slice(2)}`;
        const trimmedCode = code.trim();
        const { svg: renderedSvg } = await mermaid.render(id, trimmedCode);
        if (!cancelled) {
          setSvg(renderedSvg);
          setError("");
        }
      } catch (err) {
        if (!cancelled) {
          setError(err instanceof Error ? err.message : String(err));
          setSvg("");
        }
      } finally {
        if (!cancelled) {
          setIsLoading(false);
        }
      }
    };

    render();

    return () => {
      cancelled = true;
    };
  }, [code]);

  if (error) {
    return (
      <div className="mermaid-error">
        <div className="mermaid-error-title">Mermaid Error:</div>
        <pre>{error}</pre>
      </div>
    );
  }

  if (isLoading || !svg) {
    return (
      <div className="mermaid-loading">
        <span>Rendering diagram...</span>
      </div>
    );
  }

  return (
    <div
      className="mermaid-container"
      ref={containerRef}
      dangerouslySetInnerHTML={{ __html: svg }}
    />
  );
}

// Parses ACP JSON content-event array format into plain text.
// [{type:"content", content:{type:"text",text:"..."}}, ...]
function parseToolOutput(raw: string): string | null {
  try {
    const parsed = JSON.parse(raw);
    if (Array.isArray(parsed)) {
      const texts = parsed
        .filter(
          (e: Record<string, unknown>) =>
            (e.type === "content" || e.type === "tool_error") &&
            typeof (e.content as Record<string, unknown>)?.text === "string",
        )
        .map(
          (e: Record<string, unknown>) =>
            (e.content as Record<string, unknown>).text as string,
        );
      if (texts.length > 0) return texts.join("\n");
    }
    if (typeof parsed === "object" && parsed !== null) {
      const p = parsed as Record<string, unknown>;
      const val = p.text ?? p.result ?? p.output;
      if (typeof val === "string") return val;
    }
  } catch {
    /* not JSON */
  }
  return raw.trim() || null;
}

const TOOL_ACTION_KEYS: Array<[string, TranslationKeys]> = [
  ["reading", "toolReading"],
  ["patching", "toolPatching"],
  ["running", "toolRunning"],
  ["searching", "toolSearching"],
  ["listing", "toolListing"],
  ["writing", "toolWriting"],
  ["creating", "toolCreating"],
  ["deleting", "toolDeleting"],
];

const TOOL_ACTION_KEYS_EN: Array<[string, string]> = [
  ["reading", "Reading"],
  ["patching", "Patching"],
  ["running", "Running"],
  ["searching", "Searching"],
  ["listing", "Listing"],
  ["writing", "Writing"],
  ["creating", "Creating"],
  ["deleting", "Deleting"],
];

function translateToolTitleEnglish(title: string): string {
  const lower = title.toLowerCase();
  for (const [word, label] of TOOL_ACTION_KEYS_EN) {
    if (lower.startsWith(word)) {
      const suffix = title.slice(word.length).replace(/\s*\([^)]*\)\s*$/, "");
      return label + suffix;
    }
  }
  const m = title.match(/^Applied\s+(\d+)\s+block/i);
  if (m) return `Applied ${m[1]} block`;
  if (/^Applied\b/i.test(title)) return "Applied";
  return title;
}

// Maps raw ACP function name (e.g. "read_file") + parsed input to a display title matching active session format.
function rawFunctionNameToTitleFromInput(
  name: string,
  inp: Record<string, unknown>,
  tr: Record<TranslationKeys, string>,
): string {
  const getFilename = (): string => {
    const p = (inp.path ?? inp.file_path ?? inp.filePath ?? inp.filename ?? "") as string;
    if (!p) return "";
    return " " + p.split(/[/\\]/).pop();
  };

  const lower = name.toLowerCase();
  if (lower === "read_file" || lower === "view" || lower === "read") {
    return tr.toolReading + getFilename();
  }
  if (
    lower === "search_replace" ||
    lower === "str_replace" ||
    lower === "edit_file" ||
    lower === "edit" ||
    lower.endsWith(".edit")
  ) {
    return tr.toolPatching + getFilename();
  }
  if (
    lower === "bash" ||
    lower === "execute" ||
    lower === "run_command" ||
    lower === "shell"
  ) {
    const cmd = ((inp.command ?? inp.cmd ?? inp.script ?? "") as string).slice(
      0,
      50,
    );
    return tr.toolRunning + (cmd ? " " + cmd : "");
  }
  if (lower === "grep" || lower === "find_in_file" || lower === "ripgrep") {
    const pattern = (
      (inp.pattern ?? inp.query ?? inp.regex ?? "") as string
    ).slice(0, 50);
    return tr.toolSearching + (pattern ? " " + pattern : "");
  }
  if (lower === "glob" || lower === "list_dir" || lower === "ls") {
    const pattern = ((inp.pattern ?? "") as string).slice(0, 50);
    return tr.toolListing + (pattern ? " " + pattern : getFilename());
  }
  if (lower === "write_file" || lower === "create_file") {
    return tr.toolCreating + getFilename();
  }
  if (lower === "delete_file") {
    return tr.toolDeleting + getFilename();
  }
  return translateToolTitle(name, tr);
}

function rawFunctionNameToTitle(
  name: string,
  args: string,
  tr: Record<TranslationKeys, string>,
): string {
  let inp: Record<string, unknown> = {};
  try {
    inp = JSON.parse(args);
  } catch {
    /* ignore */
  }
  return rawFunctionNameToTitleFromInput(name, inp, tr);
}

function translateToolTitle(
  title: string,
  tr: Record<TranslationKeys, string>,
): string {
  const lower = title.toLowerCase();
  for (const [word, key] of TOOL_ACTION_KEYS) {
    if (lower.startsWith(word)) {
      const suffix = title.slice(word.length).replace(/\s*\([^)]*\)\s*$/, "");
      return tr[key] + suffix;
    }
  }
  // Handle "bash: <command>" format from vibe-acp active session
  const bashColon = title.match(/^bash:\s*(.+)$/i);
  if (bashColon)
    return tr.toolRunning + " " + bashColon[1].replace(/\s*\([^)]*\)\s*$/, "");
  const m = title.match(/^Applied\s+(\d+)\s+block/i);
  if (m) return t(tr, "toolApplied", { count: m[1] });
  if (/^Applied\b/i.test(title)) return tr.toolAppliedOk;
  return title;
}

function ReadFileBlock({
  toolCall,
  tr,
}: {
  toolCall: ToolCall;
  tr: Record<TranslationKeys, string>;
}): React.JSX.Element {
  const inp = toolCall.input ?? {};
  const inputPath = (inp.path ?? inp.file_path ?? inp.filePath ?? "") as string;
  const startLine = (inp.start_line ?? inp.startLine) as number | undefined;
  const endLine = (inp.end_line ?? inp.endLine) as number | undefined;
  const lineRange =
    startLine != null
      ? `:${startLine}${endLine != null ? `-${endLine}` : ""}`
      : "";
  const rawOut = toolCall.output
    ? (parseToolOutput(toolCall.output) ?? toolCall.output)
    : undefined;

  // Detect structured format (vibe-acp sends path:/content:/lines_read: keys)
  const hasStructuredContent =
    rawOut != null &&
    (rawOut.includes("\ncontent:") || rawOut.startsWith("content:"));

  const [ipcContent, setIpcContent] = useState<string | null>(null);
  const [ipcLoading, setIpcLoading] = useState(false);

  useEffect(() => {
    if (hasStructuredContent || !inputPath) return;
    setIpcLoading(true);
    const fsApi = window.api.fs as {
      readFile: (p: string) => Promise<{ type: string; content: string }>;
    };
    fsApi
      .readFile(inputPath)
      .then((result) => {
        if (result.type === "file") {
          if (startLine != null) {
            const lines = result.content.split("\n");
            setIpcContent(
              lines
                .slice(startLine - 1, endLine != null ? endLine : undefined)
                .join("\n"),
            );
          } else {
            setIpcContent(result.content);
          }
        }
      })
      .catch(() => {
        /* ignore */
      })
      .finally(() => setIpcLoading(false));
  }, [inputPath, hasStructuredContent]);

  const translateMetaLine = (line: string): string => {
    if (line.startsWith("lines_read:")) {
      return tr.linesRead + ": " + line.slice("lines_read:".length).trim();
    }
    if (line.startsWith("was_truncated:")) {
      const val = line.slice("was_truncated:".length).trim().toLowerCase();
      return tr.truncated + ": " + (val === "true" ? tr.yes : tr.no);
    }
    return line;
  };

  if (hasStructuredContent && rawOut) {
    const lines = rawOut.split("\n");
    const pathLines: string[] = [];
    const metaLines: string[] = [];
    const contentLines: string[] = [];
    let inContent = false;

    for (let i = 0; i < lines.length; i++) {
      const line = lines[i];
      const nextLine = lines[i + 1];

      if (line.startsWith("lines_read:") || line.startsWith("was_truncated:")) {
        metaLines.push(line);
      } else if (line.startsWith("path:")) {
        pathLines.push(line);
      } else if (line.startsWith("content:")) {
        inContent = true;
        const contentPart = line.slice(8).trimStart();
        if (contentPart) contentLines.push(contentPart);
      } else if (inContent) {
        contentLines.push(line);
      } else if (
        !inContent &&
        pathLines.length === 0 &&
        nextLine &&
        nextLine.startsWith("lines_read:")
      ) {
        pathLines.push(`path: ${line}`);
      } else if (!inContent) {
        pathLines.push(line);
      } else {
        contentLines.push(line);
      }
    }

    return (
      <div className="tool-rich-read">
        {pathLines.map((line, i) => {
          if (line.startsWith("path:")) {
            return (
              <div key={i} className="tool-file-path">
                {line.slice(5).trim()}
                {lineRange}
              </div>
            );
          }
          return (
            <div key={i} className="tool-read-meta">
              {line}
            </div>
          );
        })}
        {metaLines.map((line, i) => (
          <div key={i} className="tool-read-meta">
            {translateMetaLine(line)}
          </div>
        ))}
        {contentLines.length > 0 && (
          <pre className="tool-file-content">{contentLines.join("\n")}</pre>
        )}
      </div>
    );
  }

  // Non-structured output (e.g. "Read 10 lines from AGENTS.md") — extract lines_read if possible
  const linesReadFromSummary = rawOut
    ? rawOut.match(/Read\s+(\d+)\s+lines?\s+from/i)
    : null;
  const linesReadMeta = linesReadFromSummary
    ? tr.linesRead + ": " + linesReadFromSummary[1]
    : null;

  return (
    <div className="tool-rich-read">
      {inputPath && (
        <div className="tool-file-path">
          {inputPath}
          {lineRange}
        </div>
      )}
      {linesReadMeta && <div className="tool-read-meta">{linesReadMeta}</div>}
      {ipcLoading && <div className="tool-read-meta">…</div>}
      {ipcContent && <pre className="tool-file-content">{ipcContent}</pre>}
    </div>
  );
}

function BashBlock({
  toolCall,
  acpSessionId,
}: {
  toolCall: ToolCall;
  acpSessionId?: string;
}): React.JSX.Element {
  const inp = toolCall.input ?? {};
  const titleCmd = toolCall.title.match(/^bash:\s*(.+)$/i)?.[1] ?? "";
  const command = (
    (inp.command ?? inp.cmd ?? inp.script ?? inp.code ?? titleCmd) as string
  ).trim();
  const rawOut = toolCall.output
    ? (parseToolOutput(toolCall.output) ?? toolCall.output)
    : undefined;

  // Single-line "Ran X" summary — no useful content
  const isSummaryOnly = rawOut
    ? /^Ran\s+\S/i.test(rawOut.trim()) && !rawOut.includes("\n")
    : !rawOut;

  const [ipcOut, setIpcOut] = useState<string | null>(null);
  const [ipcLoading, setIpcLoading] = useState(false);

  useEffect(() => {
    // Wait until tool is done (rawOut set) before loading from IPC
    if (!isSummaryOnly || !acpSessionId || !rawOut) return;
    setIpcLoading(true);
    const statsApi = window.api.stats as {
      vibeSessionMessages: (
        id: string,
      ) => Promise<
        Array<{ role: string; tool_call_id?: string; content: string | null }>
      >;
    };
    statsApi
      .vibeSessionMessages(acpSessionId)
      .then((messages) => {
        const result = messages.find(
          (m) => m.role === "tool" && m.tool_call_id === toolCall.id,
        );
        if (result?.content) setIpcOut(result.content);
      })
      .catch(() => {
        /* ignore */
      })
      .finally(() => setIpcLoading(false));
    // rawOut in deps: re-runs when output arrives (undefined → "Ran dir")
  }, [acpSessionId, toolCall.id, rawOut]);

  const displayOut = ipcOut ?? (isSummaryOnly ? null : (rawOut ?? null));

  // Parse structured "command:/stdout:/stderr:/returncode:" format
  function parseBashOutput(raw: string): {
    stdout: string | null;
    stderr: string | null;
    returncode: number | null;
  } {
    const sections: Record<string, string> = {};
    let currentKey: string | null = null;
    for (const line of raw.split("\n")) {
      const header = line.match(/^(command|stdout|stderr|returncode): (.*)/);
      if (header) {
        currentKey = header[1];
        sections[currentKey] = header[2];
      } else if (currentKey) {
        sections[currentKey] += "\n" + line;
      }
    }
    return {
      stdout: sections.stdout !== undefined ? sections.stdout : null,
      stderr: sections.stderr !== undefined ? sections.stderr.trimEnd() : null,
      returncode:
        sections.returncode !== undefined
          ? parseInt(sections.returncode.trim(), 10)
          : null,
    };
  }

  let stdoutText: string | null = null;
  let stderrText: string | null = null;
  let showStderr = false;

  if (displayOut) {
    if (displayOut.includes("stdout:") || displayOut.includes("returncode:")) {
      const parsed = parseBashOutput(displayOut);
      stdoutText = parsed.stdout?.trimEnd() || null;
      const rc = parsed.returncode;
      if (parsed.stderr && parsed.stderr.length > 0 && rc !== 0) {
        stderrText = parsed.stderr;
        showStderr = true;
      }
    } else {
      stdoutText = displayOut;
    }
  }

  return (
    <div className="tool-rich-bash">
      {command && (
        <div className="tool-bash-command">
          <span className="tool-bash-prompt">$</span>
          <span className="tool-bash-cmd">{command}</span>
        </div>
      )}
      {ipcLoading && <div className="tool-read-meta">…</div>}
      {stdoutText && <pre className="tool-bash-output">{stdoutText}</pre>}
      {showStderr && stderrText && (
        <pre className="tool-bash-output tool-bash-stderr">{stderrText}</pre>
      )}
    </div>
  );
}

// Clickable path row with a quick preview button for tool call file paths.
function ToolFilePathLink({
  path,
  acpSessionId,
  tr,
}: {
  path: string;
  acpSessionId?: string;
  tr: Record<TranslationKeys, string>;
}): React.JSX.Element {
  const openFilePreview = useStore((s) => s.openFilePreview);
  return (
    <div className="tool-file-path-row">
      <span className="tool-file-path">{path}</span>
      <button
        className="tool-path-preview-btn"
        title={tr.openPreview}
        onClick={() => openFilePreview(resolveSessionPath(path, acpSessionId))}
      >
        ↗
      </button>
    </div>
  );
}

// Renders structured edit diffs (ACP diff content blocks, e.g. from the
// new edit tool) with a clickable path and +/- coloring.
function EditDiffBlock({
  toolCall,
  tr,
  acpSessionId,
}: {
  toolCall: ToolCall;
  tr: Record<TranslationKeys, string>;
  acpSessionId?: string;
}): React.JSX.Element {
  const diffs = toolCall.diffs ?? [];
  const outStr = toolCall.output ?? "";
  const isError = outStr.includes("<tool_error>");
  return (
    <div className={isError ? "tool-rich-error" : "tool-rich-diff"}>
      {diffs.map((d, i) => (
        <div key={i} className="tool-diff-pair">
          <ToolFilePathLink path={d.path} acpSessionId={acpSessionId} tr={tr} />
          {d.oldText != null && d.oldText.length > 0 && (
            <pre className="tool-diff-removed">
              {d.oldText.split("\n").map((l) => `- ${l}`).join("\n")}
            </pre>
          )}
          {d.newText.length > 0 && (
            <pre className="tool-diff-added">
              {d.newText.split("\n").map((l) => `+ ${l}`).join("\n")}
            </pre>
          )}
        </div>
      ))}
      {isError ? (
        <pre className="tool-error-output">
          {outStr.replace(/<\/?tool_error>/g, "").trim()}
        </pre>
      ) : (
        <span className="tool-success-badge">✓ {tr.toolAppliedOk}</span>
      )}
    </div>
  );
}

function ToolCallRichContent({
  toolCall,
  tr,
  acpSessionId,
}: {
  toolCall: ToolCall;
  tr: Record<TranslationKeys, string>;
  acpSessionId?: string;
}): React.JSX.Element | null {
  const name = toolCall.title.toLowerCase();
  const inp = toolCall.input ?? {};
  if (toolCall.diffs && toolCall.diffs.length > 0) {
    return <EditDiffBlock toolCall={toolCall} tr={tr} acpSessionId={acpSessionId} />;
  }
  const out = toolCall.output
    ? (parseToolOutput(toolCall.output) ?? toolCall.output)
    : undefined;

  // Input-shape-based detection (fallback when title doesn't match)
  const hasBashInput =
    typeof inp.command === "string" ||
    typeof inp.cmd === "string" ||
    typeof inp.script === "string";
  const hasReadFileInput =
    (typeof inp.path === "string" ||
      typeof inp.file_path === "string" ||
      typeof inp.filePath === "string") &&
    (inp.offset !== undefined ||
      inp.limit !== undefined ||
      inp.start_line !== undefined);
  const hasSearchReplaceInput =
    inp.old_string !== undefined ||
    inp.oldString !== undefined ||
    inp.old_str !== undefined ||
    (typeof inp.content === "string" && inp.content.includes("<<<<<<<")) ||
    (typeof inp.file_path === "string" &&
      inp.file_path.length > 0 &&
      typeof inp.content === "string" &&
      inp.content.length > 0);
  const hasGrepInput =
    (inp.pattern !== undefined ||
      inp.regex !== undefined ||
      inp.query !== undefined) &&
    (inp.path !== undefined || inp.directory !== undefined);

  // bash / execute / run_command / shell / terminal
  if (
    name.includes("bash") ||
    name.includes("execute") ||
    name.includes("run_command") ||
    name.includes("shell") ||
    name.includes("terminal") ||
    name.includes("command") ||
    name.includes("cmd") ||
    hasBashInput
  ) {
    return <BashBlock toolCall={toolCall} acpSessionId={acpSessionId} />;
  }

  // read_file / view / cat
  if (
    name === "read_file" ||
    name === "view" ||
    name === "read" ||
    name.includes("read_file") ||
    hasReadFileInput
  ) {
    return <ReadFileBlock toolCall={toolCall} tr={tr} />;
  }

  // search_replace / str_replace / edit_file / create_file
  if (
    name === "edit" ||
    name.endsWith(".edit") ||
    name.includes("replace") ||
    name.includes("str_replace") ||
    name.includes("edit_file") ||
    name.includes("create_file") ||
    name.includes("write") ||
    hasSearchReplaceInput
  ) {
    const path = (inp.path ?? inp.file_path ?? inp.filePath ?? inp.filename ?? "") as string;

    // Check if output indicates an error (more robust check)
    const outStr = out ?? "";
    const isError = outStr.includes("<tool_error>");

    // Parse SEARCH/REPLACE block format - strips marker lines
    const parseDiffBlock = (block: string): string[] =>
      block
        .split("\n")
        .filter(
          (l) =>
            !l.startsWith("<<<<<<<") &&
            !l.startsWith("=======") &&
            !l.startsWith(">>>>>>>"),
        );

    // Extract old/new content from input
    let oldStr = (inp.oldString ??
      inp.old_string ??
      inp.old_str ??
      inp.oldText ??
      inp.search ??
      inp.search_text ??
      inp.target ??
      inp.old_content ??
      "") as string;
    let newStr = (inp.newString ??
      inp.new_string ??
      inp.new_str ??
      inp.newText ??
      inp.replace ??
      inp.replace_text ??
      inp.new_content ??
      "") as string;

    // inp.content may hold the full SEARCH/REPLACE block (vibe-acp format)
    if (!oldStr && !newStr) {
      const text = (inp.text ?? inp.content ?? "") as string;
      const m = text.match(
        /<<<<<<<[^\n]*\n([\s\S]*?)\n?=======\n?([\s\S]*?)\n?>>>>>>>[^\n]*/,
      );
      if (m) {
        oldStr = m[1];
        newStr = m[2];
      }
    }

    // inp.content with inline markers but oldStr/newStr split across one field
    if (oldStr && !newStr && oldStr.includes("<<<<<<<")) {
      const sep = oldStr.indexOf("=======");
      const end = oldStr.indexOf(">>>>>>>");
      if (sep > 0 && end > sep) {
        newStr = oldStr.substring(sep + 7, end);
        oldStr = oldStr.substring(0, sep);
      }
    }
    if (!oldStr && newStr && newStr.includes("<<<<<<<")) {
      const sep = newStr.indexOf("=======");
      const end = newStr.indexOf(">>>>>>>");
      if (sep > 0 && end > sep) {
        oldStr = newStr.substring(0, sep);
        newStr = newStr.substring(sep + 7, end);
      }
    }

    const oldLines = oldStr ? parseDiffBlock(oldStr) : [];
    const newLines = newStr ? parseDiffBlock(newStr) : [];
    // New CLI edit tool: batch of change objects
    const batchPairs: Array<{ oldStr: string; newStr: string; replaceAll?: boolean }> = [];
    if (Array.isArray(inp.changes)) {
      for (const ch of inp.changes as Array<Record<string, unknown>>) {
        if (!ch) continue;
        const o = (ch.oldString ?? ch.old_string ?? "") as string;
        const n = (ch.newString ?? ch.new_string ?? "") as string;
        batchPairs.push({
          oldStr: o,
          newStr: n,
          replaceAll: ch.replaceAll === true || ch.replace_all === true,
        });
      }
    }
    const hasDiff =
      oldLines.length > 0 || newLines.length > 0 || batchPairs.length > 0;

    // Status badge (shown alongside the diff, not instead of it)
    const statusBadge = (() => {
      if (isError) return null; // error shown separately below
      if (outStr.includes("Applied")) {
        const m = outStr.match(/Applied\s+(\d+)\s+block/i);
        const label = m
          ? t(tr, "toolApplied", { count: m[1] })
          : tr.toolAppliedOk;
        return (
          <span className="tool-success-badge">
            <svg
              xmlns="http://www.w3.org/2000/svg"
              width="12"
              height="12"
              viewBox="0 0 24 24"
              fill="none"
              stroke="currentColor"
              strokeWidth="2.5"
              strokeLinecap="round"
              strokeLinejoin="round"
            >
              <polyline points="20 6 9 17 4 12" />
            </svg>
            {label}
          </span>
        );
      }
      return null;
    })();

    if (isError) {
      const errorMsg = outStr.replace(/<\/?tool_error>/g, "").trim();
      return (
        <div className="tool-rich-error">
          {path && (
            <ToolFilePathLink path={path} acpSessionId={acpSessionId} tr={tr} />
          )}
          {oldLines.length > 0 && (
            <pre className="tool-diff-removed">
              {oldLines.map((l) => `- ${l}`).join("\n")}
            </pre>
          )}
          {newLines.length > 0 && (
            <pre className="tool-diff-added">
              {newLines.map((l) => `+ ${l}`).join("\n")}
            </pre>
          )}
          {batchPairs.map((p, i) => (
            <div key={i} className="tool-diff-pair">
              {p.oldStr && (
                <pre className="tool-diff-removed">
                  {p.oldStr.split("\n").map((l) => `- ${l}`).join("\n")}
                </pre>
              )}
              <pre className="tool-diff-added">
                {p.newStr.split("\n").map((l) => `+ ${l}`).join("\n")}
              </pre>
            </div>
          ))}
          <pre className="tool-error-output">{errorMsg}</pre>
        </div>
      );
    }

    if (hasDiff) {
      return (
        <div className="tool-rich-diff">
          {path && (
            <ToolFilePathLink path={path} acpSessionId={acpSessionId} tr={tr} />
          )}
          {statusBadge}
          {oldLines.length > 0 && (
            <pre className="tool-diff-removed">
              {oldLines.map((l) => `- ${l}`).join("\n")}
            </pre>
          )}
          {newLines.length > 0 && (
            <pre className="tool-diff-added">
              {newLines.map((l) => `+ ${l}`).join("\n")}
            </pre>
          )}
          {batchPairs.map((p, i) => (
            <div key={i} className="tool-diff-pair">
              {p.replaceAll && <div className="tool-read-meta">replace_all</div>}
              {p.oldStr && (
                <pre className="tool-diff-removed">
                  {p.oldStr.split("\n").map((l) => `- ${l}`).join("\n")}
                </pre>
              )}
              <pre className="tool-diff-added">
                {p.newStr.split("\n").map((l) => `+ ${l}`).join("\n")}
              </pre>
            </div>
          ))}
        </div>
      );
    }

    // No diff info — show output or success badge only
    if (out) {
      return (
        <div className="tool-rich-diff">
          {path && (
            <ToolFilePathLink path={path} acpSessionId={acpSessionId} tr={tr} />
          )}
          {statusBadge ?? <pre className="tool-bash-output">{out}</pre>}
        </div>
      );
    }

    return (
      <div className="tool-rich-result">
      {path && (
        <ToolFilePathLink path={path} acpSessionId={acpSessionId} tr={tr} />
      )}
        {statusBadge}
      </div>
    );
  }

  // grep / search / find_in_file / ripgrep
  if (
    name.includes("grep") ||
    name.includes("search") ||
    name === "find_in_file" ||
    name === "ripgrep" ||
    hasGrepInput
  ) {
    const pattern = (inp.pattern ?? inp.query ?? inp.regex ?? "") as string;
    const path = (inp.path ?? inp.directory ?? inp.dir ?? inp.filePath ?? "") as string;
    return (
      <div className="tool-rich-grep">
        <div className="tool-grep-header">
          {pattern && <code className="tool-grep-pattern">{pattern}</code>}
          {path && <span className="tool-grep-path">{path}</span>}
        </div>
        {out && <pre className="tool-grep-output">{out}</pre>}
      </div>
    );
  }

  // glob / list_dir / ls
  if (name.includes("glob") || name.includes("list") || name === "ls") {
    const path = (inp.path ?? inp.directory ?? inp.dir ?? inp.filePath ?? "") as string;
    const pattern = (inp.pattern ?? "") as string;
    return (
      <div className="tool-rich-grep">
        <div className="tool-grep-header">
          {pattern && <code className="tool-grep-pattern">{pattern}</code>}
          {path && <span className="tool-grep-path">{path}</span>}
        </div>
        {out && <pre className="tool-grep-output">{out}</pre>}
      </div>
    );
  }

  return null; // fall through to JSON fallback
}

interface SubagentMessage {
  role: "user" | "assistant" | "tool" | "system";
  content: string | null;
  injected: boolean;
  message_id: string;
  tool_calls?: Array<{
    id: string;
    function: { name: string; arguments: string };
  }>;
  name?: string;
  tool_call_id?: string;
}

interface SubagentSession {
  sessionId: string;
  agentName: string;
  title: string | null;
  startTime: string;
  messages: SubagentMessage[];
}

function SubagentToolCall({
  name,
  args,
  tr,
}: {
  name: string;
  args: string;
  tr: Record<TranslationKeys, string>;
}): React.JSX.Element {
  const translatedName = rawFunctionNameToTitle(name, args, tr);

  return (
    <div className="subagent-tool-call">
      <svg
        xmlns="http://www.w3.org/2000/svg"
        width="11"
        height="11"
        viewBox="0 0 24 24"
        fill="none"
        stroke="currentColor"
        strokeWidth="2.5"
        strokeLinecap="round"
        strokeLinejoin="round"
      >
        <polyline points="20 6 9 17 4 12" />
      </svg>
      <code className="subagent-tool-name">{translatedName}</code>
    </div>
  );
}

function SubagentSessionBlock({
  session,
  tr,
}: {
  session: SubagentSession;
  tr: Record<TranslationKeys, string>;
}): React.JSX.Element {
  const [open, setOpen] = useState(false);
  const visibleMessages = session.messages.filter(
    (m) => !m.injected && (m.role === "user" || m.role === "assistant"),
  );
  const userCount = visibleMessages.filter((m) => m.role === "user").length;
  const assistantCount = visibleMessages.filter(
    (m) => m.role === "assistant",
  ).length;

  return (
    <div className="subagent-session-block">
      <button
        className="subagent-session-header"
        onClick={() => setOpen((v) => !v)}
      >
        <svg
          className={`subagent-chevron${open ? " open" : ""}`}
          xmlns="http://www.w3.org/2000/svg"
          width="12"
          height="12"
          viewBox="0 0 24 24"
          fill="none"
          stroke="currentColor"
          strokeWidth="2.5"
          strokeLinecap="round"
          strokeLinejoin="round"
        >
          <polyline points="9 18 15 12 9 6" />
        </svg>
        <span className="subagent-agent-name">
          {tr.subagent}: {session.agentName}
        </span>
        <span className="subagent-meta">
          {userCount} prompts · {assistantCount} responses
        </span>
      </button>
      {open && (
        <div className="subagent-messages">
          {visibleMessages.map((m) => (
            <div
              key={m.message_id}
              className={`subagent-msg subagent-msg-${m.role}`}
            >
              <span className={`subagent-msg-role subagent-msg-role-${m.role}`}>
                {m.role === "user" ? tr.subagentPrompt : tr.subagentResponse}
              </span>
              <div className="subagent-msg-body">
                {m.content && <MarkdownContent content={m.content} />}
                {m.tool_calls &&
                  m.tool_calls.map((tc) => (
                    <SubagentToolCall
                      key={tc.id}
                      name={tc.function.name}
                      args={tc.function.arguments}
                      tr={tr}
                    />
                  ))}
              </div>
            </div>
          ))}
        </div>
      )}
    </div>
  );
}

function parseTaskOutput(output: string): string | null {
  const raw = parseToolOutput(output);
  if (!raw) return null;
  // Structured format: "response: <text>\nturns_used: N\ncompleted: True"
  const responseMatch = raw.match(
    /^response:\s*([\s\S]*?)(?:\nturns_used:|\ncompleted:|$)/i,
  );
  if (responseMatch) return responseMatch[1].trim() || null;
  return raw;
}

function TaskToolBlock({
  toolCall,
  acpSessionId,
}: {
  toolCall: ToolCall;
  acpSessionId?: string;
}): React.JSX.Element {
  const [subSessions, setSubSessions] = useState<SubagentSession[] | null>(
    null,
  );
  const [loading, setLoading] = useState(false);
  const [expanded, setExpanded] = useState(false);
  const loadedForStatus = useRef<string | null>(null);
  const { config } = useStore();
  const tr = useTranslation(config.language);

  const agentName = (toolCall.input?.agent ?? "subagent") as string;
  const task = (toolCall.input?.task ?? "") as string;

  const fetchSessions = async (): Promise<void> => {
    if (!acpSessionId) {
      setSubSessions([]);
      return;
    }
    setLoading(true);
    try {
      const statsApi = window.api.stats as unknown as {
        vibeSubagentSessions: (id: string) => Promise<SubagentSession[]>;
      };
      const sessions = await statsApi.vibeSubagentSessions(acpSessionId);
      const byAgent = sessions.filter((s) => s.agentName === agentName);
      if (byAgent.length <= 1 || !task) {
        setSubSessions(byAgent);
        return;
      }
      // Multiple sessions with same agent — narrow by task content match
      const normalize = (s: string) => s.trim().replace(/\s+/g, " ");
      const taskText = normalize(task);
      const matched = byAgent.filter((s) => {
        const firstUserMsg = s.messages.find(
          (m) => m.role === "user" && !m.injected,
        );
        if (!firstUserMsg?.content) return true;
        const msgText = normalize(firstUserMsg.content);
        return (
          msgText === taskText ||
          msgText.includes(taskText) ||
          taskText.includes(msgText)
        );
      });
      // Fall back to all agent sessions if task matching yields nothing
      setSubSessions(matched.length > 0 ? matched : byAgent);
    } catch {
      setSubSessions([]);
    } finally {
      setLoading(false);
    }
  };

  // Auto-load from jsonl when tool completes (with brief delay for file flush)
  useEffect(() => {
    if (toolCall.status === "done" && loadedForStatus.current !== "done") {
      loadedForStatus.current = "done";
      const timer = setTimeout(() => {
        fetchSessions();
      }, 600);
      return () => clearTimeout(timer);
    }
  }, [toolCall.status]);

  const load = async (): Promise<void> => {
    if (loading) return;
    if (subSessions !== null && loadedForStatus.current === "done") return;
    await fetchSessions();
  };

  const toggle = (): void => {
    const next = !expanded;
    setExpanded(next);
    if (next) load();
  };

  const StatusIcon = (): React.JSX.Element => {
    if (toolCall.status === "running") {
      return (
        <svg
          className="tool-spin"
          xmlns="http://www.w3.org/2000/svg"
          width="12"
          height="12"
          viewBox="0 0 24 24"
          fill="none"
          stroke="currentColor"
          strokeWidth="2.5"
          strokeLinecap="round"
        >
          <path d="M21 12a9 9 0 1 1-6.219-8.56" />
        </svg>
      );
    }
    if (toolCall.status === "done") {
      return (
        <svg
          xmlns="http://www.w3.org/2000/svg"
          width="12"
          height="12"
          viewBox="0 0 24 24"
          fill="none"
          stroke="currentColor"
          strokeWidth="2.5"
          strokeLinecap="round"
          strokeLinejoin="round"
        >
          <polyline points="20 6 9 17 4 12" />
        </svg>
      );
    }
    return (
      <svg
        xmlns="http://www.w3.org/2000/svg"
        width="12"
        height="12"
        viewBox="0 0 24 24"
        fill="none"
        stroke="currentColor"
        strokeWidth="2.5"
        strokeLinecap="round"
        strokeLinejoin="round"
      >
        <line x1="18" y1="6" x2="6" y2="18" />
        <line x1="6" y1="6" x2="18" y2="18" />
      </svg>
    );
  };

  const rawOutput = toolCall.output ? parseToolOutput(toolCall.output) : null;
  const outputText = rawOutput ? parseTaskOutput(rawOutput) : null;
  const turnsUsedMatch = rawOutput?.match(/turns_used:\s*(\d+)/i);
  const turnsUsed = turnsUsedMatch ? parseInt(turnsUsedMatch[1]) : null;

  return (
    <div className={`tool-call-block tool-call-${toolCall.status}`}>
      <button className="tool-call-header" onClick={toggle}>
        <span className={`tool-call-icon tool-call-icon-${toolCall.status}`}>
          <StatusIcon />
        </span>
        <span className="tool-call-title">
          <span className="task-agent-label">
            {tr.subagent}: {agentName}
          </span>
          {task && (
            <span className="task-preview">
              {task.length > 60 ? task.slice(0, 60) + "…" : task}
            </span>
          )}
        </span>
        <span className="tool-call-chevron">{expanded ? "▲" : "▼"}</span>
      </button>
      {expanded && (
        <div className="tool-call-content">
          {task && <div className="task-prompt">{task}</div>}
          {loading && (
            <div className="subagent-loading">{tr.subagentLoading}</div>
          )}
          {subSessions &&
            subSessions.length > 0 &&
            subSessions.map((s) => (
              <SubagentSessionBlock key={s.sessionId} session={s} tr={tr} />
            ))}
          {outputText && (
            <div className="task-output-text">
              {turnsUsed !== null && (
                <div className="tool-read-meta">
                  {tr.turnsUsed}: {turnsUsed}
                </div>
              )}
            </div>
          )}
        </div>
      )}
    </div>
  );
}

function ToolCallBlock({
  toolCall,
  acpSessionId,
}: {
  toolCall: ToolCall;
  acpSessionId?: string;
}): React.JSX.Element {
  const [expanded, setExpanded] = useState(false);
  const { config } = useStore();
  const tr = useTranslation(config.language);

  // Delegate task tool calls to specialized block.
  // vibe-acp may send title as "task", "task: explore", "Running explore agent: …", etc.
  // Detect reliably by checking input shape (both task + agent keys present).
  const isTaskTool =
    toolCall.title === "task" ||
    toolCall.title.startsWith("task:") ||
    (toolCall.input != null &&
      "agent" in toolCall.input &&
      "task" in toolCall.input);
  if (isTaskTool) {
    return <TaskToolBlock toolCall={toolCall} acpSessionId={acpSessionId} />;
  }

  // Status icon
  const StatusIcon = (): React.JSX.Element => {
    if (toolCall.status === "running") {
      return (
        <svg
          className="tool-spin"
          xmlns="http://www.w3.org/2000/svg"
          width="12"
          height="12"
          viewBox="0 0 24 24"
          fill="none"
          stroke="currentColor"
          strokeWidth="2.5"
          strokeLinecap="round"
        >
          <path d="M21 12a9 9 0 1 1-6.219-8.56" />
        </svg>
      );
    }
    if (toolCall.status === "done") {
      return (
        <svg
          xmlns="http://www.w3.org/2000/svg"
          width="12"
          height="12"
          viewBox="0 0 24 24"
          fill="none"
          stroke="currentColor"
          strokeWidth="2.5"
          strokeLinecap="round"
          strokeLinejoin="round"
        >
          <polyline points="20 6 9 17 4 12" />
        </svg>
      );
    }
    return (
      <svg
        xmlns="http://www.w3.org/2000/svg"
        width="12"
        height="12"
        viewBox="0 0 24 24"
        fill="none"
        stroke="currentColor"
        strokeWidth="2.5"
        strokeLinecap="round"
        strokeLinejoin="round"
      >
        <line x1="18" y1="6" x2="6" y2="18" />
        <line x1="6" y1="6" x2="18" y2="18" />
      </svg>
    );
  };

  return (
    <div className={`tool-call-block tool-call-${toolCall.status}`}>
      <button
        className="tool-call-header"
        onClick={() => setExpanded((v) => !v)}
      >
        <span className={`tool-call-icon tool-call-icon-${toolCall.status}`}>
          <StatusIcon />
        </span>
        <span className="tool-call-title">
          {(() => {
            const translated = translateToolTitle(toolCall.title, tr);
            if (translated !== toolCall.title) return translated;
            return rawFunctionNameToTitleFromInput(
              toolCall.title,
              toolCall.input ?? {},
              tr,
            );
          })()}
        </span>
        <span className="tool-call-chevron">{expanded ? "▲" : "▼"}</span>
      </button>
      {expanded && (
        <div className="tool-call-content">
          {(() => {
            const richContent = ToolCallRichContent({
              toolCall,
              tr,
              acpSessionId,
            });
            if (richContent !== null) {
              return richContent;
            }
            return (
              <>
                {extractToolCallPath(toolCall) && (
                  <ToolCallPathChip
                    path={extractToolCallPath(toolCall) as string}
                    acpSessionId={acpSessionId}
                  />
                )}
                {toolCall.input && Object.keys(toolCall.input).length > 0 && (
                  <pre className="tool-call-input">
                    {JSON.stringify(toolCall.input, null, 2)}
                  </pre>
                )}
                {toolCall.output && (
                  <pre className="tool-call-output">
                    {parseToolOutput(toolCall.output) ?? toolCall.output}
                  </pre>
                )}
              </>
            );
          })()}
        </div>
      )}
    </div>
  );
}

function ThinkingSection({
  blocks,
  isStreaming,
}: {
  blocks: ThinkingBlock[];
  isStreaming?: boolean;
}): React.JSX.Element {
  const { config } = useStore();
  const tr = useTranslation(config.language);
  const [open, setOpen] = useState(false);

  const paragraphs = blocks
    .flatMap((b) => b.content.split(/\n\n+/))
    .map((p) => p.trim())
    .filter((p) => p.length > 0);

  return (
    <div className="thinking-section">
      <button className="thinking-toggle" onClick={() => setOpen((v) => !v)}>
        <span className="thinking-dot" />
        <span className="thinking-label">
          {isStreaming ? tr.thinkingThinking : tr.thinking}
        </span>
        <span className="thinking-chevron">{open ? "▲" : "▼"}</span>
      </button>
      {open && (
        <div className="thinking-content">
          <ul className="thinking-paragraphs">
            {paragraphs.map((p, i) => (
              <li key={i} className="thinking-paragraph">
                {p}
              </li>
            ))}
          </ul>
        </div>
      )}
    </div>
  );
}

const MISTRAL_LOGO = (
  <svg viewBox="0 0 24 24" fill="currentColor">
    <path d="M13.3715 16.358H16.1144V13.6486H13.3712L13.3715 16.358H10.6283V13.6486H7.88568V16.358H10.6283V19.0676H2.3999V16.358H5.14279V5.52002H7.88568V8.22963H10.6286V10.939H13.3715V8.22963H16.1144V5.52002H18.8572V16.358H21.5999V19.0676H13.3715V16.358Z" />
  </svg>
);

interface MarkdownSegment {
  type: "markdown" | "mermaid";
  content: string;
}

function parseContent(content: string): MarkdownSegment[] {
  const segments: MarkdownSegment[] = [];
  const regex = /```mermaid\r?\n([\s\S]*?)```/g;
  let lastIndex = 0;
  let match;

  while ((match = regex.exec(content)) !== null) {
    if (match.index > lastIndex) {
      segments.push({
        type: "markdown",
        content: content.slice(lastIndex, match.index),
      });
    }
    segments.push({
      type: "mermaid",
      content: match[1].trim(),
    });
    lastIndex = match.index + match[0].length;
  }

  if (lastIndex < content.length) {
    segments.push({
      type: "markdown",
      content: content.slice(lastIndex),
    });
  }

  return segments;
}

// Extracts a primary file/directory path from a tool call input, if present.
function extractToolCallPath(toolCall: ToolCall): string | null {
  const inp = toolCall.input ?? {};
  const candidates = [inp.path, inp.file_path, inp.filePath, inp.filename, inp.file, inp.dir, inp.directory];
  for (const c of candidates) {
    if (typeof c === "string" && c.length > 0) return c;
  }
  return null;
}

// Resolves a possibly relative path against the owning session working directory.
function resolveSessionPath(rawPath: string, acpSessionId?: string): string {
  const BS = String.fromCharCode(92);
  let path = rawPath.startsWith("@") ? rawPath.slice(1) : rawPath;
  const st = useStore.getState();
  const session = acpSessionId
    ? st.sessions.find((s) => s.acpSessionId === acpSessionId)
    : st.sessions.find((s) => s.id === st.currentSessionId);
  const cwd = session?.workingDirectory || st.workingDirectory || "";
  const isAbs = path.startsWith("/") || path.startsWith(BS) || /^[A-Za-z]:/.test(path);
  if (!isAbs && cwd) {
    const sep = cwd.includes(BS) ? BS : "/";
    const base = cwd.endsWith("/") || cwd.endsWith(BS) ? cwd : cwd + sep;
    path = base + path;
  }
  return path;
}

// Clickable path chip shown in expanded tool calls.
function ToolCallPathChip({
  path,
  acpSessionId,
}: {
  path: string;
  acpSessionId?: string;
}): React.JSX.Element {
  const openFilePreview = useStore((s) => s.openFilePreview);
  return (
    <button
      className="tool-call-path"
      title={path}
      onClick={() => openFilePreview(resolveSessionPath(path, acpSessionId))}
    >
      {path}
    </button>
  );
}

// Heuristic: does this inline code snippet look like a file path?
function looksLikeInlinePath(text: string): boolean {
  const t = text.trim();
  if (!t || t.length > 512) return false;
  if (t.includes(" ") || t.includes(String.fromCharCode(9)) || t.includes(String.fromCharCode(10))) {
    return false;
  }
  const p = t.startsWith("@") ? t.slice(1) : t;
  if (p.includes("://")) return false;
  const BS = String.fromCharCode(92);
  const abs = p.startsWith("/") || p.startsWith(BS) || /^[A-Za-z]:/.test(p);
  const rel = (p.includes("/") || p.includes(BS)) && /^[A-Za-z0-9._-]/.test(p.charAt(0));
  return abs || rel;
}


const markdownComponents = {
  a: ({
    href,
    children,
    ...props
  }: React.AnchorHTMLAttributes<HTMLAnchorElement> & {
    children?: React.ReactNode;
  }) => (
    <a
      {...props}
      href={href}
      onClick={(e) => {
        e.preventDefault();
        if (href) window.api.shell.openPath(href).catch(console.error);
      }}
    >
      {children}
    </a>
  ),
  code: ({
    className,
    children,
    ...rest
  }: {
    className?: string;
    children?: React.ReactNode;
  } & React.HTMLAttributes<HTMLElement>) => {
    const text = Array.isArray(children) ? children.join("") : String(children ?? "");
    if (!className && looksLikeInlinePath(text)) {
      const st = useStore.getState();
      const open = (): void => st.openFilePreview(resolveSessionPath(text));
      return (
        <code
          className="markdown-path-link"
          role="button"
          tabIndex={0}
          title={text}
          onClick={open}
          onKeyDown={(e): void => {
            if (e.key === "Enter" || e.key === " ") open();
          }}
        >
          {children}
        </code>
      );
    }
    return (
      <code className={className} {...rest}>
        {children}
      </code>
    );
  },
};

function MarkdownContent({
  content,
  isStreaming,
}: {
  content: string;
  isStreaming?: boolean;
}): React.JSX.Element {
  const segments = useMemo(() => parseContent(content), [content]);

  return (
    <div className="markdown-content">
      {segments.map((segment, idx) => {
        if (segment.type === "mermaid") {
          return (
            <div key={idx} className="mermaid-wrapper">
              <MermaidDiagram code={segment.content} />
            </div>
          );
        }
        return (
          <ReactMarkdown
            key={idx}
            remarkPlugins={[remarkGfm]}
            components={markdownComponents}
          >
            {segment.content}
          </ReactMarkdown>
        );
      })}
      {isStreaming && <span className="cursor-blink">▍</span>}
    </div>
  );
}

interface AttachmentFileProps {
  path: string;
  isDir: boolean;
  content: string;
}

function AttachmentFile({
  path,
  isDir,
  content,
}: AttachmentFileProps): React.JSX.Element {
  const [expanded, setExpanded] = useState(false);

  const shortName = path.split(/[/\\]/).pop() ?? path;
  const displayPath = isDir && !path.endsWith("/") ? path + "/" : path;

  return (
    <div className="attachment-file">
      <button
        className="attachment-header"
        onClick={() => setExpanded((v) => !v)}
      >
        <span className="attachment-icon">{isDir ? "📁" : "📄"}</span>
        <span className="attachment-path">{displayPath}</span>
        <span className="attachment-chevron">{expanded ? "▲" : "▼"}</span>
      </button>
      {expanded && (
        <div className="attachment-content">
          <pre className="attachment-code">{content}</pre>
        </div>
      )}
    </div>
  );
}

function UserMessageContent({
  content,
  attachments,
}: {
  content: string;
  attachments?: { path: string; isDir: boolean; content: string }[];
}): React.JSX.Element {
  const parts: React.ReactNode[] = [];
  const atPathRegex = /@([^\s]+)/g;
  let lastIndex = 0;
  let match;

  while ((match = atPathRegex.exec(content)) !== null) {
    if (match.index > lastIndex) {
      parts.push(
        <MarkdownContent
          key={`text-${lastIndex}`}
          content={content.slice(lastIndex, match.index)}
        />,
      );
    }

    const rawPath = match[1];
    const isDir = rawPath.endsWith("/");
    const path = isDir ? rawPath.slice(0, -1) : rawPath;

    if (attachments) {
      const attachment = attachments.find(
        (a) =>
          a.path === rawPath || a.path === path || a.path + "/" === rawPath,
      );
      if (attachment) {
        parts.push(
          <AttachmentFile
            key={`att-${match.index}`}
            path={attachment.path}
            isDir={attachment.isDir}
            content={attachment.content}
          />,
        );
      } else {
        parts.push(
          <span key={`att-${match.index}`} className="attachment-marker">
            @{rawPath}
          </span>,
        );
      }
    } else {
      parts.push(
        <span key={`att-${match.index}`} className="attachment-marker">
          @{rawPath}
        </span>,
      );
    }

    lastIndex = match.index + match[0].length;
  }

  if (lastIndex < content.length) {
    parts.push(
      <MarkdownContent
        key={`text-${lastIndex}`}
        content={content.slice(lastIndex)}
      />,
    );
  }

  return <>{parts}</>;
}

export default function MessageBubble({
  message,
  isStreaming,
  isAgentBusy,
  acpSessionId,
}: Props): React.JSX.Element {
  const { config } = useStore();
  const isUser = message.sender === "user";
  const hasThinking = message.thinking && message.thinking.length > 0;
  const hasToolCalls = message.toolCalls && message.toolCalls.length > 0;

  const handleCopy = () => {
    navigator.clipboard.writeText(message.content);
  };

  const formatTime = (timestamp: number) => {
    return new Date(timestamp).toLocaleTimeString([], {
      hour: "2-digit",
      minute: "2-digit",
    });
  };

  if (message.sender === "system" && message.isContext) {
    return (
      <div className="msg-row msg-context">
        <div className="msg-context-label">
          Context from previous conversation
        </div>
        <div className="msg-context-content">
          <MarkdownContent content={message.content} />
        </div>
      </div>
    );
  }

  if (isUser) {
    return (
      <div className="msg-row msg-user">
        <div className="msg-bubble">
          <div className="msg-text">
            <UserMessageContent
              content={message.content}
              attachments={message.attachments}
            />
          </div>
          <div className="msg-actions-row">
            <div className="msg-actions-left" />
            <div className="msg-actions-right">
              <div className="msg-time">{formatTime(message.timestamp)}</div>
              <button
                className="msg-action-btn"
                onClick={handleCopy}
                title="Copy"
              >
                <svg
                  xmlns="http://www.w3.org/2000/svg"
                  width="16"
                  height="16"
                  viewBox="0 0 24 24"
                  fill="none"
                  stroke="currentColor"
                  strokeWidth="2"
                  strokeLinecap="round"
                  strokeLinejoin="round"
                >
                  <rect width="14" height="14" x="8" y="8" rx="2" ry="2" />
                  <path d="M4 16c-1.1 0-2-.9-2-2V4c0-1.1.9-2 2-2h10c1.1 0 2 .9 2 2" />
                </svg>
              </button>
            </div>
          </div>
        </div>
      </div>
    );
  }

  return (
    <div className="msg-row msg-ai">
      <div
        className={`msg-avatar-wrap${isAgentBusy ? " msg-avatar-busy" : ""}`}
      >
        <div className="msg-avatar">{MISTRAL_LOGO}</div>
        {isAgentBusy && (
          <svg
            className="msg-avatar-spinner"
            viewBox="0 0 36 36"
            xmlns="http://www.w3.org/2000/svg"
          >
            <circle
              cx="18"
              cy="18"
              r="15"
              fill="none"
              stroke="currentColor"
              strokeWidth="2.5"
              strokeLinecap="round"
              strokeDasharray="60 40"
            />
          </svg>
        )}
      </div>
      <div className="msg-content">
        {hasThinking && (
          <ThinkingSection
            blocks={message.thinking!}
            isStreaming={isStreaming && !message.isComplete}
          />
        )}
        {hasToolCalls && (
          <div className="tool-calls-list">
            {message.toolCalls!.map((tc) => (
              <ToolCallBlock
                key={tc.id}
                toolCall={tc}
                acpSessionId={acpSessionId}
              />
            ))}
          </div>
        )}
        {message.content && (
          <div className="msg-text">
            <MarkdownContent
              content={message.content}
              isStreaming={isStreaming && !message.isComplete}
            />
          </div>
        )}
        <div className="msg-actions-row">
          <div className="msg-actions-left">
            <div className="msg-time">{formatTime(message.timestamp)}</div>
          </div>
          <div className="msg-actions-right">
            <button
              className="msg-action-btn"
              onClick={handleCopy}
              title="Copy"
            >
              <svg
                xmlns="http://www.w3.org/2000/svg"
                width="16"
                height="16"
                viewBox="0 0 24 24"
                fill="none"
                stroke="currentColor"
                strokeWidth="2"
                strokeLinecap="round"
                strokeLinejoin="round"
              >
                <rect width="14" height="14" x="8" y="8" rx="2" ry="2" />
                <path d="M4 16c-1.1 0-2-.9-2-2V4c0-1.1.9-2 2-2h10c1.1 0 2 .9 2 2" />
              </svg>
            </button>
          </div>
        </div>
      </div>
    </div>
  );
}
