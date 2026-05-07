import { useState } from "react";
import {
  AlertDialog,
  AlertDialogBackdrop,
  AlertDialogPopup,
} from "./Dialog";
import { Tooltip } from "./Tooltip";
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
};

const SIDEBAR_BTN =
  "w-7 h-7 flex items-center justify-center rounded-md border border-border bg-transparent text-fg-dim hover:bg-bg-soft hover:text-fg cursor-pointer text-sm leading-none";

const FULL_BTN =
  "w-full rounded-md border border-border bg-transparent px-2.5 py-2 text-left text-[13px] text-fg cursor-pointer hover:bg-bg-soft";

const ALERT_BTN =
  "rounded-md border border-border bg-transparent px-2.5 py-1.5 text-[13px] text-fg cursor-pointer hover:bg-bg-soft";

export function Sidebar({
  conversations,
  currentId,
  collapsed,
  onToggle,
  onSelect,
  onNew,
  onRename,
  onDelete,
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
      <aside className="flex w-10 flex-col items-center gap-1.5 border-r border-border-soft bg-bg-elev py-2">
        <Tooltip
          label="Expand sidebar"
          className={SIDEBAR_BTN}
          onClick={onToggle}
        >
          »
        </Tooltip>
        <Tooltip
          label="New chat"
          className={SIDEBAR_BTN}
          onClick={onNew}
        >
          +
        </Tooltip>
      </aside>
    );
  }

  return (
    <aside className="flex w-60 flex-col border-r border-border-soft bg-bg-elev">
      <div className="flex items-center gap-1.5 border-b border-border-soft p-2.5">
        <button className={FULL_BTN} onClick={onNew}>
          + New chat
        </button>
        <Tooltip
          label="Collapse sidebar"
          className={SIDEBAR_BTN}
          onClick={onToggle}
        >
          «
        </Tooltip>
      </div>

      <ul className="flex-1 list-none overflow-y-auto p-1.5 m-0">
        {sorted.map((c) => {
          const isCurrent = c.id === currentId;
          const isRenaming = renamingId === c.id;
          return (
            <li
              key={c.id}
              className={`group relative flex items-center gap-1.5 rounded-md px-2 py-1.5 text-[13px] text-fg cursor-pointer hover:bg-bg-soft ${
                isCurrent ? "bg-accent-soft" : ""
              }`}
              onClick={() => !isRenaming && onSelect(c.id)}
            >
              {isRenaming ? (
                <input
                  className="flex-1 rounded border border-accent bg-bg px-1 py-0.5 text-fg outline-none"
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
                <span
                  className="flex-1 overflow-hidden text-ellipsis whitespace-nowrap"
                  title={c.title}
                >
                  {c.title || "Untitled"}
                </span>
              )}
              {!isRenaming && (
                <span
                  className="hidden gap-0.5 group-hover:inline-flex"
                  onClick={(e) => e.stopPropagation()}
                >
                  <button
                    className="cursor-pointer rounded border-none bg-transparent px-1.5 py-0.5 text-[11px] text-fg-dim hover:bg-bg-soft hover:text-fg"
                    title="Rename"
                    onClick={() => startRename(c)}
                  >
                    edit
                  </button>
                  <button
                    className="cursor-pointer rounded border-none bg-transparent px-1.5 py-0.5 text-[11px] text-fg-dim hover:bg-bg-soft hover:text-danger"
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
          <li className="p-2.5 text-[12px] italic text-fg-dim">
            No conversations yet.
          </li>
        )}
      </ul>

      <AlertDialog.Root
        open={!!pendingDelete}
        onOpenChange={(v) => !v && setPendingDelete(null)}
      >
        <AlertDialog.Portal>
          <AlertDialogBackdrop />
          <AlertDialogPopup className="flex w-[360px] flex-col gap-2.5 p-4">
            <AlertDialog.Title className="m-0 text-[1.05em] font-semibold">
              Delete conversation?
            </AlertDialog.Title>
            <AlertDialog.Description className="m-0 text-[13px] leading-snug text-fg-dim">
              "{pendingDelete?.title}" will be removed. This can't be undone.
            </AlertDialog.Description>
            <div className="mt-1.5 flex justify-end gap-2">
              <AlertDialog.Close className={ALERT_BTN}>
                Cancel
              </AlertDialog.Close>
              <AlertDialog.Close
                className="cursor-pointer rounded-md border border-danger bg-danger px-2.5 py-1.5 text-[13px] text-white hover:brightness-105"
                onClick={() => {
                  if (pendingDelete) onDelete(pendingDelete.id);
                  setPendingDelete(null);
                }}
              >
                Delete
              </AlertDialog.Close>
            </div>
          </AlertDialogPopup>
        </AlertDialog.Portal>
      </AlertDialog.Root>
    </aside>
  );
}
