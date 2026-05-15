import React, { useState } from "react";
import { VaultEntry, stripMdExt } from "./vault";

export type CtxTarget =
  | { kind: "root" }
  | { kind: "file"; path: string }
  | { kind: "dir"; path: string };

type Props = {
  entries: VaultEntry[];
  activePath: string | null;
  onOpen: (path: string) => void;
  onDelete: (path: string) => void;
  onRename: (path: string) => void;
  onContextMenu: (x: number, y: number, target: CtxTarget) => void;
  // Move from -> targetDir (null = vault root). Folder moves are
  // rejected if targetDir is the source itself or a descendant.
  onMove: (from: string, targetDir: string | null) => void;
  // When true, ignore each folder's local expanded state and render
  // every level open. Used while a filter is active so matches are
  // visible without manual expansion.
  forceExpanded?: boolean;
};

const DRAG_MIME = "application/x-rezon-vault-path";

export function FileTree({
  entries,
  activePath,
  onOpen,
  onDelete,
  onRename,
  onContextMenu,
  onMove,
  forceExpanded,
}: Props) {
  const [rootHover, setRootHover] = useState(false);
  return (
    <div
      className={`flex min-h-full flex-col text-[13px] ${
        rootHover ? "bg-accent-soft/30" : ""
      }`}
      onContextMenu={(e) => {
        e.preventDefault();
        onContextMenu(e.clientX, e.clientY, { kind: "root" });
      }}
      onDragOver={(e) => {
        if (!e.dataTransfer.types.includes(DRAG_MIME)) return;
        e.preventDefault();
        e.dataTransfer.dropEffect = "move";
        setRootHover(true);
      }}
      onDragLeave={(e) => {
        // Only clear when leaving the tree itself, not when crossing
        // into a child element.
        if (e.currentTarget === e.target) setRootHover(false);
      }}
      onDrop={(e) => {
        if (!e.dataTransfer.types.includes(DRAG_MIME)) return;
        e.preventDefault();
        setRootHover(false);
        const from = e.dataTransfer.getData(DRAG_MIME);
        if (from) onMove(from, null);
      }}
    >
      {entries.map((e) => (
        <TreeNode
          key={e.path}
          entry={e}
          depth={0}
          activePath={activePath}
          onOpen={onOpen}
          onDelete={onDelete}
          onRename={onRename}
          onContextMenu={onContextMenu}
          onMove={onMove}
          forceExpanded={forceExpanded}
        />
      ))}
    </div>
  );
}

function TreeNode({
  entry,
  depth,
  activePath,
  onOpen,
  onDelete,
  onRename,
  onContextMenu,
  onMove,
  forceExpanded,
}: {
  entry: VaultEntry;
  depth: number;
  activePath: string | null;
  onOpen: (path: string) => void;
  onDelete: (path: string) => void;
  onRename: (path: string) => void;
  onContextMenu: (x: number, y: number, target: CtxTarget) => void;
  onMove: (from: string, targetDir: string | null) => void;
  forceExpanded?: boolean;
}) {
  const [localExpanded, setLocalExpanded] = useState(true);
  const expanded = forceExpanded || localExpanded;
  const setExpanded = (v: boolean | ((p: boolean) => boolean)) =>
    setLocalExpanded(v);
  const [dropHover, setDropHover] = useState(false);
  const pad = { paddingLeft: 6 + depth * 12 };

  const dragProps = {
    draggable: true,
    onDragStart: (e: React.DragEvent) => {
      e.dataTransfer.setData(DRAG_MIME, entry.path);
      e.dataTransfer.effectAllowed = "move";
    },
  };

  if (entry.kind === "dir") {
    return (
      <div>
        <button
          type="button"
          className={`flex w-full cursor-pointer items-center gap-1 border-none py-0.5 pr-2 text-left text-fg ${
            dropHover ? "bg-accent-soft" : "bg-transparent hover:bg-bg-soft"
          }`}
          style={pad}
          {...dragProps}
          onDragOver={(e) => {
            if (!e.dataTransfer.types.includes(DRAG_MIME)) return;
            e.preventDefault();
            e.stopPropagation();
            e.dataTransfer.dropEffect = "move";
            setDropHover(true);
          }}
          onDragLeave={() => setDropHover(false)}
          onDrop={(e) => {
            if (!e.dataTransfer.types.includes(DRAG_MIME)) return;
            e.preventDefault();
            e.stopPropagation();
            setDropHover(false);
            const from = e.dataTransfer.getData(DRAG_MIME);
            if (from && from !== entry.path) onMove(from, entry.path);
          }}
          onClick={() => setExpanded((x) => !x)}
          onContextMenu={(e) => {
            e.preventDefault();
            e.stopPropagation();
            onContextMenu(e.clientX, e.clientY, {
              kind: "dir",
              path: entry.path,
            });
          }}
        >
          <span className="opacity-60">{expanded ? "▾" : "▸"}</span>
          <span className="truncate">{entry.name}</span>
        </button>
        {expanded && (
          <div>
            {entry.children.map((c) => (
              <TreeNode
                key={c.path}
                entry={c}
                depth={depth + 1}
                activePath={activePath}
                onOpen={onOpen}
                onDelete={onDelete}
                onRename={onRename}
                onContextMenu={onContextMenu}
                onMove={onMove}
                forceExpanded={forceExpanded}
              />
            ))}
          </div>
        )}
      </div>
    );
  }

  const active = entry.path === activePath;
  return (
    <div
      className={`group flex items-center justify-between gap-1 pr-2 text-fg ${
        active ? "bg-accent-soft" : "hover:bg-bg-soft"
      }`}
      style={pad}
      {...dragProps}
      onContextMenu={(e) => {
        e.preventDefault();
        e.stopPropagation();
        onContextMenu(e.clientX, e.clientY, {
          kind: "file",
          path: entry.path,
        });
      }}
    >
      <button
        type="button"
        className="flex flex-1 cursor-pointer items-center gap-1 border-none bg-transparent py-0.5 text-left"
        onClick={() => onOpen(entry.path)}
        title={entry.path}
      >
        <span className="opacity-50">·</span>
        <span className="truncate">{stripMdExt(entry.name)}</span>
      </button>
      <div className="hidden gap-1 text-[11px] opacity-60 group-hover:flex">
        <button
          type="button"
          className="cursor-pointer border-none bg-transparent text-inherit"
          onClick={(e) => {
            e.stopPropagation();
            onRename(entry.path);
          }}
          title="Rename"
        >
          ✎
        </button>
        <button
          type="button"
          className="cursor-pointer border-none bg-transparent text-inherit"
          onClick={(e) => {
            e.stopPropagation();
            onDelete(entry.path);
          }}
          title="Delete"
        >
          ✕
        </button>
      </div>
    </div>
  );
}
