import { useState } from "react";
import { Conversation } from "./types";

type Props = {
  conversations: Conversation[];
  currentId: string | null;
  onSelect: (id: string) => void;
  onNew: () => void;
  onRename: (id: string, title: string) => void;
  onDelete: (id: string) => void;
  onOpenSettings: () => void;
};

export function Sidebar({
  conversations,
  currentId,
  onSelect,
  onNew,
  onRename,
  onDelete,
  onOpenSettings,
}: Props) {
  const [renamingId, setRenamingId] = useState<string | null>(null);
  const [renameDraft, setRenameDraft] = useState("");

  const sorted = [...conversations].sort((a, b) => b.updatedAt - a.updatedAt);

  function startRename(c: Conversation) {
    setRenamingId(c.id);
    setRenameDraft(c.title);
  }

  function commitRename() {
    if (renamingId) {
      const trimmed = renameDraft.trim();
      if (trimmed) onRename(renamingId, trimmed);
    }
    setRenamingId(null);
  }

  return (
    <aside className="sidebar">
      <div className="sidebar-top">
        <button className="new-chat" onClick={onNew}>
          + New chat
        </button>
      </div>
      <ul className="conv-list">
        {sorted.map((c) => {
          const isCurrent = c.id === currentId;
          const isRenaming = renamingId === c.id;
          return (
            <li
              key={c.id}
              className={`conv-item${isCurrent ? " current" : ""}`}
              onClick={() => !isRenaming && onSelect(c.id)}
            >
              {isRenaming ? (
                <input
                  className="conv-rename"
                  autoFocus
                  value={renameDraft}
                  onChange={(e) => setRenameDraft(e.currentTarget.value)}
                  onBlur={commitRename}
                  onKeyDown={(e) => {
                    if (e.key === "Enter") {
                      e.preventDefault();
                      commitRename();
                    } else if (e.key === "Escape") {
                      setRenamingId(null);
                    }
                  }}
                  onClick={(e) => e.stopPropagation()}
                />
              ) : (
                <span className="conv-title" title={c.title}>
                  {c.title || "Untitled"}
                </span>
              )}
              {!isRenaming && (
                <span className="conv-actions" onClick={(e) => e.stopPropagation()}>
                  <button
                    className="conv-action"
                    title="Rename"
                    onClick={() => startRename(c)}
                  >
                    edit
                  </button>
                  <button
                    className="conv-action conv-delete"
                    title="Delete"
                    onClick={() => {
                      if (confirm(`Delete "${c.title}"?`)) onDelete(c.id);
                    }}
                  >
                    del
                  </button>
                </span>
              )}
            </li>
          );
        })}
        {sorted.length === 0 && (
          <li className="conv-empty">No conversations yet.</li>
        )}
      </ul>
      <div className="sidebar-bottom">
        <button className="settings-btn" onClick={onOpenSettings}>
          Settings
        </button>
      </div>
    </aside>
  );
}
