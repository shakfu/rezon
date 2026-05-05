import { useState } from "react";
import * as AlertDialog from "@radix-ui/react-alert-dialog";
import * as Tooltip from "@radix-ui/react-tooltip";
import { Conversation } from "./types";

type Props = {
  conversations: Conversation[];
  currentId: string | null;
  collapsed: boolean;
  onToggle: () => void;
  onSelect: (id: string) => void;
  onNew: () => void;
  onRename: (id: string, title: string) => void;
  onDelete: (id: string) => void;
  onOpenSettings: () => void;
};

function IconButton({
  label,
  children,
  ...rest
}: {
  label: string;
  children: React.ReactNode;
} & React.ButtonHTMLAttributes<HTMLButtonElement>) {
  return (
    <Tooltip.Root>
      <Tooltip.Trigger asChild>
        <button {...rest}>{children}</button>
      </Tooltip.Trigger>
      <Tooltip.Portal>
        <Tooltip.Content className="tooltip" side="right" sideOffset={6}>
          {label}
          <Tooltip.Arrow className="tooltip-arrow" />
        </Tooltip.Content>
      </Tooltip.Portal>
    </Tooltip.Root>
  );
}

export function Sidebar({
  conversations,
  currentId,
  collapsed,
  onToggle,
  onSelect,
  onNew,
  onRename,
  onDelete,
  onOpenSettings,
}: Props) {
  const [renamingId, setRenamingId] = useState<string | null>(null);
  const [renameDraft, setRenameDraft] = useState("");
  const [pendingDelete, setPendingDelete] = useState<Conversation | null>(null);

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

  if (collapsed) {
    return (
      <aside className="sidebar sidebar-collapsed">
        <IconButton
          label="Expand sidebar"
          className="sidebar-toggle"
          onClick={onToggle}
        >
          »
        </IconButton>
        <IconButton
          label="New chat"
          className="sidebar-toggle"
          onClick={onNew}
        >
          +
        </IconButton>
      </aside>
    );
  }

  return (
    <aside className="sidebar">
      <div className="sidebar-top">
        <button className="new-chat" onClick={onNew}>
          + New chat
        </button>
        <IconButton
          label="Collapse sidebar"
          className="sidebar-toggle"
          onClick={onToggle}
        >
          «
        </IconButton>
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
                <span
                  className="conv-actions"
                  onClick={(e) => e.stopPropagation()}
                >
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
                    onClick={() => setPendingDelete(c)}
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

      <AlertDialog.Root
        open={!!pendingDelete}
        onOpenChange={(v) => !v && setPendingDelete(null)}
      >
        <AlertDialog.Portal>
          <AlertDialog.Overlay className="dialog-overlay" />
          <AlertDialog.Content className="dialog dialog-alert">
            <AlertDialog.Title className="dialog-title">
              Delete conversation?
            </AlertDialog.Title>
            <AlertDialog.Description className="dialog-desc">
              "{pendingDelete?.title}" will be removed. This can't be undone.
            </AlertDialog.Description>
            <div className="dialog-actions">
              <AlertDialog.Cancel asChild>
                <button className="rs-btn">Cancel</button>
              </AlertDialog.Cancel>
              <AlertDialog.Action asChild>
                <button
                  className="rs-btn rs-btn-danger"
                  onClick={() => {
                    if (pendingDelete) onDelete(pendingDelete.id);
                    setPendingDelete(null);
                  }}
                >
                  Delete
                </button>
              </AlertDialog.Action>
            </div>
          </AlertDialog.Content>
        </AlertDialog.Portal>
      </AlertDialog.Root>
    </aside>
  );
}
