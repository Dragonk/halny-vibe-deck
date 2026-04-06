import React, { useState } from "react";
import { useStore } from "../store";
import { useTranslation } from "../i18n";

interface Props {
  config: import("../types").AppConfig;
}

export default function MessageQueue({ config }: Props): React.JSX.Element | null {
  const { messageQueue, removeFromMessageQueue, updateQueueMessage, getNextQueuedMessage } = useStore();
  const tr = useTranslation(config.language);
  const [editingId, setEditingId] = useState<string | null>(null);
  const [editText, setEditText] = useState("");

  if (messageQueue.length === 0) return null;

  const handleEdit = (id: string, content: string) => {
    setEditingId(id);
    setEditText(content);
  };

  const handleSaveEdit = (id: string) => {
    if (editText.trim()) {
      updateQueueMessage(id, editText.trim());
    } else {
      removeFromMessageQueue(id);
    }
    setEditingId(null);
    setEditText("");
  };

  const handleCancelEdit = () => {
    setEditingId(null);
    setEditText("");
  };

  return (
    <div className="message-queue-panel">
      <div className="mq-header">
        <span className="mq-title">{tr.messageQueue} ({messageQueue.length})</span>
        <span className="mq-hint">{tr.messageQueueHint}</span>
      </div>
      <div className="mq-list">
        {messageQueue.map((msg, idx) => (
          <div key={msg.id} className="mq-item">
            <div className="mq-item-header">
              <span className="mq-item-num">#{idx + 1}</span>
              <div className="mq-item-actions">
                {editingId === msg.id ? (
                  <>
                    <button
                      className="mq-btn mq-btn-save"
                      onClick={() => handleSaveEdit(msg.id)}
                      title={tr.mqSave}
                    >
                      ✓
                    </button>
                    <button
                      className="mq-btn mq-btn-cancel"
                      onClick={handleCancelEdit}
                      title={tr.mqCancel}
                    >
                      ✕
                    </button>
                  </>
                ) : (
                  <>
                    <button
                      className="mq-btn mq-btn-edit"
                      onClick={() => handleEdit(msg.id, msg.content)}
                      title={tr.mqEdit}
                    >
                      ✎
                    </button>
                    <button
                      className="mq-btn mq-btn-remove"
                      onClick={() => removeFromMessageQueue(msg.id)}
                      title={tr.mqRemove}
                    >
                      ×
                    </button>
                  </>
                )}
              </div>
            </div>
            {editingId === msg.id ? (
              <textarea
                className="mq-item-edit-input"
                value={editText}
                onChange={(e) => setEditText(e.target.value)}
                autoFocus
              />
            ) : (
              <div className="mq-item-content">{msg.content}</div>
            )}
          </div>
        ))}
      </div>
    </div>
  );
}
