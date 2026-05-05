import * as Dialog from "@radix-ui/react-dialog";
import { Settings, Theme } from "./types";

type Props = {
  open: boolean;
  settings: Settings;
  onChange: (s: Settings) => void;
  onClose: () => void;
};

export function SettingsDrawer({ open, settings, onChange, onClose }: Props) {
  return (
    <Dialog.Root open={open} onOpenChange={(v) => !v && onClose()}>
      <Dialog.Portal>
        <Dialog.Overlay className="dialog-overlay" />
        <Dialog.Content className="dialog dialog-settings">
          <div className="dialog-header">
            <Dialog.Title className="dialog-title">Settings</Dialog.Title>
            <Dialog.Close
              className="dialog-close"
              aria-label="Close settings"
            >
              ×
            </Dialog.Close>
          </div>
          <Dialog.Description className="visually-hidden">
            Theme, font size, and default system prompt for new conversations.
          </Dialog.Description>
          <div className="dialog-body">
            <label className="setting">
              <span>Theme</span>
              <select
                value={settings.theme}
                onChange={(e) =>
                  onChange({
                    ...settings,
                    theme: e.currentTarget.value as Theme,
                  })
                }
              >
                <option value="system">System</option>
                <option value="light">Light</option>
                <option value="dark">Dark</option>
              </select>
            </label>
            <label className="setting">
              <span>Font size ({settings.fontSize}px)</span>
              <input
                type="range"
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
            <label className="setting setting-block">
              <span>Default system prompt</span>
              <textarea
                rows={4}
                value={settings.defaultSystemPrompt}
                onChange={(e) =>
                  onChange({
                    ...settings,
                    defaultSystemPrompt: e.currentTarget.value,
                  })
                }
              />
              <small>
                Used when starting a new conversation. Existing conversations
                keep their own prompt.
              </small>
            </label>
          </div>
        </Dialog.Content>
      </Dialog.Portal>
    </Dialog.Root>
  );
}
