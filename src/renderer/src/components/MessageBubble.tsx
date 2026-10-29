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

// v0.10.2: some tool inputs arrive with fields as structured blocks (arrays
// of {type:"text", text} etc.) instead of plain strings - coerce them so
// string operations (.match/.includes) cannot crash the renderer.
function asStr(v: unknown): string {
  if (typeof v === "string") return v;
  if (Array.isArray(v)) {
    return v
      .map((b) => {
        if (typeof b === "string") return b;
        if (b && typeof b === "object") {
          const o = b as Record<string, unknown>;
          if (typeof o.text === "string") return o.text;
        }
        return "";
      })
      .join("");
  }
  if (v && typeof v === "object") {
    const o = v as Record<string, unknown>;
    if (typeof o.text === "string") return o.text;
    try {
      return JSON.stringify(v);
    } catch {
      return "";
    }
  }
  return "";
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
    lower === "shell" ||
    lower.endsWith(".bash") ||
    lower.endsWith(".execute")
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
        onClick={() => openFilePreview(resolveSessionPath(path, acpSessionId), undefined, remoteIdForSession(acpSessionId))}
      >
        ↗
      </button>
    </div>
  );
}

// v0.12: split text into lines, dropping a single trailing empty line
// (files normally end with a newline, so the last split element is empty).
function cutLines(text: string): string[] {
  const lines = text.split("\n");
  if (lines.length > 0 && lines[lines.length - 1] === "") lines.pop();
  return lines;
}

type DiffRow = { kind: "ctx" | "del" | "add"; oldNo?: number; newNo?: number; text: string };

// v0.12: build git-style diff rows (context/del/add) for one edit pair.
function buildDiffRows(
  d: { oldText: string | null; newText: string; startLine?: number | null; newStartLine?: number | null },
  previousContent?: string | null,
): DiffRow[] {
  const oldLines = d.oldText != null ? cutLines(d.oldText) : [];
  const newLines = cutLines(d.newText);
  const sl = typeof d.startLine === "number" && d.startLine > 0 ? d.startLine : null;
  const nsl = typeof d.newStartLine === "number" && d.newStartLine > 0 ? d.newStartLine : sl;
  const rows: DiffRow[] = [];
  const prevLines = sl != null && previousContent ? cutLines(previousContent) : null;
  const oldStart = sl ?? 1;
  const newStart = nsl ?? oldStart;
  if (prevLines) {
    // context before the change (git shows 2 surrounding lines)
    for (let i = oldStart - 3; i < oldStart - 1; i++) {
      if (i >= 0 && i < prevLines.length) {
        rows.push({ kind: "ctx", oldNo: i + 1, newNo: i + 1, text: prevLines[i] });
      }
    }
  }
  for (let i = 0; i < oldLines.length; i++) {
    rows.push({ kind: "del", oldNo: oldStart + i, text: oldLines[i] });
  }
  for (let i = 0; i < newLines.length; i++) {
    rows.push({ kind: "add", newNo: newStart + i, text: newLines[i] });
  }
  if (prevLines) {
    // context after the change; new-file numbers shift by the line delta
    const delta = newLines.length - oldLines.length;
    const afterFrom = oldStart - 1 + oldLines.length;
    for (let i = afterFrom; i < afterFrom + 2 && i < prevLines.length; i++) {
      rows.push({ kind: "ctx", oldNo: i + 1, newNo: i + 1 + delta, text: prevLines[i] });
    }
  }
  return rows;
}

// Renders structured edit diffs (ACP diff content blocks, e.g. from the
// edit tool) as a git-style hunk: line numbers, context lines, +/- coloring.
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
      {diffs.map((d, i) => {
        const rows = buildDiffRows(d, toolCall.previousContent).slice(0, 400);
        const sl = typeof d.startLine === "number" && d.startLine > 0 ? d.startLine : null;
        const nsl = typeof d.newStartLine === "number" && d.newStartLine > 0 ? d.newStartLine : sl;
        const oldCount = rows.filter((r) => r.kind !== "add").length;
        const newCount = rows.filter((r) => r.kind !== "del").length;
        return (
          <div key={i} className="tool-diff-pair">
            <ToolFilePathLink path={d.path} acpSessionId={acpSessionId} tr={tr} />
            {sl != null && (
              <div className="tool-diff-hunk">
                {"@@ -" + sl + "," + oldCount + " +" + (nsl ?? sl) + "," + newCount + " @@"}
              </div>
            )}
            <div className="tool-diff-table">
              {rows.map((r, j) => (
                <div key={j} className={"tool-diff-row tool-diff-" + r.kind}>
                  <span className="tool-diff-lno">{r.oldNo != null ? r.oldNo : ""}</span>
                  <span className="tool-diff-lno">{r.newNo != null ? r.newNo : ""}</span>
                  <span className="tool-diff-sign">
                    {r.kind === "del" ? "-" : r.kind === "add" ? "+" : " "}
                  </span>
                  <span className="tool-diff-text">{r.text}</span>
                </div>
              ))}
            </div>
          </div>
        );
      })}
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

// v0.12: resolve the raw structured output of a process tool. Live wire
// rawOutput is flat; session previews from disk nest the payload under
// "structured_content". Older sessions keep a JSON dump in the output text.
function processRawFrom(toolCall: ToolCall): Record<string, unknown> {
  const raw = toolCall.rawOutput;
  if (raw && typeof raw === "object") {
    const r = raw as Record<string, unknown>;
    const sc = r["structured_content"] ?? r["structuredContent"];
    if (sc && typeof sc === "object") return sc as Record<string, unknown>;
    return r;
  }
  if (toolCall.output) {
    try {
      const parsed = JSON.parse(toolCall.output);
      if (parsed && typeof parsed === "object" && !Array.isArray(parsed)) {
        return parsed as Record<string, unknown>;
      }
    } catch {
      /* not JSON */
    }
  }
  return {};
}

// v0.12: process.start/list/stop card - command, process table, output text.
function ProcessBlock({ toolCall }: { toolCall: ToolCall }): React.JSX.Element {
  const inp = toolCall.input ?? {};
  const ro = processRawFrom(toolCall);
  const cmd = asStr(inp.command ?? inp.cmd) || String(ro["processId"] ?? ro["process_id"] ?? "");
  const processes = Array.isArray(ro["processes"])
    ? (ro["processes"] as Array<Record<string, unknown>>)
    : null;
  const outText =
    asStr(ro["output"]) ||
    (toolCall.output ? (parseToolOutput(toolCall.output) ?? toolCall.output) : "");
  const statusStr = ro["status"] != null ? String(ro["status"]) : "";
  const exitTop = ro["exitCode"];
  return (
    <div className="proc-card">
      {cmd.length > 0 && <div className="proc-cmd">{cmd}</div>}
      {processes ? (
        <div className="proc-table">
          {processes.map((p, i) => {
            const status = String(p["status"] ?? "");
            const exitCode = p["exitCode"] ?? p["exit_code"];
            return (
              <div key={i} className="proc-tr">
                <span className="proc-tr-cmd">
                  {String(p["command"] ?? p["cmd"] ?? p["processId"] ?? "")}
                </span>
                <span className={"proc-status" + (status === "running" ? " running" : "")}>
                  {status + (typeof exitCode === "number" ? " · exit " + exitCode : "")}
                </span>
              </div>
            );
          })}
        </div>
      ) : (
        <div className="proc-card-meta">
          {statusStr.length > 0 && (
            <span className={"proc-status" + (statusStr === "running" ? " running" : "")}>
              {statusStr + (typeof exitTop === "number" ? " · exit " + exitTop : "")}
            </span>
          )}
          {outText.length > 0 && <pre className="proc-out">{outText}</pre>}
        </div>
      )}
    </div>
  );
}

// v0.12: muted one-liner for discovery-style tools (search_tool_functions).
function ToolLineBlock({ toolCall }: { toolCall: ToolCall }): React.JSX.Element {
  const out = toolCall.output ? (parseToolOutput(toolCall.output) ?? toolCall.output) : undefined;
  const text = out && out.length > 0 ? out : toolCall.title;
  return (
    <div className="tool-discovery-line">
      <span className="tool-discovery-dot">·</span>
      <span>{text}</span>
    </div>
  );
}

// v0.13: transcript of a subagent session read from the unified session store.
interface SubagentWorkData {
  messages: Array<{
    role: string;
    content: string;
    thinking?: Array<{ id: string; content: string }>;
    toolCalls?: Array<{ title: string; status: string }>;
  }>;
}

/**
 * v1.0.37: a ticking clock for running work. Tool calls and subagents can run
 * for many minutes (a build, a test suite, an agent turn) - a static "..." gave
 * no sense of progress, and a finished look was plainly wrong. The timer keeps
 * running until the status says otherwise; there is no client-side timeout.
 */
function useTicker(active: boolean): number {
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    if (!active) return;
    const iv = window.setInterval(() => setNow(Date.now()), 1000);
    return () => window.clearInterval(iv);
  }, [active]);
  return now;
}

function formatDuration(ms: number): string {
  if (!Number.isFinite(ms) || ms < 0) return "";
  const total = Math.floor(ms / 1000);
  const h = Math.floor(total / 3600);
  const m = Math.floor((total % 3600) / 60);
  const sec = total % 60;
  if (h > 0) return h + "h " + m + "m";
  if (m > 0) return m + "m " + String(sec).padStart(2, "0") + "s";
  return sec + "s";
}

const SPINNER_ICON = (
  <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2.5" strokeLinecap="round" className="tool-spin">
    <path d="M21 12a9 9 0 1 1-6.219-8.56" />
  </svg>
);

/**
 * v1.0.37: `agents.list` result card. The tool returns
 * `{agents:[{name,status,pending_turn,error?,config_name?}]}` in rawOutput,
 * which the main process used to drop - so the card rendered the bare title
 * "Listed subagents" and the session looked like it had no agents at all.
 */
function AgentsListBlock({
  toolCall,
  tr,
}: {
  toolCall: ToolCall;
  tr: Record<TranslationKeys, string>;
}): React.JSX.Element {
  const raw = (toolCall.rawOutput ?? {}) as Record<string, unknown>;
  const list = Array.isArray(raw.agents)
    ? (raw.agents as Array<Record<string, unknown>>)
    : Array.isArray(raw.subagents)
      ? (raw.subagents as Array<Record<string, unknown>>)
      : [];
  return (
    <div className="agents-list-card">
      <div className="agents-list-header">
        <span className="agents-list-title">{toolCall.title}</span>
        <span className={"subagent-status" + (toolCall.status === "running" ? " running" : "")}>
          {toolCall.status === "running" ? SPINNER_ICON : toolCall.status === "error" ? "✕" : "✓"}
        </span>
      </div>
      {list.length === 0 ? (
        <div className="agents-list-empty">{tr.subagentNoActive}</div>
      ) : (
        <table className="agents-list-table">
          <thead>
            <tr>
              <th>{tr.subagent}</th>
              <th>{tr.toolRunning}</th>
            </tr>
          </thead>
          <tbody>
            {list.map((a, i) => {
              const status = String(a.status ?? (a.pending_turn ? "running" : "idle"));
              const failed = status === "failed" || status === "error";
              return (
                <tr key={i}>
                  <td className="agents-list-name">{String(a.name ?? a.agent ?? "?")}</td>
                  <td className={failed ? "agents-list-status error" : "agents-list-status"}>
                    {status === "running" || a.pending_turn ? SPINNER_ICON : failed ? "✕" : "✓"}
                    <span>{status}</span>
                    {a.error ? <span className="agents-list-error">{String(a.error)}</span> : null}
                  </td>
                </tr>
              );
            })}
          </tbody>
        </table>
      )}
    </div>
  );
}

// v1.0.37: subagent invocation card (agents.spawn / task tool).
// It follows the LIVE run from the main-process tracker: animated spinner,
// elapsed time, per-step status and the child's transcript as it grows. The
// old card trusted the parent tool call's status, which the app itself closed
// on PromptComplete - so a busy agent looked finished and "show work" showed
// nothing (it read the child file once, only after that wrong "done").
function SubagentBlock({
  toolCall,
  tr,
  acpSessionId,
}: {
  toolCall: ToolCall;
  tr: Record<TranslationKeys, string>;
  acpSessionId?: string;
}): React.JSX.Element {
  const run = useStore((st) => st.subagentRuns[toolCall.id]);
  const sa = toolCall.subagent;
  const agent =
    run?.agent ??
    (typeof sa?.agent === "string" && sa.agent.length > 0
      ? sa.agent
      : typeof sa?.agentName === "string" && sa.agentName.length > 0
        ? sa.agentName
        : tr.subagent);
  const task = run?.task ?? (typeof sa?.task === "string" ? sa.task : "");
  const childId = run?.childSessionId ?? (typeof sa?.childSessionId === "string" ? sa.childSessionId : null);
  const status = run?.status ?? toolCall.status;
  const running = status === "running";
  const failed = status === "error";
  const now = useTicker(running);
  const elapsed = run?.elapsedMs ?? (toolCall.startedAt ? now - toolCall.startedAt : 0);
  const [promptOpen, setPromptOpen] = useState(false);
  // open by default while the agent works - the whole point is to see progress
  const [open, setOpen] = useState(running);
  const [work, setWork] = useState<SubagentWorkData | null>(null);
  const [workErr, setWorkErr] = useState(false);
  const [workLoading, setWorkLoading] = useState(false);

  useEffect(() => {
    if (running) setOpen(true);
  }, [running]);

  /** Disk fallback for a finished child whose run state was lost (app restart). */
  const loadWork = async (): Promise<void> => {
    if (workLoading || !childId) return;
    setWorkLoading(true);
    setWorkErr(false);
    try {
      const statsApi = window.api.stats as unknown as {
        sessionPreview: (sessionId: string) => Promise<unknown>;
        remoteSessionPreview: (remoteId: string, sessionId: string) => Promise<unknown>;
      };
      const rid = remoteIdForSession(acpSessionId);
      const res = rid
        ? await statsApi.remoteSessionPreview(rid, childId)
        : await statsApi.sessionPreview(childId);
      if (res && typeof res === "object" && Array.isArray((res as SubagentWorkData).messages)) {
        setWork(res as SubagentWorkData);
      } else {
        setWorkErr(true);
      }
    } catch {
      setWorkErr(true);
    }
    setWorkLoading(false);
  };

  const messages = run?.messages ?? work?.messages ?? [];
  const steps = run?.steps ?? [];
  const toggleWork = (): void => {
    const next = !open;
    setOpen(next);
    if (next && messages.length === 0 && !running) void loadWork();
  };

  return (
    <div className={"subagent-card" + (running ? " running" : "")}>
      <div className="subagent-header">
        <span className="subagent-badge">{tr.subagent}</span>
        <span className="subagent-agent">{agent}</span>
        {run?.truncated ? <span className="subagent-truncated">{tr.subagentShowingLastN}</span> : null}
        <span className={"subagent-status" + (running ? " running" : failed ? " error" : " done")}>
          {running ? SPINNER_ICON : failed ? "✕" : "✓"}
        </span>
        {elapsed > 0 ? (
          <span className="subagent-elapsed" title={running ? tr.toolRunning : undefined}>
            {running ? formatDuration(elapsed) : t(tr, "subagentFinishedIn", { duration: formatDuration(elapsed) })}
          </span>
        ) : null}
      </div>
      {task.length > 0 && (
        <div
          className={"subagent-task" + (promptOpen ? " open" : "")}
          onClick={() => setPromptOpen((v) => !v)}
        >
          <span className="subagent-task-label">{tr.subagentPromptLabel}</span>
          <span className="subagent-task-text">{task}</span>
        </div>
      )}
      {failed && toolCall.output && toolCall.output.length > 0 && (
        <pre className="subagent-error-out">{toolCall.output}</pre>
      )}
      {steps.length > 0 && (
        <ul className="subagent-steps">
          {steps.map((st, i) => {
            const stepRunning = st.status === "in_progress" || st.status === "running";
            const stepError = st.status === "error" || st.status === "failed";
            return (
              <li key={i} className={"subagent-step" + (stepRunning ? " running" : stepError ? " error" : "")}>
                <span className="subagent-step-icon">
                  {stepRunning ? SPINNER_ICON : stepError ? "✕" : "✓"}
                </span>
                <span className="subagent-step-title">{st.title}</span>
              </li>
            );
          })}
        </ul>
      )}
      {childId && (
        <button className="subagent-work-btn" onClick={toggleWork} disabled={workLoading}>
          {open ? tr.subagentHideWork : tr.subagentShowWork}
        </button>
      )}
      {workErr && <div className="subagent-work-error">{tr.subagentWorkUnavailable}</div>}
      {open && messages.length > 0 && (
        <div className="subagent-work">
          {messages.map((m, i) => (
            <div key={i} className="subagent-work-msg">
              {m.role === "user" ? (
                <div className="subagent-work-user">{m.content}</div>
              ) : (
                <>
                  {(m.thinking ?? []).map((th, ti) => (
                    <div key={"th" + ti} className="subagent-work-thinking">
                      {th.content}
                    </div>
                  ))}
                  {(m.toolCalls ?? []).map((tc, tci) => {
                    const tcRunning = tc.status === "in_progress" || tc.status === "running";
                    return (
                      <div
                        key={"tc" + tci}
                        className={"subagent-work-tool" + (tcRunning ? " running" : tc.status === "error" ? " error" : "")}
                      >
                        {tcRunning ? SPINNER_ICON : tc.status === "error" ? "✕" : "·"} {tc.title}
                      </div>
                    );
                  })}
                  {m.content.length > 0 && (
                    <div className="subagent-work-answer">
                      <MarkdownContent content={m.content} />
                    </div>
                  )}
                </>
              )}
            </div>
          ))}
        </div>
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
  // v0.12: route by the canonical tool name from the ACP _meta when present -
  // more precise than the title/input heuristics below.
  const tn = (toolCall.toolName ?? "").toLowerCase();
  if (tn === "bash" || tn === "file_system.bash") {
    return <BashBlock toolCall={toolCall} acpSessionId={acpSessionId} />;
  }
  if (
    tn === "read_file" ||
    tn === "file_system.read_file" ||
    tn.endsWith(".read_file")
  ) {
    return <ReadFileBlock toolCall={toolCall} tr={tr} />;
  }
  if (toolCall.subagent || tn === "agents.spawn" || tn === "task") {
    return <SubagentBlock toolCall={toolCall} tr={tr} acpSessionId={acpSessionId} />;
  }
  // v1.0.37: agents.list must render its result (the card used to show the bare
  // title, which read as "no subagents at all").
  if (tn === "agents.list" || tn === "agents.list_configs" || /listing subagents|listed subagents/i.test(toolCall.title)) {
    return <AgentsListBlock toolCall={toolCall} tr={tr} />;
  }
  if (tn.startsWith("process.")) {
    return <ProcessBlock toolCall={toolCall} />;
  }
  if (tn === "search_tool_functions" || tn === "vibe.todo") {
    return <ToolLineBlock toolCall={toolCall} />;
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
    let oldStr = asStr(
      inp.oldString ??
        inp.old_string ??
        inp.old_str ??
        inp.oldText ??
        inp.search ??
        inp.search_text ??
        inp.target ??
        inp.old_content,
    );
    let newStr = asStr(
      inp.newString ??
        inp.new_string ??
        inp.new_str ??
        inp.newText ??
        inp.replace ??
        inp.replace_text ??
        inp.new_content,
    );

    // inp.content may hold the full SEARCH/REPLACE block (vibe-acp format)
    if (!oldStr && !newStr) {
      const text = asStr(inp.text ?? inp.content);
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
        const o = asStr(ch.oldString ?? ch.old_string);
        const n = asStr(ch.newString ?? ch.new_string);
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

// ===== Work timeline (steps shown above the assistant answer) =====

const COPY_ICON = (
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
);

const STEP_CHECK_ICON = (
  <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2">
    <circle cx="12" cy="12" r="10" />
    <path d="M7.5 12L10.5 15L16.5 9" />
  </svg>
);

const STEP_SPINNER_ICON = (
  <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2.5" strokeLinecap="round">
    <path d="M21 12a9 9 0 1 1-6.219-8.56" />
  </svg>
);

const STEP_ERROR_ICON = (
  <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2">
    <circle cx="12" cy="12" r="10" />
    <path d="M9 9L15 15" />
    <path d="M15 9L9 15" />
  </svg>
);

const STEP_CHEVRON_ICON = (
  <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2">
    <path d="M10.5 8L14.5 12L10.5 16" />
  </svg>
);

interface TimelineStep {
  key: string;
  kind: "thinking" | "tool";
  order: number;
  thinking?: ThinkingBlock;
  toolCall?: ToolCall;
}

function buildTimelineSteps(message: Message): TimelineStep[] {
  const tools = message.toolCalls ?? [];
  const thinkingSteps: TimelineStep[] = (message.thinking ?? []).map((b) => ({
    key: "t-" + b.id,
    kind: "thinking",
    order: b.startedAt ?? 0,
    thinking: b,
  }));
  const toolSteps: TimelineStep[] = tools.map((tc, i) => ({
    key: "c-" + tc.id,
    kind: "tool",
    order: tc.startedAt ?? Number.MAX_SAFE_INTEGER - (tools.length - i),
    toolCall: tc,
  }));
  return [...thinkingSteps, ...toolSteps].sort((a, b) => a.order - b.order);
}

function thinkingDurationSeconds(block: ThinkingBlock): number {
  const end = block.completedAt ?? Date.now();
  return Math.max(0, Math.round((end - (block.startedAt ?? end)) / 1000));
}

// Short label for a step, used both in timeline rows and the summary line.
function stepLabel(step: TimelineStep, tr: Record<TranslationKeys, string>): string {
  if (step.kind === "thinking") {
    const block = step.thinking as ThinkingBlock;
    if (!block.isComplete) return tr.thinkingThinking;
    const seconds = thinkingDurationSeconds(block);
    if (seconds >= 1) return t(tr, "thoughtForLabel", { seconds });
    return tr.thinking;
  }
  const tc = step.toolCall as ToolCall;
  const translated = translateToolTitle(tc.title, tr);
  if (translated !== tc.title) return translated;
  return rawFunctionNameToTitleFromInput(tc.title, tc.input ?? {}, tr);
}

function StepRow({
  step,
  tr,
  acpSessionId,
  last,
}: {
  step: TimelineStep;
  tr: Record<TranslationKeys, string>;
  acpSessionId?: string;
  last: boolean;
}): React.JSX.Element {
  const [open, setOpen] = useState(false);
  const running =
    step.kind === "tool"
      ? (step.toolCall as ToolCall).status === "running"
      : !(step.thinking as ThinkingBlock).isComplete;
  const isError =
    step.kind === "tool" && (step.toolCall as ToolCall).status === "error";
  const statusClass = running
    ? "step-running"
    : isError
      ? "step-error"
      : "step-done";
  const icon = running
    ? STEP_SPINNER_ICON
    : isError
      ? STEP_ERROR_ICON
      : STEP_CHECK_ICON;
  // v1.0.37: a long command (10, 15, 30+ minutes) shows a spinner and a live
  // counter for as long as the tool is running. There is no client-side cap.
  const now = useTicker(running && step.kind === "tool");
  const startedAt = step.kind === "tool" ? (step.toolCall as ToolCall).startedAt : undefined;
  const elapsed = running && startedAt ? now - startedAt : 0;

  return (
    <div className={"step-row " + statusClass + (open ? " open" : "")}>
      <div className="step-rail">
        <span className="step-icon">{icon}</span>
        {!last && <span className="step-connector" />}
      </div>
      <div className="step-body">
        <button className="step-header" onClick={() => setOpen((v) => !v)}>
          <span className={"step-label" + (running ? " steps-shimmer" : "")}>
            {stepLabel(step, tr)}
          </span>
          {elapsed > 0 && (
            <span className="step-elapsed" title={tr.toolRunning}>
              {formatDuration(elapsed)}
            </span>
          )}
          <span className="step-chevron">{STEP_CHEVRON_ICON}</span>
        </button>
        {open && (
          <div className="step-content">
            {step.kind === "thinking" ? (
              <div className="step-reasoning">
                {(step.thinking as ThinkingBlock).content
                  .split(/\n\n+/)
                  .map((p) => p.trim())
                  .filter((p) => p.length > 0)
                  .map((p, i) => (
                    <p key={i}>{p}</p>
                  ))}
              </div>
            ) : (
              (() => {
                const tc = step.toolCall as ToolCall;
                // v0.13: subagent invocations (agents.spawn / task) get the rich card
                if (tc.subagent || tc.toolName === 'agents.spawn') {
                  return <SubagentBlock toolCall={tc} tr={tr} acpSessionId={acpSessionId} />;
                }
                // v1.0.37: agents.list has its own card (agent table)
                if (
                  tc.toolName === 'agents.list' ||
                  tc.toolName === 'agents.list_configs' ||
                  /listing subagents|listed subagents/i.test(tc.title)
                ) {
                  return <AgentsListBlock toolCall={tc} tr={tr} />;
                }
                const isTaskTool =
                  tc.title === "task" ||
                  tc.title.startsWith("task:") ||
                  (tc.input != null && "agent" in tc.input && "task" in tc.input);
                if (isTaskTool) {
                  return <TaskToolBlock toolCall={tc} acpSessionId={acpSessionId} />;
                }
                const richContent = ToolCallRichContent({ toolCall: tc, tr, acpSessionId });
                if (richContent !== null) return richContent;
                return (
                  <>
                    {extractToolCallPath(tc) && (
                      <ToolCallPathChip
                        path={extractToolCallPath(tc) as string}
                        acpSessionId={acpSessionId}
                      />
                    )}
                    {tc.input && Object.keys(tc.input).length > 0 && (
                      <pre className="tool-call-input">
                        {JSON.stringify(tc.input, null, 2)}
                      </pre>
                    )}
                    {tc.output && (
                      <pre className="tool-call-output">
                        {parseToolOutput(tc.output) ?? tc.output}
                      </pre>
                    )}
                    {/* v1.0.37: never a bare sentence with no body - say why the
                        card is empty instead of looking like a broken session */}
                    {!tc.output && tc.status !== 'running' && (
                      <div className="tool-no-output">{tr.toolNoOutput}</div>
                    )}
                    {tc.status === 'running' && !tc.output && (
                      <div className="tool-no-output running">
                        <span className="tool-spin-wrap">{SPINNER_ICON}</span>
                        <span>{tr.toolRunning}</span>
                      </div>
                    )}
                  </>
                );
              })()
            )}
          </div>
        )}
      </div>
    </div>
  );
}

// Collapsible "harness steps" section rendered above the assistant answer,
// mirroring the official Mistral Vibe chat layout: one muted summary line
// that expands into a vertical timeline of thinking + tool steps. While
// the agent is working the section stays open with an animated spinner
// and shimmering label on the running step ("work in progress").
function StepsTimeline({
  message,
  isAgentBusy,
  acpSessionId,
}: {
  message: Message;
  isAgentBusy?: boolean;
  acpSessionId?: string;
}): React.JSX.Element | null {
  const { config } = useStore();
  const tr = useTranslation(config.language);
  const [manual, setManual] = useState<boolean | null>(null);
  const steps = useMemo(
    () => buildTimelineSteps(message),
    [message.thinking, message.toolCalls],
  );
  if (steps.length === 0) return null;

  const anyRunning = steps.some(
    (s) =>
      (s.kind === "tool" && (s.toolCall as ToolCall).status === "running") ||
      (s.kind === "thinking" && !(s.thinking as ThinkingBlock).isComplete),
  );
  const active = anyRunning || !!isAgentBusy;
  const open = manual !== null ? manual : active;
  const summary = steps
    .slice(0, 4)
    .map((s) => stepLabel(s, tr))
    .join(" · ");

  return (
    <div
      className={
        "steps-section" + (open ? " open" : "") + (active ? " steps-active" : "")
      }
    >
      <button
        className="steps-summary"
        onClick={() => setManual(!open)}
      >
        <span
          className={"steps-summary-text" + (active ? " steps-shimmer" : "")}
        >
          {summary}
        </span>
        <span className="steps-chevron">{STEP_CHEVRON_ICON}</span>
      </button>
      {open && (
        <div className="steps-list">
          {steps.map((s, i) => (
            <StepRow
              key={s.key}
              step={s}
              tr={tr}
              acpSessionId={acpSessionId}
              last={i === steps.length - 1}
            />
          ))}
        </div>
      )}
    </div>
  );
}
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
  // Agent prose wraps paths in quotes/backticks and trailing punctuation
  path = path.replace(/^["'`]+/, "").replace(/["`]+$/, "").replace(/[.,;:!?)]+$/, "");
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
      onClick={() => openFilePreview(resolveSessionPath(path, acpSessionId), undefined, remoteIdForSession(acpSessionId))}
    >
      {path}
    </button>
  );
}

// Resolves the remote connection id owning the given ACP session (null = local).
function remoteIdForSession(acpSessionId?: string): string | null {
  if (!acpSessionId) return null;
  return useStore.getState().getSessionByAcpId(acpSessionId)?.remoteId ?? null;
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
      const sess = st.sessions.find((s) => s.id === st.currentSessionId);
      const open = (): void => st.openFilePreview(resolveSessionPath(text), undefined, sess?.remoteId ?? null);
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

function MessageBubble({
  message,
  isStreaming,
  isAgentBusy,
  acpSessionId,
}: Props): React.JSX.Element {
  const { config } = useStore();
  const tr = useTranslation(config.language);
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

  // CLI checkpoints ("Model changed to …", "Compacting context", …) render
  // as subtle notice rows, never as chat bubbles — localized when possible.
  const noticeText = message.content.trim();
  const isCliNotice = (text: string): boolean =>
    text.startsWith("Model changed to ") ||
    text.startsWith("Agent changed to ") ||
    text === "Compacting context" ||
    text === "Context compacted" ||
    text === "Compaction failed" ||
    text === "Compacting conversation history..." ||
    text === "Compacted conversation history" ||
    text === "Session title updated";
  if (!hasThinking && !hasToolCalls && isCliNotice(noticeText)) {
    const translateCliNotice = (text: string): string => {
      const configOptions = useStore.getState().configOptions;
      if (text.startsWith("Model changed to ")) {
        const tech = text.slice("Model changed to ".length).trim();
        const modelOpt = configOptions.find((o) => o.id === "model");
        const displayName =
          modelOpt?.options.find((op) => op.value === tech)?.name ?? tech;
        return tr.modelChangedTo.replace("{{model}}", displayName);
      }
      if (text.startsWith("Agent changed to ")) {
        const agent = text.slice("Agent changed to ".length).trim();
        return tr.agentChangedTo.replace("{{agent}}", agent);
      }
      if (text === "Compacting context") return tr.compactingContext;
      if (text === "Context compacted") return tr.contextCompacted;
      if (text === "Compaction failed") return tr.compactionFailed;
      if (text === "Compacting conversation history...") return tr.compactingHistory;
      if (text === "Compacted conversation history") return tr.compactedHistory;
      if (text === "Session title updated") return tr.sessionTitleUpdated;
      return text;
    };
    return (
      <div className="msg-row msg-notice">{translateCliNotice(noticeText)}</div>
    );
  }

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
        <div className="msg-user-col">
          <div className="msg-bubble">
            <div className="msg-text">
              <UserMessageContent
                content={message.content}
                attachments={message.attachments}
              />
            </div>
          </div>
          <div className="msg-actions-row">
            <div className="msg-actions-left" />
            <div className="msg-actions-right">
              <button
                className="msg-action-btn"
                onClick={handleCopy}
                title={tr.copyLabel}
              >
                {COPY_ICON}
              </button>
              <div className="msg-time">{formatTime(message.timestamp)}</div>
            </div>
          </div>
        </div>
      </div>
    );
  }

  return (
    <div className="msg-row msg-ai">
      <div className="msg-content">
        <StepsTimeline
          message={message}
          isAgentBusy={isAgentBusy}
          acpSessionId={acpSessionId}
        />
        {message.content && (
          <div className="msg-text">
            <MarkdownContent
              content={message.content}
              isStreaming={isStreaming && !message.isComplete}
            />
          </div>
        )}
        <div className="msg-actions-row">
          <div className="msg-actions-left" />
          <div className="msg-actions-right">
            <button
              className="msg-action-btn"
              onClick={handleCopy}
              title={tr.copyLabel}
            >
              {COPY_ICON}
            </button>
            <div className="msg-time">{formatTime(message.timestamp)}</div>
          </div>
        </div>
      </div>
    </div>
  );
}

export default React.memo(MessageBubble)
