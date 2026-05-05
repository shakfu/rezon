import { useEffect, useMemo, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { open as openDialog } from "@tauri-apps/plugin-dialog";
import "katex/dist/katex.min.css";
import "highlight.js/styles/github-dark.css";
import "./App.css";

import {
  CloudProviderInfo,
  Conversation,
  Msg,
  MsgStats,
  ModelStatus,
  Settings,
} from "./types";
import {
  deriveTitle,
  loadConversations,
  loadCurrentId,
  loadSettings,
  newConversation,
  saveConversations,
  saveCurrentId,
  saveSettings,
} from "./storage";
import { MessageBody, CopyButton } from "./MessageBody";
import { Sidebar } from "./Sidebar";
import { SettingsDrawer } from "./SettingsDrawer";

function modelName(path: string): string {
  const base = path.split(/[/\\]/).pop() ?? path;
  return base.replace(/\.gguf$/i, "");
}

function formatStats(s: MsgStats): string {
  const seconds = s.durationMs / 1000;
  const tps = seconds > 0 ? s.genTokens / seconds : 0;
  const parts: string[] = [];
  if (s.promptTokens != null) {
    if (s.cachedTokens != null && s.cachedTokens > 0) {
      parts.push(`${s.promptTokens} prompt (${s.cachedTokens} cached)`);
    } else {
      parts.push(`${s.promptTokens} prompt`);
    }
  }
  parts.push(`${s.genTokens} gen`);
  parts.push(`${tps.toFixed(1)} tok/s`);
  parts.push(`${seconds.toFixed(1)}s`);
  parts.push(s.provider);
  return parts.join(" · ");
}

function App() {
  // ---- Settings ----
  const [settings, setSettings] = useState<Settings>(() => loadSettings());
  const [settingsOpen, setSettingsOpen] = useState(false);

  // ---- Conversations ----
  const [conversations, setConversations] = useState<Conversation[]>(() =>
    loadConversations(),
  );
  const [currentId, setCurrentId] = useState<string | null>(() =>
    loadCurrentId(),
  );

  // Ensure at least one conversation and a valid current selection.
  useEffect(() => {
    if (conversations.length === 0) {
      const c = newConversation(settings.defaultSystemPrompt);
      setConversations([c]);
      setCurrentId(c.id);
    } else if (!currentId || !conversations.some((c) => c.id === currentId)) {
      setCurrentId(conversations[0].id);
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  useEffect(() => {
    saveConversations(conversations);
  }, [conversations]);
  useEffect(() => {
    saveCurrentId(currentId);
  }, [currentId]);
  useEffect(() => {
    saveSettings(settings);
  }, [settings]);

  // Apply theme + font size to root.
  useEffect(() => {
    document.documentElement.dataset.theme = settings.theme;
    document.documentElement.style.fontSize = `${settings.fontSize}px`;
  }, [settings.theme, settings.fontSize]);

  const current = useMemo(
    () => conversations.find((c) => c.id === currentId) ?? null,
    [conversations, currentId],
  );

  function updateConversation(id: string, mut: (c: Conversation) => Conversation) {
    setConversations((prev) =>
      prev.map((c) => (c.id === id ? { ...mut(c), updatedAt: Date.now() } : c)),
    );
  }

  function newChat() {
    const c = newConversation(settings.defaultSystemPrompt);
    setConversations((prev) => [c, ...prev]);
    setCurrentId(c.id);
  }

  function deleteConversation(id: string) {
    setConversations((prev) => prev.filter((c) => c.id !== id));
    if (currentId === id) {
      const remaining = conversations.filter((c) => c.id !== id);
      setCurrentId(remaining.length > 0 ? remaining[0].id : null);
    }
  }

  // ---- Inference state (model + provider) ----
  const [input, setInput] = useState("");
  const [streaming, setStreaming] = useState(false);
  const [modelPath, setModelPath] = useState("");
  const [loadedPath, setLoadedPath] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [provider, setProvider] = useState<string>("local");
  const [cloudProviders, setCloudProviders] = useState<CloudProviderInfo[]>([]);
  const [cloudModel, setCloudModel] = useState<Record<string, string>>({});
  const [cloudBaseUrl, setCloudBaseUrl] = useState<Record<string, string>>({});
  const [cloudApiKey, setCloudApiKey] = useState<Record<string, string>>({});
  const [systemPromptOpen, setSystemPromptOpen] = useState(false);
  const streamingRef = useRef(false);
  const streamingConvIdRef = useRef<string | null>(null);
  const scrollRef = useRef<HTMLDivElement>(null);

  // ---- Tauri event wiring ----
  useEffect(() => {
    let cancelled = false;
    const unlistens: (UnlistenFn | undefined)[] = [];

    (async () => {
      unlistens.push(
        await listen<string>("chat-token", (e) => {
          if (!streamingRef.current) return;
          const cid = streamingConvIdRef.current;
          if (!cid) return;
          setConversations((prev) =>
            prev.map((c) => {
              if (c.id !== cid) return c;
              const msgs = [...c.messages];
              const last = msgs[msgs.length - 1];
              if (last && last.role === "assistant") {
                msgs[msgs.length - 1] = {
                  ...last,
                  content: last.content + e.payload,
                };
              }
              return { ...c, messages: msgs };
            }),
          );
        }),
      );

      unlistens.push(
        await listen<MsgStats>("chat-stats", (e) => {
          const cid = streamingConvIdRef.current;
          if (!cid) return;
          setConversations((prev) =>
            prev.map((c) => {
              if (c.id !== cid) return c;
              const msgs = [...c.messages];
              const last = msgs[msgs.length - 1];
              if (last && last.role === "assistant") {
                msgs[msgs.length - 1] = { ...last, stats: e.payload };
              }
              return { ...c, messages: msgs };
            }),
          );
        }),
      );

      unlistens.push(
        await listen("chat-done", () => {
          streamingRef.current = false;
          setStreaming(false);
        }),
      );

      unlistens.push(
        await listen<string>("model-loading", (e) => {
          setModelPath(e.payload);
          setLoading(true);
          setLoadError(null);
        }),
      );
      unlistens.push(
        await listen<ModelStatus>("model-loaded", (e) => {
          setLoading(false);
          setLoadedPath(e.payload.path ?? null);
          if (e.payload.path) setModelPath(e.payload.path);
        }),
      );
      unlistens.push(
        await listen<string>("model-load-error", (e) => {
          setLoading(false);
          setLoadError(e.payload);
        }),
      );

      if (cancelled) {
        unlistens.forEach((u) => u && u());
        return;
      }

      try {
        const status = await invoke<ModelStatus>("model_status");
        if (status.loaded && status.path) {
          setLoadedPath(status.path);
          setModelPath(status.path);
        }
      } catch {
        /* ignore */
      }
      try {
        const list = await invoke<CloudProviderInfo[]>("cloud_providers");
        setCloudProviders(list);
        setCloudModel((prev) => {
          const next = { ...prev };
          for (const p of list) {
            if (!next[p.key] && p.defaultModel) next[p.key] = p.defaultModel;
          }
          return next;
        });
      } catch {
        /* ignore */
      }
    })();

    return () => {
      cancelled = true;
      unlistens.forEach((u) => u && u());
    };
  }, []);

  // Auto-scroll on new content for the current conversation.
  useEffect(() => {
    scrollRef.current?.scrollTo({ top: scrollRef.current.scrollHeight });
  }, [current?.messages]);

  async function browse(kind: "file" | "directory") {
    const selected = await openDialog({
      multiple: false,
      directory: kind === "directory",
      filters:
        kind === "file"
          ? [{ name: "GGUF model", extensions: ["gguf"] }]
          : undefined,
    });
    if (typeof selected === "string") setModelPath(selected);
  }

  async function loadModel() {
    const path = modelPath.trim();
    if (!path || loading) return;
    setLoading(true);
    setLoadError(null);
    try {
      const status = await invoke<ModelStatus>("load_model", { path });
      setLoadedPath(status.path ?? null);
    } catch (err) {
      setLoadError(String(err));
    } finally {
      setLoading(false);
    }
  }

  // ---- Provider state derivations ----
  const activeCloud = cloudProviders.find((p) => p.key === provider);
  const activeCloudModel = activeCloud
    ? (cloudModel[activeCloud.key] ?? "").trim()
    : "";
  const activeBaseUrl = activeCloud
    ? (cloudBaseUrl[activeCloud.key] ?? "").trim()
    : "";
  const activeApiKey = activeCloud ? (cloudApiKey[activeCloud.key] ?? "") : "";
  const cloudReady = activeCloud
    ? !!activeCloudModel &&
      (!activeCloud.userConfigurable || !!activeBaseUrl)
    : false;

  async function send() {
    if (!current) return;
    const text = input.trim();
    if (!text || streaming) return;
    if (provider === "local" && !loadedPath) return;
    if (activeCloud && !cloudReady) return;

    const userMsg: Msg = { role: "user", content: text };
    const newAssistant: Msg = { role: "assistant", content: "" };
    const history = [...current.messages, userMsg];
    const titled = current.messages.length === 0;
    updateConversation(current.id, (c) => ({
      ...c,
      title: titled ? deriveTitle(text) : c.title,
      messages: [...history, newAssistant],
    }));
    setInput("");
    streamingRef.current = true;
    streamingConvIdRef.current = current.id;
    setStreaming(true);

    const payload: Msg[] = [
      { role: "system", content: current.systemPrompt },
      ...history,
    ];

    const opts = activeCloud
      ? activeCloud.userConfigurable
        ? {
            provider: activeCloud.key,
            model: activeCloudModel,
            baseUrl: activeBaseUrl,
            apiKey: activeApiKey,
          }
        : { provider: activeCloud.key, model: activeCloudModel }
      : { provider: "local" };

    try {
      await invoke<string>("chat", { messages: payload, opts });
    } catch (err) {
      streamingRef.current = false;
      setStreaming(false);
      const errText = String(err);
      const cid = current.id;
      setConversations((prev) =>
        prev.map((c) => {
          if (c.id !== cid) return c;
          const msgs = [...c.messages];
          const last = msgs[msgs.length - 1];
          if (last && last.role === "assistant" && last.content === "") {
            msgs[msgs.length - 1] = {
              role: "assistant",
              content: errText,
              isError: true,
            };
          } else {
            msgs.push({ role: "assistant", content: errText, isError: true });
          }
          return { ...c, messages: msgs };
        }),
      );
    }
  }

  const sendDisabled =
    streaming ||
    !input.trim() ||
    !current ||
    (provider === "local" && !loadedPath) ||
    (!!activeCloud && !cloudReady);

  const inputDisabled =
    !current ||
    streaming ||
    (provider === "local" && !loadedPath) ||
    (!!activeCloud && !cloudReady);

  const placeholder = streaming
    ? "Generating..."
    : provider === "local" && !loadedPath
      ? "Load a model first..."
      : activeCloud && !activeCloudModel
        ? `Set a ${activeCloud.label} model name...`
        : activeCloud && activeCloud.userConfigurable && !activeBaseUrl
          ? "Set a base URL..."
          : "Send a message (Enter to send, Shift+Enter for newline)";

  const headerLabel = activeCloud
    ? `${activeCloud.key}: ${activeCloudModel || "?"}`
    : loadedPath
      ? modelName(loadedPath)
      : "no model loaded";

  return (
    <div className="app">
      <Sidebar
        conversations={conversations}
        currentId={currentId}
        onSelect={setCurrentId}
        onNew={newChat}
        onRename={(id, title) => updateConversation(id, (c) => ({ ...c, title }))}
        onDelete={deleteConversation}
        onOpenSettings={() => setSettingsOpen(true)}
      />
      <main className="chat">
        <header className="chat-header">
          <span className="chat-title" title={current?.title}>
            {current?.title ?? "rezo"}
          </span>
          <span className="model-state" title={loadedPath ?? undefined}>
            {headerLabel}
          </span>
        </header>

        <div className="provider-row">
          <label>
            <input
              type="radio"
              name="provider"
              value="local"
              checked={provider === "local"}
              onChange={() => setProvider("local")}
            />
            Local
          </label>
          {cloudProviders.map((p) => (
            <label key={p.key}>
              <input
                type="radio"
                name="provider"
                value={p.key}
                checked={provider === p.key}
                onChange={() => setProvider(p.key)}
              />
              {p.label}
            </label>
          ))}
          {activeCloud && !activeCloud.apiKeySet && (
            <span className="provider-warn">{activeCloud.envVar} not set</span>
          )}
        </div>

        {provider === "local" ? (
          <div className="model-row">
            <input
              value={modelPath}
              onChange={(e) => setModelPath(e.currentTarget.value)}
              placeholder="/path/to/model.gguf"
              disabled={loading}
            />
            <button
              type="button"
              onClick={() => browse("file")}
              disabled={loading}
              title="Pick a .gguf file"
            >
              Browse...
            </button>
            <button onClick={loadModel} disabled={loading || !modelPath.trim()}>
              {loading ? "Loading..." : "Load"}
            </button>
          </div>
        ) : activeCloud ? (
          activeCloud.userConfigurable ? (
            <div className="model-row model-row-stack">
              <input
                value={cloudModel[activeCloud.key] ?? ""}
                onChange={(e) =>
                  setCloudModel((prev) => ({
                    ...prev,
                    [activeCloud.key]: e.currentTarget.value,
                  }))
                }
                placeholder="model (e.g. llama3.2, qwen2.5-coder:7b)"
              />
              <input
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
            <div className="model-row">
              <select
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

        {loadError && (
          <div className="banner banner-error" role="alert">
            <span className="banner-text">{loadError}</span>
            <button
              className="banner-dismiss"
              onClick={() => setLoadError(null)}
              aria-label="Dismiss"
            >
              ×
            </button>
          </div>
        )}

        {current && (
          <div className="system-prompt-row">
            <button
              className="system-prompt-toggle"
              onClick={() => setSystemPromptOpen((v) => !v)}
            >
              {systemPromptOpen ? "▾" : "▸"} System prompt
            </button>
            {!systemPromptOpen && (
              <span className="system-prompt-preview" title={current.systemPrompt}>
                {current.systemPrompt || "(empty)"}
              </span>
            )}
            {systemPromptOpen && (
              <textarea
                className="system-prompt-edit"
                rows={3}
                value={current.systemPrompt}
                onChange={(e) =>
                  updateConversation(current.id, (c) => ({
                    ...c,
                    systemPrompt: e.currentTarget.value,
                  }))
                }
                placeholder="Instructions for the assistant for this conversation."
              />
            )}
          </div>
        )}

        <div className="chat-log" ref={scrollRef}>
          {current && current.messages.length === 0 && (
            <div className="chat-empty">
              {activeCloud
                ? activeCloudModel
                  ? "Send a message to begin."
                  : `Enter a ${activeCloud.label} model name to begin.`
                : loadedPath
                  ? "Send a message to begin."
                  : "Load a model to begin. Point at a .gguf file."}
            </div>
          )}
          {current?.messages.map((m, i) => (
            <div
              key={i}
              className={`msg msg-${m.role}${m.isError ? " msg-error" : ""}`}
            >
              <div className="msg-role">
                <span>{m.role}</span>
                {m.content && (
                  <CopyButton getText={() => m.content} label="Copy" />
                )}
              </div>
              <div className="msg-content">
                {m.content ? (
                  m.isError ? (
                    <pre className="error-text">{m.content}</pre>
                  ) : (
                    <MessageBody content={m.content} />
                  )
                ) : streaming && i === current.messages.length - 1 ? (
                  <span className="dot-pulse">…</span>
                ) : (
                  ""
                )}
              </div>
              {m.stats && (
                <div className="msg-stats">{formatStats(m.stats)}</div>
              )}
            </div>
          ))}
        </div>

        <form
          className="chat-input"
          onSubmit={(e) => {
            e.preventDefault();
            send();
          }}
        >
          <textarea
            value={input}
            onChange={(e) => setInput(e.currentTarget.value)}
            onKeyDown={(e) => {
              if (e.key === "Enter" && !e.shiftKey) {
                e.preventDefault();
                send();
              }
            }}
            placeholder={placeholder}
            rows={2}
            disabled={inputDisabled}
          />
          {streaming ? (
            <button
              type="button"
              className="stop"
              onClick={() => {
                invoke("cancel_chat").catch(() => {});
              }}
            >
              Stop
            </button>
          ) : (
            <button type="submit" disabled={sendDisabled}>
              Send
            </button>
          )}
        </form>
      </main>
      <SettingsDrawer
        open={settingsOpen}
        settings={settings}
        onChange={setSettings}
        onClose={() => setSettingsOpen(false)}
      />
    </div>
  );
}

export default App;
