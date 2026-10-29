import React, { useState } from "react";
import { useStore } from "../store";
import { useTranslation, t } from "../i18n";

interface Props {
  config: import("../types").AppConfig;
}

const PencilIcon = (): React.JSX.Element => (
  <svg xmlns="http://www.w3.org/2000/svg" width="14" height="14" viewBox="0 0 24 24">
    <path d="M8 21L21.5 7.5L16.5 2.5L3 16L3 21L8 21Z" stroke="currentColor" strokeWidth="2" strokeLinecap="square" fill="none" />
  </svg>
);

const XIcon = (): React.JSX.Element => (
  <svg xmlns="http://www.w3.org/2000/svg" width="14" height="14" viewBox="0 0 24 24">
    <path d="M20 4L4 20" stroke="currentColor" strokeWidth="2" strokeLinecap="square" fill="none" />
    <path d="M4 4L20 20" stroke="currentColor" strokeWidth="2" strokeLinecap="square" fill="none" />
  </svg>
);

const CheckIcon = (): React.JSX.Element => (
  <svg xmlns="http://www.w3.org/2000/svg" width="14" height="14" viewBox="0 0 24 24">
    <path d="M20 6L9 17L4 12" stroke="currentColor" strokeWidth="2" strokeLinecap="square" fill="none" />
  </svg>
);

export default function MessageQueue({ config }: Props): React.JSX.Element | null {
  // v1.0.37: only THIS session's prompts. The server queue is global; showing
  // everything made prompts of other sessions look like they belonged here
  // (and "send now" then delivered them into the wrong chat).
  const messageQueue = useStore((st) => st.messageQueue.filter((m) => {
    const sess = st.sessions.find((x) => x.id === st.currentSessionId);
    return !m.acpSessionId || !sess?.acpSessionId || m.acpSessionId === sess.acpSessionId;
  }));
  const { removeFromMessageQueue, updateQueueMessage, clearMessageQueue, sendQueuedNow } = useStore();
  const currentAcpId = useStore((st) => st.sessions.find((x) => x.id === st.currentSessionId)?.acpSessionId);
  const tr = useTranslation(config.language);
  const [editingId, setEditingId] = useState<string | null>(null);
  const [editText, setEditText] = useState("");

  if (messageQueue.length === 0) return null;

  const count = messageQueue.length;
  const title = t(tr, count === 1 ? "messageInQueueOne" : "messagesInQueue", { count });

  const startEdit = (id: string, content: string) => {
    setEditingId(id);
    setEditText(content);
  };

  const saveEdit = (id: string) => {
    if (editText.trim()) {
      updateQueueMessage(id, editText.trim());
    } else {
      removeFromMessageQueue(id);
    }
    setEditingId(null);
    setEditText("");
  };

  const cancelEdit = () => {
    setEditingId(null);
    setEditText("");
  };

  return (
    <div className="msg-queue" role="list">
      <div className="msg-queue-header">
        <p className="msg-queue-title">{title}</p>
        <button
          type="button"
          className="msg-queue-clear"
          onClick={() => {
            for (const m of messageQueue) removeFromMessageQueue(m.id);
          }}
        >
          {tr.clearQueueLabel}
        </button>
      </div>
      <div className="msg-queue-list">
        {messageQueue.map((msg, idx) => (
          <div key={msg.id} className="msg-queue-item" role="listitem">
            <span className="msg-queue-num">{idx + 1}</span>
            {editingId === msg.id ? (
              <input
                className="msg-queue-edit"
                value={editText}
                onChange={(e) => setEditText(e.target.value)}
                onKeyDown={(e) => {
                  if (e.key === "Enter") saveEdit(msg.id);
                  if (e.key === "Escape") cancelEdit();
                }}
                autoFocus
              />
            ) : (
              <p className="msg-queue-text">{msg.content}</p>
            )}
            <div className="msg-queue-actions">
              {editingId === msg.id ? (
                <>
                  <button type="button" className="msg-queue-btn" onClick={() => saveEdit(msg.id)} title={tr.mqSave}>
                    <CheckIcon />
                  </button>
                  <button type="button" className="msg-queue-btn" onClick={cancelEdit} title={tr.mqCancel}>
                    <XIcon />
                  </button>
                </>
              ) : (
                <>
                  <button
                    type="button"
                    className="msg-queue-btn"
                    onClick={() => sendQueuedNow(msg.id)}
                    title={tr.mqSendNow}
                  >
                    <svg xmlns="http://www.w3.org/2000/svg" width="14" height="14" viewBox="0 0 24 24">
                      <path d="M13 2L4.5 13.5H11L9.5 22L19.5 9.5H13.5L13 2Z" stroke="currentColor" strokeWidth="2" strokeLinecap="square" fill="none" />
                    </svg>
                  </button>
                  <button type="button" className="msg-queue-btn" onClick={() => startEdit(msg.id, msg.content)} title={tr.mqEdit}>
                    <PencilIcon />
                  </button>
                  <button type="button" className="msg-queue-btn" onClick={() => removeFromMessageQueue(msg.id)} title={tr.mqRemove}>
                    <XIcon />
                  </button>
                </>
              )}
            </div>
          </div>
        ))}
      </div>
    </div>
  );
}
