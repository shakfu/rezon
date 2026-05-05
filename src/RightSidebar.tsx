import { Combobox } from "@base-ui/react/combobox";
import { Tooltip } from "@base-ui/react/tooltip";
import { BaseSelect } from "./Select";
import { CloudProviderInfo, Conversation } from "./types";

type Props = {
  collapsed: boolean;
  onToggle: () => void;
  // Provider state (app-global)
  provider: string;
  setProvider: (p: string) => void;
  cloudProviders: CloudProviderInfo[];
  cloudModel: Record<string, string>;
  setCloudModel: (
    fn: (prev: Record<string, string>) => Record<string, string>,
  ) => void;
  cloudBaseUrl: Record<string, string>;
  setCloudBaseUrl: (
    fn: (prev: Record<string, string>) => Record<string, string>,
  ) => void;
  cloudApiKey: Record<string, string>;
  setCloudApiKey: (
    fn: (prev: Record<string, string>) => Record<string, string>,
  ) => void;
  // Local model state
  modelPath: string;
  setModelPath: (s: string) => void;
  loadedPath: string | null;
  loading: boolean;
  onBrowseFile: () => void;
  onLoadModel: () => void;
  // Per-conversation
  current: Conversation | null;
  onSystemPromptChange: (value: string) => void;
};

const INPUT =
  "w-full box-border rounded-md border border-border bg-transparent px-2 py-1.5 text-[13px] text-fg font-[inherit] outline-none focus-visible:ring-2 focus-visible:ring-accent";

const BTN =
  "rounded-md border border-border bg-transparent px-2.5 py-1.5 text-[13px] text-fg cursor-pointer hover:bg-bg-soft disabled:opacity-50 disabled:cursor-not-allowed";

const SIDEBAR_BTN =
  "w-7 h-7 flex items-center justify-center rounded-md border border-border bg-transparent text-fg-dim hover:bg-bg-soft hover:text-fg cursor-pointer text-sm leading-none";

const TOOLTIP_POPUP =
  "tooltip z-[200] select-none rounded bg-fg px-2 py-1 text-[11px] leading-none text-bg";

function SidebarToggle({
  side,
  label,
  onClick,
  children,
}: {
  side: "left" | "right";
  label: string;
  onClick: () => void;
  children: React.ReactNode;
}) {
  return (
    <Tooltip.Root>
      <Tooltip.Trigger className={SIDEBAR_BTN} onClick={onClick}>
        {children}
      </Tooltip.Trigger>
      <Tooltip.Portal>
        <Tooltip.Positioner side={side} sideOffset={6}>
          <Tooltip.Popup className={TOOLTIP_POPUP}>
            {label}
            <Tooltip.Arrow className="fill-fg" />
          </Tooltip.Popup>
        </Tooltip.Positioner>
      </Tooltip.Portal>
    </Tooltip.Root>
  );
}

function ModelCombobox({
  items,
  value,
  onChange,
  placeholder,
}: {
  items: string[];
  value: string;
  onChange: (v: string) => void;
  placeholder?: string;
}) {
  return (
    <Combobox.Root
      items={items}
      inputValue={value}
      onInputValueChange={onChange}
    >
      <div className="relative">
        <Combobox.Input className={`${INPUT} pr-8`} placeholder={placeholder} />
        <Combobox.Trigger
          className="absolute right-1 top-1/2 -translate-y-1/2 flex h-6 w-6 cursor-pointer items-center justify-center rounded text-fg-dim hover:bg-bg-soft hover:text-fg"
          aria-label="Show recommended models"
        >
          ▾
        </Combobox.Trigger>
      </div>
      <Combobox.Portal>
        <Combobox.Positioner sideOffset={4} className="z-[150]">
          <Combobox.Popup className="max-h-64 min-w-[var(--anchor-width)] overflow-y-auto rounded-md border border-border bg-bg-elev py-1 text-[13px] text-fg shadow-[0_10px_30px_rgba(0,0,0,0.3)]">
            <Combobox.Empty className="px-2.5 py-1.5 text-[12px] italic text-fg-dim">
              No matches — press Enter to use as-is.
            </Combobox.Empty>
            <Combobox.List>
              {(item: string) => (
                <Combobox.Item
                  key={item}
                  value={item}
                  className="cursor-pointer px-2.5 py-1.5 hover:bg-bg-soft data-[highlighted]:bg-accent-soft"
                >
                  {item}
                </Combobox.Item>
              )}
            </Combobox.List>
          </Combobox.Popup>
        </Combobox.Positioner>
      </Combobox.Portal>
    </Combobox.Root>
  );
}

export function RightSidebar(props: Props) {
  const {
    collapsed,
    onToggle,
    provider,
    setProvider,
    cloudProviders,
    cloudModel,
    setCloudModel,
    cloudBaseUrl,
    setCloudBaseUrl,
    cloudApiKey,
    setCloudApiKey,
    modelPath,
    setModelPath,
    loading,
    onBrowseFile,
    onLoadModel,
    current,
    onSystemPromptChange,
  } = props;

  const activeCloud = cloudProviders.find((p) => p.key === provider);

  if (collapsed) {
    return (
      <aside className="flex w-10 flex-col items-center border-l border-border-soft bg-bg-elev py-2">
        <SidebarToggle side="left" label="Expand sidebar" onClick={onToggle}>
          «
        </SidebarToggle>
      </aside>
    );
  }

  return (
    <aside className="flex w-72 flex-col overflow-y-auto border-l border-border-soft bg-bg-elev">
      <div className="flex justify-start border-b border-border-soft px-2.5 py-2">
        <SidebarToggle side="left" label="Collapse sidebar" onClick={onToggle}>
          »
        </SidebarToggle>
      </div>

      <Section title="Provider">
        <BaseSelect
          value={provider}
          onValueChange={setProvider}
          items={[
            { value: "local", label: "Local" },
            ...cloudProviders.map((p) => ({ value: p.key, label: p.label })),
          ]}
        />
        {activeCloud && !activeCloud.apiKeySet && (
          <div className="text-[12px] text-danger">
            {activeCloud.envVar} not set
          </div>
        )}
      </Section>

      <Section title="Model">
        {provider === "local" ? (
          <div className="flex flex-col gap-1.5">
            <input
              className={INPUT}
              value={modelPath}
              onChange={(e) => setModelPath(e.currentTarget.value)}
              placeholder="/path/to/model.gguf"
              disabled={loading}
            />
            <div className="flex gap-1.5">
              <button
                type="button"
                className={BTN}
                onClick={onBrowseFile}
                disabled={loading}
              >
                Browse...
              </button>
              <button
                className={`${BTN} flex-1`}
                onClick={onLoadModel}
                disabled={loading || !modelPath.trim()}
              >
                {loading ? "Loading..." : "Load"}
              </button>
            </div>
          </div>
        ) : activeCloud ? (
          activeCloud.userConfigurable ? (
            <div className="flex flex-col gap-1.5">
              <input
                className={INPUT}
                value={cloudModel[activeCloud.key] ?? ""}
                onChange={(e) =>
                  setCloudModel((prev) => ({
                    ...prev,
                    [activeCloud.key]: e.currentTarget.value,
                  }))
                }
                placeholder="model (e.g. llama3.2)"
              />
              <input
                className={INPUT}
                value={cloudBaseUrl[activeCloud.key] ?? ""}
                onChange={(e) =>
                  setCloudBaseUrl((prev) => ({
                    ...prev,
                    [activeCloud.key]: e.currentTarget.value,
                  }))
                }
                placeholder="base URL (e.g. http://localhost:11434/v1)"
              />
              <input
                className={INPUT}
                type="password"
                value={cloudApiKey[activeCloud.key] ?? ""}
                onChange={(e) =>
                  setCloudApiKey((prev) => ({
                    ...prev,
                    [activeCloud.key]: e.currentTarget.value,
                  }))
                }
                placeholder="API key (optional)"
              />
            </div>
          ) : (
            <ModelCombobox
              items={activeCloud.recommendedModels}
              value={cloudModel[activeCloud.key] ?? ""}
              onChange={(v) =>
                setCloudModel((prev) => ({ ...prev, [activeCloud.key]: v }))
              }
              placeholder={activeCloud.defaultModel}
            />
          )
        ) : null}
      </Section>

      <section className="flex flex-1 flex-col gap-2 px-3.5 py-3">
        <h3 className="m-0 text-[11px] font-semibold uppercase tracking-wider text-fg-dim">
          System prompt
        </h3>
        {current ? (
          <textarea
            className={`${INPUT} flex-1 min-h-[140px] resize-y`}
            value={current.systemPrompt}
            onChange={(e) => onSystemPromptChange(e.currentTarget.value)}
            placeholder="Instructions for the assistant for this conversation."
          />
        ) : (
          <div className="text-[12px] italic text-fg-dim">
            No conversation selected.
          </div>
        )}
      </section>
    </aside>
  );
}

function Section({
  title,
  children,
}: {
  title: string;
  children: React.ReactNode;
}) {
  return (
    <section className="flex flex-col gap-2 border-b border-border-soft px-3.5 py-3">
      <h3 className="m-0 text-[11px] font-semibold uppercase tracking-wider text-fg-dim">
        {title}
      </h3>
      {children}
    </section>
  );
}
