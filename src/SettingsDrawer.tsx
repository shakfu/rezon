import { Dialog } from "@base-ui/react/dialog";
import { BaseSelect } from "./Select";
import { Settings, Theme } from "./types";

type Props = {
  open: boolean;
  settings: Settings;
  onChange: (s: Settings) => void;
  onClose: () => void;
};

const INPUT =
  "rounded-md border border-border bg-transparent px-2 py-1 text-[13px] text-fg font-[inherit] outline-none focus-visible:ring-2 focus-visible:ring-accent";

export function SettingsDrawer({ open, settings, onChange, onClose }: Props) {
  return (
    <Dialog.Root open={open} onOpenChange={(v) => !v && onClose()}>
      <Dialog.Portal>
        <Dialog.Backdrop className="dialog-overlay fixed inset-0 z-[100] bg-black/40" />
        <Dialog.Popup className="dialog fixed top-1/2 left-1/2 z-[101] flex w-[420px] max-w-[90vw] max-h-[80vh] -translate-x-1/2 -translate-y-1/2 flex-col rounded-lg border border-border bg-bg-elev text-fg shadow-[0_20px_60px_rgba(0,0,0,0.4)]">
          <div className="flex items-center justify-between border-b border-border-soft px-4 py-3">
            <Dialog.Title className="m-0 text-[1.05em] font-semibold">
              Settings
            </Dialog.Title>
            <Dialog.Close
              className="cursor-pointer border-none bg-transparent px-1 text-[22px] leading-none text-fg"
              aria-label="Close settings"
            >
              ×
            </Dialog.Close>
          </div>
          <Dialog.Description className="sr-only-x">
            Theme, font size, and default system prompt for new conversations.
          </Dialog.Description>
          <div className="flex flex-col gap-3.5 overflow-y-auto px-4 py-3.5">
            <div className="flex items-center justify-between gap-3 text-[13px]">
              <span>Theme</span>
              <BaseSelect
                className="w-[140px]"
                value={settings.theme}
                onValueChange={(v) =>
                  onChange({ ...settings, theme: v as Theme })
                }
                items={[
                  { value: "system", label: "System" },
                  { value: "light", label: "Light" },
                  { value: "dark", label: "Dark" },
                ]}
              />
            </div>
            <label className="flex items-center justify-between gap-3 text-[13px]">
              <span>Font size ({settings.fontSize}px)</span>
              <input
                type="range"
                className="w-40"
                min={12}
                max={20}
                step={1}
                value={settings.fontSize}
                onChange={(e) =>
                  onChange({
                    ...settings,
                    fontSize: Number(e.currentTarget.value),
                  })
                }
              />
            </label>
            <label className="flex flex-col gap-1.5 text-[13px]">
              <span>Default system prompt</span>
              <textarea
                rows={4}
                className={`${INPUT} resize-y`}
                value={settings.defaultSystemPrompt}
                onChange={(e) =>
                  onChange({
                    ...settings,
                    defaultSystemPrompt: e.currentTarget.value,
                  })
                }
              />
              <small className="text-[11px] text-fg-dim">
                Used when starting a new conversation. Existing conversations
                keep their own prompt.
              </small>
            </label>
          </div>
        </Dialog.Popup>
      </Dialog.Portal>
    </Dialog.Root>
  );
}
