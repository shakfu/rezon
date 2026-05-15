import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { open as openDialog } from "@tauri-apps/plugin-dialog";
import {
  VaultEntry,
  basename,
  createFile,
  deletePath,
  embedLoadModel,
  embedStatus,
  indexOpen,
  indexTouch,
  listTree,
  loadActiveTab,
  loadOpenTabs,
  loadVaultPath,
  mkdir,
  readFile,
  related as fetchRelated,
  renamePath,
  resolveWikilink,
  saveActiveTab,
  saveOpenTabs,
  saveVaultPath,
  search as searchVault,
  searchSemantic,
  stripMdExt,
  writeFile,
  type EmbedStatus,
  type RelatedHit,
  type SearchHit,
} from "./vault";
import { FileTree, type CtxTarget } from "./FileTree";
import { MilkdownEditor } from "./MilkdownEditor";

type TabState = {
  path: string;
  // Last markdown read from disk (or last write). Used to skip
  // redundant writes and as the editor's initial value.
  diskContent: string;
  // Current editor content. Differs from diskContent while the
  // debounced save is in flight.
  liveContent: string;
};

export function NotesView() {
  const [vault, setVault] = useState<string | null>(() => loadVaultPath());
  const [tree, setTree] = useState<VaultEntry[]>([]);
  const [tabs, setTabs] = useState<TabState[]>([]);
  const [activePath, setActivePath] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  // Inline prompt state. `window.prompt` does not work in Tauri's
  // webview, so we render a small input bar at the top of the sidebar
  // instead. `kind` tells the submit handler what to create / do.
  const [prompt, setPrompt] = useState<
    | { kind: "newFile"; parentDir?: string }
    | { kind: "newFolder"; parentDir?: string }
    | { kind: "rename"; path: string; initial: string }
    | null
  >(null);
  const [promptValue, setPromptValue] = useState("");
  const [pendingDelete, setPendingDelete] = useState<string | null>(null);
  const [ctxMenu, setCtxMenu] = useState<
    { x: number; y: number; target: CtxTarget } | null
  >(null);
  const [filter, setFilter] = useState("");
  const [embed, setEmbed] = useState<EmbedStatus>({
    loaded: false,
    path: null,
    dim: null,
  });
  const [relatedHits, setRelatedHits] = useState<RelatedHit[]>([]);
  // "filter" -> name-only client filter (instant, no IPC).
  // "search" -> debounced FTS5 call returning content snippets.
  // "semantic" -> debounced vector KNN; requires an embedding model.
  const [searchMode, setSearchMode] = useState<
    "filter" | "search" | "semantic"
  >("filter");
  const [hits, setHits] = useState<SearchHit[]>([]);
  const [searching, setSearching] = useState(false);
  const searchSeq = useRef(0);

  // Debounced save: a single timer per tab, refreshed on each edit.
  const saveTimers = useRef<Map<string, ReturnType<typeof setTimeout>>>(new Map());

  const refreshTree = useCallback(async (v: string) => {
    try {
      const t = await listTree(v);
      setTree(t);
    } catch (e) {
      setError(String(e));
    }
  }, []);

  // Track embed-model status. Polled lightly so the panel updates
  // when auto-load on app start finishes and when the user picks a
  // model from the dialog below.
  useEffect(() => {
    let alive = true;
    const tick = () => {
      embedStatus()
        .then((s) => {
          if (alive) setEmbed(s);
        })
        .catch(() => {});
    };
    tick();
    const id = setInterval(tick, 2500);
    return () => {
      alive = false;
      clearInterval(id);
    };
  }, []);

  // Initial load: tree + previously open tabs.
  useEffect(() => {
    if (!vault) return;
    refreshTree(vault);
    indexOpen(vault).catch((e) => setError(String(e)));
    (async () => {
      const persisted = loadOpenTabs();
      const initial: TabState[] = [];
      for (const p of persisted) {
        try {
          const content = await readFile(vault, p);
          initial.push({ path: p, diskContent: content, liveContent: content });
        } catch {
          // File gone since last session - drop the tab.
        }
      }
      setTabs(initial);
      const lastActive = loadActiveTab();
      const stillOpen = initial.find((t) => t.path === lastActive);
      setActivePath(stillOpen ? lastActive : initial[0]?.path ?? null);
    })();
  }, [vault, refreshTree]);

  // Persist tab list / active tab.
  useEffect(() => {
    saveOpenTabs(tabs.map((t) => t.path));
  }, [tabs]);
  useEffect(() => {
    saveActiveTab(activePath);
  }, [activePath]);

  const pickEmbedModel = useCallback(async () => {
    const picked = await openDialog({
      directory: false,
      multiple: false,
      filters: [{ name: "GGUF embedding model", extensions: ["gguf"] }],
    });
    if (typeof picked !== "string") return;
    try {
      const next = await embedLoadModel(picked);
      setEmbed(next);
    } catch (e) {
      setError(String(e));
    }
  }, []);

  const pickVault = useCallback(async () => {
    const picked = await openDialog({ directory: true, multiple: false });
    if (typeof picked === "string") {
      setVault(picked);
      saveVaultPath(picked);
      setTabs([]);
      setActivePath(null);
    }
  }, []);

  const openPath = useCallback(
    async (path: string) => {
      if (!vault) return;
      const existing = tabs.find((t) => t.path === path);
      if (existing) {
        setActivePath(path);
        return;
      }
      try {
        const content = await readFile(vault, path);
        setTabs((prev) => [
          ...prev,
          { path, diskContent: content, liveContent: content },
        ]);
        setActivePath(path);
      } catch (e) {
        setError(String(e));
      }
    },
    [vault, tabs],
  );

  const closeTab = useCallback(
    (path: string) => {
      // Flush a pending save before dropping the tab.
      const timer = saveTimers.current.get(path);
      if (timer) {
        clearTimeout(timer);
        saveTimers.current.delete(path);
        const t = tabs.find((x) => x.path === path);
        if (t && vault && t.liveContent !== t.diskContent) {
          writeFile(vault, path, t.liveContent).catch(() => {});
        }
      }
      setTabs((prev) => {
        const next = prev.filter((t) => t.path !== path);
        if (activePath === path) {
          setActivePath(next[next.length - 1]?.path ?? null);
        }
        return next;
      });
    },
    [tabs, activePath, vault],
  );

  const onEditorChange = useCallback(
    (path: string, markdown: string) => {
      if (!vault) return;
      setTabs((prev) =>
        prev.map((t) =>
          t.path === path ? { ...t, liveContent: markdown } : t,
        ),
      );
      const existing = saveTimers.current.get(path);
      if (existing) clearTimeout(existing);
      const timer = setTimeout(() => {
        saveTimers.current.delete(path);
        writeFile(vault, path, markdown)
          .then(() => {
            setTabs((prev) =>
              prev.map((t) =>
                t.path === path ? { ...t, diskContent: markdown } : t,
              ),
            );
            // Update FTS index immediately so search results don't
            // wait on the filesystem watcher (which can lag on macOS).
            indexTouch(vault, path).catch(() => {});
          })
          .catch((e) => setError(String(e)));
      }, 500);
      saveTimers.current.set(path, timer);
    },
    [vault],
  );

  const onWikilinkClick = useCallback(
    async (target: string) => {
      if (!vault) return;
      try {
        const resolved = await resolveWikilink(vault, target, true);
        if (resolved.created) await refreshTree(vault);
        await openPath(resolved.path);
      } catch (e) {
        setError(String(e));
      }
    },
    [vault, openPath, refreshTree],
  );

  const startPrompt = useCallback(
    (next: NonNullable<typeof prompt>) => {
      setPrompt(next);
      setPromptValue(next.kind === "rename" ? next.initial : "");
    },
    [],
  );

  const submitPrompt = useCallback(async () => {
    if (!vault || !prompt) return;
    const raw = promptValue.trim();
    if (!raw) {
      setPrompt(null);
      return;
    }
    const safe = raw.replace(/\\/g, "/").replace(/^\/+/, "");
    try {
      if (prompt.kind === "newFile") {
        const withExt = /\.(md|markdown)$/i.test(safe) ? safe : `${safe}.md`;
        const base = prompt.parentDir ?? vault;
        const path = `${base}/${withExt}`;
        await createFile(vault, path);
        await refreshTree(vault);
        await openPath(path);
      } else if (prompt.kind === "newFolder") {
        const base = prompt.parentDir ?? vault;
        const path = `${base}/${safe}`;
        await mkdir(vault, path);
        await refreshTree(vault);
      } else {
        const dir = prompt.path.slice(
          0,
          prompt.path.length - basename(prompt.path).length,
        );
        const to = dir + safe.replace(/\//g, "-");
        await renamePath(vault, prompt.path, to);
        setTabs((prev) =>
          prev.map((t) => (t.path === prompt.path ? { ...t, path: to } : t)),
        );
        if (activePath === prompt.path) setActivePath(to);
        await refreshTree(vault);
      }
      setPrompt(null);
      setPromptValue("");
    } catch (e) {
      setError(String(e));
    }
  }, [vault, prompt, promptValue, refreshTree, openPath, activePath]);

  const onDelete = useCallback((path: string) => {
    setPendingDelete(path);
  }, []);

  const confirmDelete = useCallback(async () => {
    if (!vault || !pendingDelete) return;
    const path = pendingDelete;
    setPendingDelete(null);
    try {
      await deletePath(vault, path);
      setTabs((prev) => prev.filter((t) => t.path !== path));
      if (activePath === path) setActivePath(null);
      await refreshTree(vault);
    } catch (e) {
      setError(String(e));
    }
  }, [vault, pendingDelete, activePath, refreshTree]);

  const onRename = useCallback(
    (path: string) => {
      startPrompt({ kind: "rename", path, initial: basename(path) });
    },
    [startPrompt],
  );

  const onMove = useCallback(
    async (from: string, targetDir: string | null) => {
      if (!vault) return;
      const destDir = targetDir ?? vault;
      // Reject moves into self or descendants for folder drags.
      if (destDir === from || destDir.startsWith(from + "/")) return;
      const to = `${destDir}/${basename(from)}`;
      if (to === from) return;
      try {
        await renamePath(vault, from, to);
        // Update any open tab paths affected by the move. Folder
        // moves rewrite the prefix of all descendant tab paths.
        setTabs((prev) =>
          prev.map((t) => {
            if (t.path === from) return { ...t, path: to };
            if (t.path.startsWith(from + "/")) {
              return { ...t, path: to + t.path.slice(from.length) };
            }
            return t;
          }),
        );
        setActivePath((cur) => {
          if (cur === from) return to;
          if (cur && cur.startsWith(from + "/")) {
            return to + cur.slice(from.length);
          }
          return cur;
        });
        await refreshTree(vault);
      } catch (e) {
        setError(String(e));
      }
    },
    [vault, refreshTree],
  );

  const activeTab = useMemo(
    () => tabs.find((t) => t.path === activePath) ?? null,
    [tabs, activePath],
  );

  // Related-notes refresh: triggered when the active file or embed
  // status changes. Embeddings catch up in the background after a
  // save; we re-fetch a couple seconds later so freshly-embedded
  // chunks show up without a manual reload.
  useEffect(() => {
    if (!vault || !activePath || !embed.loaded) {
      setRelatedHits([]);
      return;
    }
    let cancelled = false;
    const run = () => {
      fetchRelated(vault, activePath, 8)
        .then((hits) => {
          if (!cancelled) setRelatedHits(hits);
        })
        .catch(() => {});
    };
    run();
    // Retry once after embeddings have had a chance to catch up.
    const t = setTimeout(run, 3000);
    return () => {
      cancelled = true;
      clearTimeout(t);
    };
  }, [vault, activePath, embed.loaded, embed.path]);

  // Debounced FTS5 query. Each call gets a sequence number; only the
  // latest one is allowed to write into state, so out-of-order
  // resolutions can't overwrite fresh results.
  useEffect(() => {
    if ((searchMode !== "search" && searchMode !== "semantic") || !vault) {
      setHits([]);
      setSearching(false);
      return;
    }
    const q = filter.trim();
    if (!q) {
      setHits([]);
      setSearching(false);
      return;
    }
    const seq = ++searchSeq.current;
    setSearching(true);
    // Semantic queries do a model forward pass server-side, so debounce
    // them harder than FTS5 to keep the embedder responsive.
    const debounce = searchMode === "semantic" ? 350 : 200;
    const t = setTimeout(() => {
      const run =
        searchMode === "semantic"
          ? searchSemantic(vault, q)
          : searchVault(vault, q);
      run
        .then((rows) => {
          if (seq === searchSeq.current) {
            setHits(rows);
            setSearching(false);
          }
        })
        .catch((e) => {
          if (seq === searchSeq.current) {
            setError(String(e));
            setSearching(false);
          }
        });
    }, debounce);
    return () => clearTimeout(t);
  }, [filter, searchMode, vault]);

  // Filter the tree by case-insensitive name substring. Folders are
  // kept iff any descendant file matches; matching folders keep their
  // full subtree so the user can see context around the hit.
  const filteredTree = useMemo(() => {
    const q = filter.trim().toLowerCase();
    if (!q) return tree;
    const prune = (entries: VaultEntry[]): VaultEntry[] => {
      const out: VaultEntry[] = [];
      for (const e of entries) {
        if (e.kind === "file") {
          if (stripMdExt(e.name).toLowerCase().includes(q)) out.push(e);
        } else {
          const selfHit = e.name.toLowerCase().includes(q);
          const kids = selfHit ? e.children : prune(e.children);
          if (selfHit || kids.length > 0) {
            out.push({ ...e, children: kids });
          }
        }
      }
      return out;
    };
    return prune(tree);
  }, [tree, filter]);

  if (!vault) {
    return (
      <div className="flex h-full flex-col items-center justify-center gap-3 p-6 text-center">
        <div className="text-[14px] opacity-70">
          Pick a folder to use as your vault.
        </div>
        <button
          type="button"
          className="cursor-pointer rounded-md border-none bg-accent px-3 py-1.5 text-[13px] font-semibold text-white"
          onClick={pickVault}
        >
          Choose folder...
        </button>
      </div>
    );
  }

  return (
    <div className="flex h-full min-h-0 flex-1">
      <aside className="flex w-60 shrink-0 flex-col border-r border-border-soft">
        <div className="flex items-center justify-between border-b border-border-soft px-2 py-1.5 text-[11px] uppercase tracking-wider text-fg-dim">
          <span className="truncate" title={vault}>
            {basename(vault)}
          </span>
          <div className="flex gap-1 normal-case tracking-normal">
            <button
              type="button"
              className="cursor-pointer border-none bg-transparent text-inherit"
              onClick={() => startPrompt({ kind: "newFile" })}
              title="New note"
            >
              +
            </button>
            <button
              type="button"
              className="cursor-pointer border-none bg-transparent text-inherit"
              onClick={() => startPrompt({ kind: "newFolder" })}
              title="New folder"
            >
              ▣
            </button>
            <button
              type="button"
              className="cursor-pointer border-none bg-transparent text-inherit"
              onClick={() => refreshTree(vault)}
              title="Refresh"
            >
              ⟳
            </button>
            <button
              type="button"
              className="cursor-pointer border-none bg-transparent text-inherit"
              onClick={pickEmbedModel}
              title={
                embed.loaded
                  ? `Embed model: ${embed.path ?? "?"} (dim ${embed.dim ?? "?"})`
                  : "Load embedding model (for related-notes)"
              }
            >
              {embed.loaded ? "◆" : "◇"}
            </button>
            <button
              type="button"
              className="cursor-pointer border-none bg-transparent text-inherit"
              onClick={pickVault}
              title="Change vault"
            >
              …
            </button>
          </div>
        </div>
        <div className="flex flex-col gap-1 border-b border-border-soft px-2 py-1.5">
          <div className="flex items-center gap-0.5 rounded-sm border border-border-soft p-0.5 text-[10px]">
            <button
              type="button"
              className={`flex-1 cursor-pointer rounded-sm border-none px-1 py-0.5 text-inherit ${
                searchMode === "filter"
                  ? "bg-accent text-white"
                  : "bg-transparent"
              }`}
              onClick={() => setSearchMode("filter")}
            >
              Filter
            </button>
            <button
              type="button"
              className={`flex-1 cursor-pointer rounded-sm border-none px-1 py-0.5 text-inherit ${
                searchMode === "search"
                  ? "bg-accent text-white"
                  : "bg-transparent"
              }`}
              onClick={() => setSearchMode("search")}
            >
              Text
            </button>
            <button
              type="button"
              disabled={!embed.loaded}
              title={
                embed.loaded
                  ? "Vector similarity search"
                  : "Load an embedding model first"
              }
              className={`flex-1 cursor-pointer rounded-sm border-none px-1 py-0.5 text-inherit disabled:cursor-not-allowed disabled:opacity-40 ${
                searchMode === "semantic"
                  ? "bg-accent text-white"
                  : "bg-transparent"
              }`}
              onClick={() => setSearchMode("semantic")}
            >
              Semantic
            </button>
          </div>
          <input
            className="w-full rounded-sm border border-border bg-transparent px-1.5 py-0.5 text-[12px] text-fg outline-none focus-visible:ring-1 focus-visible:ring-accent"
            placeholder={
              searchMode === "filter"
                ? "Filter by name..."
                : searchMode === "search"
                  ? "Full-text search..."
                  : "Semantic search..."
            }
            value={filter}
            onChange={(e) => setFilter(e.currentTarget.value)}
            onKeyDown={(e) => {
              if (e.key === "Escape") {
                e.preventDefault();
                setFilter("");
              }
            }}
          />
        </div>
        {prompt && (
          <form
            className="flex items-center gap-1 border-b border-border-soft px-2 py-1.5"
            onSubmit={(e) => {
              e.preventDefault();
              submitPrompt();
            }}
          >
            <input
              autoFocus
              className="flex-1 rounded-sm border border-border bg-transparent px-1.5 py-0.5 text-[12px] text-fg outline-none focus-visible:ring-1 focus-visible:ring-accent"
              value={promptValue}
              placeholder={
                prompt.kind === "newFile"
                  ? "note-name"
                  : prompt.kind === "newFolder"
                    ? "folder-name"
                    : "new name"
              }
              onChange={(e) => setPromptValue(e.currentTarget.value)}
              onKeyDown={(e) => {
                if (e.key === "Escape") {
                  e.preventDefault();
                  setPrompt(null);
                  setPromptValue("");
                }
              }}
            />
            <button
              type="submit"
              className="cursor-pointer rounded-sm border-none bg-accent px-2 py-0.5 text-[11px] font-semibold text-white"
            >
              OK
            </button>
            <button
              type="button"
              className="cursor-pointer rounded-sm border border-border bg-transparent px-2 py-0.5 text-[11px] text-fg"
              onClick={() => {
                setPrompt(null);
                setPromptValue("");
              }}
            >
              Cancel
            </button>
          </form>
        )}
        <div className="flex-1 overflow-y-auto py-1">
          {(searchMode === "search" || searchMode === "semantic") &&
          filter.trim() !== "" ? (
            <SearchResults
              hits={hits}
              searching={searching}
              activePath={activePath}
              onOpen={openPath}
            />
          ) : (
            <FileTree
              entries={filteredTree}
              activePath={activePath}
              forceExpanded={filter.trim() !== ""}
              onOpen={openPath}
              onDelete={onDelete}
              onRename={onRename}
              onContextMenu={(x, y, target) => setCtxMenu({ x, y, target })}
              onMove={onMove}
            />
          )}
        </div>
      </aside>

      <main className="flex min-w-0 flex-1 flex-col">
        <div className="flex shrink-0 items-center gap-1 overflow-x-auto border-b border-border-soft bg-bg-soft/40 px-1 py-1">
          {tabs.length === 0 && (
            <span className="px-2 text-[12px] opacity-50">
              Open a note from the sidebar.
            </span>
          )}
          {tabs.map((t) => {
            const active = t.path === activePath;
            const dirty = t.liveContent !== t.diskContent;
            return (
              <div
                key={t.path}
                className={`flex shrink-0 items-center gap-1 rounded-md border px-2 py-0.5 text-[12px] ${
                  active
                    ? "border-border bg-bg text-fg"
                    : "border-transparent text-fg-dim hover:bg-bg-soft"
                }`}
              >
                <button
                  type="button"
                  className="cursor-pointer border-none bg-transparent text-inherit"
                  onClick={() => setActivePath(t.path)}
                  title={t.path}
                >
                  {stripMdExt(basename(t.path))}
                  {dirty ? " •" : ""}
                </button>
                <button
                  type="button"
                  className="cursor-pointer border-none bg-transparent text-inherit opacity-60 hover:opacity-100"
                  onClick={() => closeTab(t.path)}
                  title="Close tab"
                >
                  ×
                </button>
              </div>
            );
          })}
        </div>

        {error && (
          <div className="flex items-start gap-2 whitespace-pre-wrap bg-danger-soft px-3 py-2 text-[12px] text-danger">
            <span className="flex-1">{error}</span>
            <button
              type="button"
              className="cursor-pointer border-none bg-transparent text-inherit"
              onClick={() => setError(null)}
            >
              ×
            </button>
          </div>
        )}

        <div className="flex-1 overflow-y-auto">
          {activeTab ? (
            <div key={activeTab.path} className="mx-auto max-w-3xl px-6 py-6">
              <MilkdownEditor
                initialMarkdown={activeTab.diskContent}
                onChange={(md) => onEditorChange(activeTab.path, md)}
                onWikilinkClick={onWikilinkClick}
              />
              {embed.loaded && relatedHits.length > 0 && (
                <RelatedPanel
                  hits={relatedHits}
                  onOpen={openPath}
                />
              )}
            </div>
          ) : (
            <div className="p-6 text-center opacity-50">
              No note open.
            </div>
          )}
        </div>
      </main>
      {ctxMenu && (
        <ContextMenu
          x={ctxMenu.x}
          y={ctxMenu.y}
          target={ctxMenu.target}
          onClose={() => setCtxMenu(null)}
          onNewFileHere={(parentDir) => {
            setCtxMenu(null);
            startPrompt({ kind: "newFile", parentDir });
          }}
          onNewFolderHere={(parentDir) => {
            setCtxMenu(null);
            startPrompt({ kind: "newFolder", parentDir });
          }}
          onRename={(path) => {
            setCtxMenu(null);
            onRename(path);
          }}
          onDelete={(path) => {
            setCtxMenu(null);
            onDelete(path);
          }}
        />
      )}
      {pendingDelete && (
        <div className="fixed inset-0 z-40 flex items-center justify-center bg-black/40">
          <div className="w-[360px] max-w-[90vw] rounded-md border border-border bg-bg p-4 text-[13px] text-fg shadow-lg">
            <div className="mb-3">
              Delete <strong>{basename(pendingDelete)}</strong>?
            </div>
            <div className="flex justify-end gap-2">
              <button
                type="button"
                className="cursor-pointer rounded-md border border-border bg-transparent px-3 py-1.5 text-fg"
                onClick={() => setPendingDelete(null)}
              >
                Cancel
              </button>
              <button
                type="button"
                className="cursor-pointer rounded-md border-none bg-danger px-3 py-1.5 font-semibold text-white"
                onClick={confirmDelete}
              >
                Delete
              </button>
            </div>
          </div>
        </div>
      )}
    </div>
  );
}

function RelatedPanel({
  hits,
  onOpen,
}: {
  hits: RelatedHit[];
  onOpen: (path: string) => void;
}) {
  return (
    <div className="mt-8 border-t border-border-soft pt-4">
      <div className="mb-2 text-[11px] uppercase tracking-wider text-fg-dim">
        Related notes
      </div>
      <div className="flex flex-col gap-1.5">
        {hits.map((h) => (
          <button
            key={h.path}
            type="button"
            className="flex flex-col items-start gap-0.5 rounded-md border-none bg-bg-soft/50 px-3 py-2 text-left hover:bg-bg-soft"
            onClick={() => onOpen(h.path)}
            title={h.path}
          >
            <div className="flex w-full items-baseline justify-between gap-2">
              <span className="truncate text-[12px] font-medium text-fg">
                {stripMdExt(basename(h.path))}
              </span>
              <span className="shrink-0 font-mono text-[10px] text-fg-dim">
                {h.score.toFixed(2)}
              </span>
            </div>
            <span className="line-clamp-2 text-[11px] text-fg-dim">
              {h.snippet}
            </span>
          </button>
        ))}
      </div>
    </div>
  );
}

function SearchResults({
  hits,
  searching,
  activePath,
  onOpen,
}: {
  hits: SearchHit[];
  searching: boolean;
  activePath: string | null;
  onOpen: (path: string) => void;
}) {
  if (searching && hits.length === 0) {
    return <div className="px-3 py-2 text-[12px] opacity-50">Searching...</div>;
  }
  if (hits.length === 0) {
    return <div className="px-3 py-2 text-[12px] opacity-50">No matches.</div>;
  }
  return (
    <div className="flex flex-col">
      {hits.map((h) => {
        const active = h.path === activePath;
        return (
          <button
            key={h.path}
            type="button"
            className={`flex flex-col items-start gap-0.5 border-none px-2 py-1.5 text-left text-fg ${
              active ? "bg-accent-soft" : "bg-transparent hover:bg-bg-soft"
            }`}
            onClick={() => onOpen(h.path)}
            title={h.path}
          >
            <span className="truncate text-[12px] font-medium">
              {stripMdExt(basename(h.path))}
            </span>
            <span
              className="line-clamp-2 text-[11px] opacity-70"
              dangerouslySetInnerHTML={{ __html: renderSnippet(h.snippet) }}
            />
          </button>
        );
      })}
    </div>
  );
}

// FTS5 emits `<<match>>` markers (configured in the snippet() call).
// Convert to a styled <strong> while escaping all other HTML.
function renderSnippet(s: string): string {
  const escape = (t: string) =>
    t
      .replace(/&/g, "&amp;")
      .replace(/</g, "&lt;")
      .replace(/>/g, "&gt;");
  return escape(s)
    .replace(/&lt;&lt;/g, '<strong class="text-fg">')
    .replace(/&gt;&gt;/g, "</strong>");
}

function ContextMenu({
  x,
  y,
  target,
  onClose,
  onNewFileHere,
  onNewFolderHere,
  onRename,
  onDelete,
}: {
  x: number;
  y: number;
  target: CtxTarget;
  onClose: () => void;
  onNewFileHere: (parentDir: string | undefined) => void;
  onNewFolderHere: (parentDir: string | undefined) => void;
  onRename: (path: string) => void;
  onDelete: (path: string) => void;
}) {
  const items: { label: string; action: () => void }[] = [];
  if (target.kind === "root") {
    items.push({ label: "New note", action: () => onNewFileHere(undefined) });
    items.push({
      label: "New folder",
      action: () => onNewFolderHere(undefined),
    });
  } else if (target.kind === "dir") {
    items.push({
      label: "New note here",
      action: () => onNewFileHere(target.path),
    });
    items.push({
      label: "New folder here",
      action: () => onNewFolderHere(target.path),
    });
    items.push({ label: "Rename", action: () => onRename(target.path) });
    items.push({ label: "Delete folder", action: () => onDelete(target.path) });
  } else {
    items.push({ label: "Rename", action: () => onRename(target.path) });
    items.push({ label: "Delete", action: () => onDelete(target.path) });
  }

  // Clamp menu inside the viewport.
  const left = Math.min(x, window.innerWidth - 200);
  const top = Math.min(y, window.innerHeight - items.length * 28 - 8);

  return (
    <>
      <div
        className="fixed inset-0 z-40"
        onClick={onClose}
        onContextMenu={(e) => {
          e.preventDefault();
          onClose();
        }}
      />
      <div
        className="fixed z-50 min-w-[160px] rounded-md border border-border bg-bg py-1 text-[12px] text-fg shadow-lg"
        style={{ left, top }}
        onContextMenu={(e) => e.preventDefault()}
      >
        {items.map((it) => (
          <button
            key={it.label}
            type="button"
            className="block w-full cursor-pointer border-none bg-transparent px-3 py-1 text-left text-inherit hover:bg-bg-soft"
            onClick={it.action}
          >
            {it.label}
          </button>
        ))}
      </div>
    </>
  );
}
