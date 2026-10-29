import test from "node:test"
import assert from "node:assert/strict"
import { createRequire } from "node:module"

const require = createRequire(import.meta.url)
const registry = require("../../out/main/sessionRegistry.js")

let counter = 0
function freshSession() {
  counter += 1
  const id = "sess-" + counter
  registry.resetSession(id)
  return id
}

test("text deltas carry a growing offset and a stable message id", () => {
  const s = freshSession()
  const a = registry.appendText(s, "msg-1", "Hello")
  const b = registry.appendText(s, "msg-1", "Hello world")
  assert.equal(a.msgId, b.msgId)
  assert.equal(a.offset, 0)
  assert.equal(a.delta, "Hello")
  // the CLI sometimes resends the accumulated text: only the tail is a delta
  assert.equal(b.offset, 5)
  assert.equal(b.delta, " world")
  const snap = registry.snapshot(s)
  assert.equal(snap.messages.length, 1)
  assert.equal(snap.messages[0].content, "Hello world")
})

test("a resent short text is not duplicated (the old 40-char guard bug)", () => {
  const s = freshSession()
  registry.appendText(s, "m", "Hi")
  const second = registry.appendText(s, "m", "Hi")
  assert.equal(second.delta, "")
  registry.appendText(s, "m", "Hi there")
  const snap = registry.snapshot(s)
  assert.equal(snap.messages[0].content, "Hi there")
})

test("incremental deltas (not resends) are concatenated exactly", () => {
  const s = freshSession()
  registry.appendText(s, "m", "abc")
  registry.appendText(s, "m", "def")
  registry.appendText(s, "m", "ghi")
  assert.equal(registry.snapshot(s).messages[0].content, "abcdefghi")
})

test("reasoning after streamed text opens a new segment", () => {
  const s = freshSession()
  registry.appendText(s, "m", "answer part")
  const th = registry.appendThinking(s, "th-1", "let me think")
  const snap = registry.snapshot(s)
  assert.equal(snap.messages.length, 2)
  assert.equal(snap.messages[0].content, "answer part")
  assert.equal(snap.messages[1].id, th.msgId)
  assert.equal(snap.messages[1].thinking[0].content, "let me think")
})

test("a tool call attaches to the segment that was current, not to the newest one", () => {
  const s = freshSession()
  const first = registry.appendText(s, "m", "before tool")
  const tool = registry.startToolCall(s, {
    id: "tc-1",
    title: "bash",
    kind: "execute",
    status: "running",
  })
  assert.equal(tool.msgId, first.msgId)
  const after = registry.appendText(s, "m2", "after tool")
  assert.notEqual(after.msgId, first.msgId)
  const snap = registry.snapshot(s)
  const owner = snap.messages.find((m) => m.toolCalls.some((t) => t.id === "tc-1"))
  assert.equal(owner.id, first.msgId)
})

test("a user message closes the previous segment", () => {
  const s = freshSession()
  registry.appendText(s, "m", "first answer")
  registry.noteUserMessage(s, "u-1", "question")
  const next = registry.appendText(s, "m", "second answer")
  const snap = registry.snapshot(s)
  assert.deepEqual(
    snap.messages.map((m) => m.role),
    ["ai", "user", "ai"],
  )
  assert.notEqual(next.msgId, snap.messages[0].id)
})

test("seq grows monotonically and snapshots carry it", () => {
  const s = freshSession()
  const s1 = registry.appendText(s, "m", "a").seq
  const s2 = registry.appendText(s, "m", "ab").seq
  assert.ok(s2 > s1)
  assert.equal(registry.snapshot(s).seq, s2)
  const end = registry.endTurn(s)
  assert.ok(end > s2)
})

test("loadHistory seeds the registry (resume path uses the same ids)", () => {
  const s = freshSession()
  registry.loadHistory(
    s,
    [
      { role: "user", content: "hi" },
      { role: "ai", content: "hello", thinking: [{ id: "t1", content: "hm" }], toolCalls: [] },
    ],
    "a title",
    [{ content: "step", status: "pending" }],
  )
  const snap = registry.snapshot(s)
  assert.equal(snap.messages.length, 2)
  assert.equal(snap.messages[1].id, "ai-1")
  assert.equal(snap.title, "a title")
  assert.equal(snap.plan[0].content, "step")
  assert.equal(registry.hasSession(s), true)
})

test("updateToolCall finds the call wherever it lives", () => {
  const s = freshSession()
  registry.appendText(s, "m", "x")
  registry.startToolCall(s, { id: "tc-9", title: "bash", kind: "execute", status: "running" })
  const upd = registry.updateToolCall(s, "tc-9", { status: "done", output: "ok" })
  assert.ok(upd.msgId)
  const snap = registry.snapshot(s)
  const tc = snap.messages.flatMap((m) => m.toolCalls).find((t) => t.id === "tc-9")
  assert.equal(tc.status, "done")
  assert.equal(tc.output, "ok")
})
