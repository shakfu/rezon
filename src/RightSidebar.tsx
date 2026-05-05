import * as Tooltip from "@radix-ui/react-tooltip";
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
  const activeCloudModel = activeCloud
    ? (cloudModel[activeCloud.key] ?? "").trim()
    : "";

  if (collapsed) {
    return (
      <aside className="right-sidebar right-sidebar-collapsed">
        <Tooltip.Root>
          <Tooltip.Trigger asChild>
            <button className="sidebar-toggle" onClick={onToggle}>
              «
            </button>
          </Tooltip.Trigger>
          <Tooltip.Portal>
            <Tooltip.Content className="tooltip" side="left" sideOffset={6}>
              Expand sidebar
              <Tooltip.Arrow className="tooltip-arrow" />
            </Tooltip.Content>
          </Tooltip.Portal>
        </Tooltip.Root>
      </aside>
    );
  }

  return (
    <aside className="right-sidebar">
      <div className="rs-top">
        <Tooltip.Root>
          <Tooltip.Trigger asChild>
            <button className="sidebar-toggle" onClick={onToggle}>
              »
            </button>
          </Tooltip.Trigger>
          <Tooltip.Portal>
            <Tooltip.Content className="tooltip" side="left" sideOffset={6}>
              Collapse sidebar
              <Tooltip.Arrow className="tooltip-arrow" />
            </Tooltip.Content>
          </Tooltip.Portal>
        </Tooltip.Root>
      </div>
      <section className="rs-section">
        <h3 className="rs-heading">Provider</h3>
        <select
          className="rs-input"
          value={provider}
          onChange={(e) => setProvider(e.currentTarget.value)}
        >
          <option value="local">Local</option>
          {cloudProviders.map((p) => (
            <option key={p.key} value={p.key}>
              {p.label}
            </option>
          ))}
        </select>
        {activeCloud && !activeCloud.apiKeySet && (
          <div className="rs-warn">{activeCloud.envVar} not set</div>
        )}
      </section>

      <section className="rs-section">
        <h3 className="rs-heading">Model</h3>
        {provider === "local" ? (
          <div className="rs-stack">
            <input
              className="rs-input"
              value={modelPath}
              onChange={(e) => setModelPath(e.currentTarget.value)}
              placeholder="/path/to/model.gguf"
              disabled={loading}
            />
            <div className="rs-row">
              <button
                type="button"
                className="rs-btn"
                onClick={onBrowseFile}
                disabled={loading}
              >
                Browse...
              </button>
              <button
                className="rs-btn rs-btn-primary"
                onClick={onLoadModel}
                disabled={loading || !modelPath.trim()}
              >
                {loading ? "Loading..." : "Load"}
              </button>
            </div>
          </div>
        ) : activeCloud ? (
          activeCloud.userConfigurable ? (
            <div className="rs-stack">
              <input
                className="rs-input"
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
                className="rs-input"
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
                className="rs-input"
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
            <div className="rs-stack">
              <select
                className="rs-input"
                value={
                  activeCloud.recommendedModels.includes(activeCloudModel)
                    ? activeCloudModel
                    : "__custom__"
                }
                onChange={(e) => {
                  const v = e.currentTarget.value;
                  if (v !== "__custom__") {
                    setCloudModel((prev) => ({
                      ...prev,
                      [activeCloud.key]: v,
                    }));
                  }
                }}
              >
                {activeCloud.recommendedModels.map((m) => (
                  <option key={m} value={m}>
                    {m}
                  </option>
                ))}
                <option value="__custom__">Custom...</option>
              </select>
              <input
                className="rs-input"
                value={cloudModel[activeCloud.key] ?? ""}
                onChange={(e) =>
                  setCloudModel((prev) => ({
                    ...prev,
                    [activeCloud.key]: e.currentTarget.value,
                  }))
                }
                placeholder={activeCloud.defaultModel}
              />
            </div>
          )
        ) : null}
      </section>

      <section className="rs-section rs-section-grow">
        <h3 className="rs-heading">System prompt</h3>
        {current ? (
          <textarea
            className="rs-input rs-system-prompt"
            value={current.systemPrompt}
            onChange={(e) => onSystemPromptChange(e.currentTarget.value)}
            placeholder="Instructions for the assistant for this conversation."
          />
        ) : (
          <div className="rs-empty">No conversation selected.</div>
        )}
      </section>
    </aside>
  );
}
