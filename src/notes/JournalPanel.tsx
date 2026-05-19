// Vault edit-history viewer. Modal dialog that reads
// `<vault>/.rezon-history/log.jsonl` via the `vault_journal_recent`
// Tauri command and renders entries newest-first. Read-only for v1
// — the undo / redo buttons in NotesView's toolbar are the action
// surface. A future iteration can add a per-row "revert here"
// button, but that requires multi-step undo semantics not yet
// in core.

import { useEffect, useState } from "react";
import { Dialog, DialogBackdrop, DialogPopup } from "../Dialog";
import {
  vaultJournalRecent,
  type JournalEntryDto,
} from "./vault";

export function JournalPanel({
  vault,
  open,
  onClose,
}: {
  vault: string | null;
  open: boolean;
  onClose: () => void;
}) {
  const [entries, setEntries] = useState<JournalEntryDto[]>([]);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);

  // Refresh whenever the dialog opens. Cheap (capped at 100 rows on
  // the backend); avoids the staleness pitfall of "loaded once on
  // mount."
  useEffect(() => {
    if (!open || !vault) return;
    let cancelled = false;
    setLoading(true);
    setError(null);
    vaultJournalRecent(vault)
      .then((rows) => {
        if (!cancelled) setEntries(rows);
      })
      .catch((e) => {
        if (!cancelled) setError(String(e));
      })
      .finally(() => {
        if (!cancelled) setLoading(false);
      });
    return () => {
      cancelled = true;
    };
  }, [open, vault]);

  return (
    <Dialog.Root open={open} onOpenChange={(v) => !v && onClose()}>
      <Dialog.Portal>
        <DialogBackdrop />
        <DialogPopup className="flex w-[640px] max-w-[92vw] max-h-[80vh] flex-col">
          <div className="flex items-center justify-between border-b border-border-soft px-4 py-3">
            <Dialog.Title className="m-0 text-[1.05em] font-semibold">
              Vault history
            </Dialog.Title>
            <Dialog.Close
              className="cursor-pointer border-none bg-transparent px-1 text-[22px] leading-none text-fg"
              aria-label="Close"
            >
              ×
            </Dialog.Close>
          </div>
          <Dialog.Description className="sr-only">
            Recent edits to the active vault. Read-only.
          </Dialog.Description>
          <div className="flex flex-col gap-2 overflow-y-auto px-4 py-3 text-[12px]">
            {loading && (
              <div className="text-fg-dim">Loading…</div>
            )}
            {error && (
              <div className="text-danger">{error}</div>
            )}
            {!loading && !error && entries.length === 0 && (
              <div className="text-fg-dim">
                No edits yet. Save a note or run an agent tool and the
                log will start filling in.
              </div>
            )}
            {!loading && !error && entries.length > 0 && (
              <table className="w-full border-collapse text-left">
                <thead>
                  <tr className="border-b border-border-soft text-[11px] uppercase tracking-wider text-fg-dim">
                    <th className="py-1 pr-2 font-normal">When</th>
                    <th className="py-1 pr-2 font-normal">Kind</th>
                    <th className="py-1 pr-2 font-normal">Tool</th>
                    <th className="py-1 font-normal">Path</th>
                  </tr>
                </thead>
                <tbody>
                  {entries.map((e) => (
                    <tr
                      key={e.id}
                      className="border-b border-border-soft/40 align-top"
                    >
                      <td className="py-1 pr-2 font-mono text-fg-dim">
                        {formatTs(e.ts)}
                      </td>
                      <td className="py-1 pr-2">
                        <KindBadge kind={e.kind} />
                      </td>
                      <td className="py-1 pr-2 font-mono">{e.tool}</td>
                      <td className="py-1 break-all font-mono">{e.path}</td>
                    </tr>
                  ))}
                </tbody>
              </table>
            )}
          </div>
          <div className="border-t border-border-soft px-4 py-2 text-[11px] text-fg-dim">
            Up to 100 most recent entries. Use Undo / Redo in the
            editor toolbar to step through the chain.
          </div>
        </DialogPopup>
      </Dialog.Portal>
    </Dialog.Root>
  );
}

function KindBadge({ kind }: { kind: string }) {
  // "write" = green-tinted (additive), "undo" = red-tinted
  // (reverting). Matches the diff-preview convention so users see
  // the same colour language across the app.
  if (kind === "undo") {
    return (
      <span className="inline-block rounded-sm bg-danger/10 px-1.5 py-0.5 text-[10px] uppercase tracking-wider text-danger">
        undo
      </span>
    );
  }
  return (
    <span className="inline-block rounded-sm bg-success/10 px-1.5 py-0.5 text-[10px] uppercase tracking-wider text-success">
      {kind}
    </span>
  );
}

function formatTs(ms: number): string {
  // Compact relative-ish format. Today: HH:MM; older: yyyy-MM-dd HH:MM.
  const d = new Date(ms);
  const now = new Date();
  const sameDay =
    d.getFullYear() === now.getFullYear() &&
    d.getMonth() === now.getMonth() &&
    d.getDate() === now.getDate();
  const hh = pad2(d.getHours());
  const mm = pad2(d.getMinutes());
  if (sameDay) return `${hh}:${mm}`;
  const yyyy = d.getFullYear();
  const mo = pad2(d.getMonth() + 1);
  const dd = pad2(d.getDate());
  return `${yyyy}-${mo}-${dd} ${hh}:${mm}`;
}

function pad2(n: number): string {
  return n < 10 ? `0${n}` : `${n}`;
}
