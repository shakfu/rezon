import { useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { open as openDialog } from "@tauri-apps/plugin-dialog";
import ReactMarkdown from "react-markdown";
import remarkGfm from "remark-gfm";
import remarkMath from "remark-math";
import rehypeKatex from "rehype-katex";
import rehypeHighlight from "rehype-highlight";
import "katex/dist/katex.min.css";
import "highlight.js/styles/github-dark.css";
import "./App.css";

function modelName(path: string): string {
  const base = path.split(/[/\\]/).pop() ?? path;
  return base.replace(/\.gguf$/i, "");
}

function closeOpenFences(src: string): string {
  const fenceCount = (src.match(/```/g) || []).length;
  return fenceCount % 2 === 1 ? src + "\n```" : src;
}

// Convert LaTeX-style \[...\] and \(...\) delimiters to $$...$$ and $...$
// so remark-math picks them up. Skips fenced code blocks so we don't mangle
// snippets that legitimately contain those sequences.
function normalizeMathDelimiters(src: string): string {
  const parts = src.split(/(```[\s\S]*?```)/g);
  return parts
    .map((part, i) => {
      if (i % 2 === 1) return part; // inside a fenced block
      return part
        .replace(/\\\[([\s\S]+?)\\\]/g, (_m, body) => `\n$$${body}$$\n`)
        .replace(/\\\(([\s\S]+?)\\\)/g, (_m, body) => `$${body}$`);
    })
    .join("");
}

function MessageBody({ content }: { content: string }) {
  const prepared = closeOpenFences(normalizeMathDelimiters(content));
  return (
    <ReactMarkdown
      remarkPlugins={[remarkGfm, remarkMath]}
      rehypePlugins={[rehypeKatex, [rehypeHighlight, { detect: true, ignoreMissing: true }]]}
    >
      {prepared}
    </ReactMarkdown>
  );
}

type Role = "system" | "user" | "assistant";
type Msg = { role: Role; content: string };
type ModelStatus = { loaded: boolean; path: string | null };
type CloudProviderInfo = {
  key: string;
  label: string;
  envVar: string;
  defaultModel: string;
  recommendedModels: string[];
  apiKeySet: boolean;
  userConfigurable: boolean;
};

const SYSTEM_PROMPT = "You are a concise, helpful assistant.";

function App() {
  const [messages, setMessages] = useState<Msg[]>([]);
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
  const streamingRef = useRef(false);
  const scrollRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    let cancelled = false;
    let unlistenToken: UnlistenFn | undefined;
    let unlistenDone: UnlistenFn | undefined;
    let unlistenLoading: UnlistenFn | undefined;
    let unlistenLoaded: UnlistenFn | undefined;
    let unlistenLoadErr: UnlistenFn | undefined;

    (async () => {
      const tok = await listen<string>("chat-token", (e) => {
        if (!streamingRef.current) return;
        setMessages((prev) => {
          const next = [...prev];
          const last = next[next.length - 1];
          if (last && last.role === "assistant") {
            next[next.length - 1] = { ...last, content: last.content + e.payload };
          }
          return next;
        });
      });
      const done = await listen("chat-done", () => {
        streamingRef.current = false;
        setStreaming(false);
      });
      const loading = await listen<string>("model-loading", (e) => {
        setModelPath(e.payload);
        setLoading(true);
        setLoadError(null);
      });
      const loaded = await listen<ModelStatus>("model-loaded", (e) => {
        setLoading(false);
        setLoadedPath(e.payload.path ?? null);
        if (e.payload.path) setModelPath(e.payload.path);
      });
      const loadErr = await listen<string>("model-load-error", (e) => {
        setLoading(false);
        setLoadError(e.payload);
      });
      if (cancelled) {
        tok();
        done();
        loading();
        loaded();
        loadErr();
        return;
      }
      unlistenToken = tok;
      unlistenDone = done;
      unlistenLoading = loading;
      unlistenLoaded = loaded;
      unlistenLoadErr = loadErr;
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
            if (!next[p.key]) next[p.key] = p.defaultModel;
          }
          return next;
        });
      } catch {
        /* ignore */
      }
    })();

    return () => {
      cancelled = true;
      unlistenToken?.();
      unlistenDone?.();
      unlistenLoading?.();
      unlistenLoaded?.();
      unlistenLoadErr?.();
    };
  }, []);

  useEffect(() => {
    scrollRef.current?.scrollTo({ top: scrollRef.current.scrollHeight });
  }, [messages]);

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
    const text = input.trim();
    if (!text || streaming) return;
    if (provider === "local" && !loadedPath) return;
    if (activeCloud && !cloudReady) return;
    const history: Msg[] = [...messages, { role: "user", content: text }];
    setMessages([...history, { role: "assistant", content: "" }]);
    setInput("");
    streamingRef.current = true;
    setStreaming(true);

    const payload: Msg[] = [
      { role: "system", content: SYSTEM_PROMPT },
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
      setMessages((prev) => {
        const next = [...prev];
        const last = next[next.length - 1];
        if (last && last.role === "assistant" && last.content === "") {
          next[next.length - 1] = {
            role: "assistant",
            content: `[error] ${String(err)}`,
          };
        }
        return next;
      });
    }
  }

  const sendDisabled =
    streaming ||
    !input.trim() ||
    (provider === "local" && !loadedPath) ||
    (!!activeCloud && !cloudReady);

  const inputDisabled =
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
    <main className="chat">
      <header className="chat-header">
        <span>rezo</span>
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
                  setCloudModel((prev) => ({ ...prev, [activeCloud.key]: v }));
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
      {loadError && <div className="load-error">{loadError}</div>}
      <div className="chat-log" ref={scrollRef}>
        {messages.length === 0 && (
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
        {messages.map((m, i) => (
          <div key={i} className={`msg msg-${m.role}`}>
            <div className="msg-role">{m.role}</div>
            <div className="msg-content">
              {m.content ? (
                <MessageBody content={m.content} />
              ) : streaming && i === messages.length - 1 ? (
                "..."
              ) : (
                ""
              )}
            </div>
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
        <button type="submit" disabled={sendDisabled}>
          Send
        </button>
      </form>
    </main>
  );
}

export default App;
