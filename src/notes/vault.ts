import { invoke } from "@tauri-apps/api/core";

export type VaultEntry =
  | { kind: "file"; name: string; path: string }
  | { kind: "dir"; name: string; path: string; children: VaultEntry[] };

export type ResolvedLink = { path: string; created: boolean };

export async function listTree(vault: string): Promise<VaultEntry[]> {
  return invoke<VaultEntry[]>("vault_list_tree", { vault });
}

export async function readFile(vault: string, path: string): Promise<string> {
  return invoke<string>("vault_read", { vault, path });
}

export async function writeFile(
  vault: string,
  path: string,
  content: string,
): Promise<void> {
  return invoke<void>("vault_write", { vault, path, content });
}

export async function createFile(vault: string, path: string): Promise<void> {
  return invoke<void>("vault_create", { vault, path });
}

export async function mkdir(vault: string, path: string): Promise<void> {
  return invoke<void>("vault_mkdir", { vault, path });
}

export async function deletePath(vault: string, path: string): Promise<void> {
  return invoke<void>("vault_delete", { vault, path });
}

export async function renamePath(
  vault: string,
  from: string,
  to: string,
): Promise<void> {
  return invoke<void>("vault_rename", { vault, from, to });
}

export type UndoReport = { path: string; targetId: string };

/// Revert the most recent journaled vault mutation. Errors when
/// there is nothing to undo. The caller is responsible for refreshing
/// any open editor tab pointing at the returned path, since the file
/// on disk has just been replaced behind the editor's back.
export async function vaultUndo(vault: string): Promise<UndoReport> {
  return invoke<UndoReport>("vault_undo", { vault });
}

export type RedoReport = {
  path: string;
  targetUndoId: string;
  wasCreation: boolean;
};

/// Reapply the most recent journaled undo. Errors when nothing to
/// redo (no recent undo, or a fresh write has invalidated the redo
/// stack). Same refresh contract as `vaultUndo`.
export async function vaultRedo(vault: string): Promise<RedoReport> {
  return invoke<RedoReport>("vault_redo", { vault });
}

export type JournalEntryDto = {
  id: string;
  ts: number;
  tool: string;
  path: string;
  /// "write" | "undo"
  kind: string;
  beforeSha: string | null;
  afterSha: string | null;
  targetId: string | null;
};

/// Recent journal entries (default 100, newest first) for the open
/// vault. Lightweight read of `.rezon-history/log.jsonl` — safe to
/// call on every panel open without throttling.
export async function vaultJournalRecent(
  vault: string,
  limit?: number,
): Promise<JournalEntryDto[]> {
  return invoke<JournalEntryDto[]>("vault_journal_recent", { vault, limit });
}

export type SearchHit = { path: string; snippet: string };

export async function indexOpen(vault: string): Promise<void> {
  return invoke<void>("vault_index_open", { vault });
}

export async function indexTouch(vault: string, path: string): Promise<void> {
  return invoke<void>("vault_index_touch", { vault, path });
}

export async function search(
  vault: string,
  query: string,
  limit = 50,
): Promise<SearchHit[]> {
  return invoke<SearchHit[]>("vault_search", { vault, query, limit });
}

export async function searchSemantic(
  vault: string,
  query: string,
  limit = 20,
): Promise<SearchHit[]> {
  return invoke<SearchHit[]>("vault_search_semantic", { vault, query, limit });
}

export type RelatedHit = { path: string; score: number; snippet: string };
export type EmbedStatus = {
  loaded: boolean;
  path: string | null;
  dim: number | null;
};

export async function related(
  vault: string,
  path: string,
  limit = 8,
): Promise<RelatedHit[]> {
  return invoke<RelatedHit[]>("vault_related", { vault, path, limit });
}

export async function embedStatus(): Promise<EmbedStatus> {
  return invoke<EmbedStatus>("embed_status");
}

export async function embedLoadModel(path: string): Promise<EmbedStatus> {
  return invoke<EmbedStatus>("embed_load_model", { path });
}

export async function resolveWikilink(
  vault: string,
  target: string,
  createIfMissing: boolean,
): Promise<ResolvedLink> {
  return invoke<ResolvedLink>("vault_resolve_wikilink", {
    vault,
    target,
    createIfMissing,
  });
}

// ---- Persistence -----------------------------------------------------

const KEY_VAULT = "rezon:vaultPath";
const KEY_TABS = "rezon:vaultTabs";
const KEY_ACTIVE = "rezon:vaultActive";
const KEY_MODE = "rezon:appMode";

export function loadVaultPath(): string | null {
  return localStorage.getItem(KEY_VAULT);
}
export function saveVaultPath(p: string | null) {
  if (p) localStorage.setItem(KEY_VAULT, p);
  else localStorage.removeItem(KEY_VAULT);
}

export function loadOpenTabs(): string[] {
  try {
    const raw = localStorage.getItem(KEY_TABS);
    if (!raw) return [];
    const parsed = JSON.parse(raw);
    return Array.isArray(parsed) ? parsed.filter((x) => typeof x === "string") : [];
  } catch {
    return [];
  }
}
export function saveOpenTabs(tabs: string[]) {
  localStorage.setItem(KEY_TABS, JSON.stringify(tabs));
}

export function loadActiveTab(): string | null {
  return localStorage.getItem(KEY_ACTIVE);
}
export function saveActiveTab(p: string | null) {
  if (p) localStorage.setItem(KEY_ACTIVE, p);
  else localStorage.removeItem(KEY_ACTIVE);
}

export type AppMode = "chat" | "notes";
export function loadAppMode(): AppMode {
  const v = localStorage.getItem(KEY_MODE);
  return v === "notes" ? "notes" : "chat";
}
export function saveAppMode(m: AppMode) {
  localStorage.setItem(KEY_MODE, m);
}

export function basename(p: string): string {
  const parts = p.split(/[/\\]/);
  return parts[parts.length - 1] || p;
}

export function stripMdExt(name: string): string {
  return name.replace(/\.(md|markdown)$/i, "");
}
