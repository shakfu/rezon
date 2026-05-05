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

const SYSTEM_PROMPT = "You are a concise, helpful assistant.";

function App() {
  const [messages, setMessages] = useState<Msg[]>([]);
  const [input, setInput] = useState("");
  const [streaming, setStreaming] = useState(false);
  const [modelPath, setModelPath] = useState("");
  const [loadedPath, setLoadedPath] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);
  const [loadError, setLoadError] = useState<string | null>(null);
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

  async function send() {
    const text = input.trim();
    if (!text || streaming || !loadedPath) return;
    const history: Msg[] = [...messages, { role: "user", content: text }];
    setMessages([...history, { role: "assistant", content: "" }]);
    setInput("");
    streamingRef.current = true;
    setStreaming(true);

    const payload: Msg[] = [
      { role: "system", content: SYSTEM_PROMPT },
      ...history,
    ];

    try {
      await invoke<string>("chat", { messages: payload });
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

  const sendDisabled = streaming || !input.trim() || !loadedPath;

  return (
    <main className="chat">
      <header className="chat-header">
        <span>rezo</span>
        <span className="model-state" title={loadedPath ?? undefined}>
          {loadedPath ? modelName(loadedPath) : "no model loaded"}
        </span>
      </header>
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
      {loadError && <div className="load-error">{loadError}</div>}
      <div className="chat-log" ref={scrollRef}>
        {messages.length === 0 && (
          <div className="chat-empty">
            {loadedPath
              ? "Send a message to begin."
              : "Load a model to begin. Point at a Hugging Face-format directory or a .gguf file."}
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
          placeholder={
            !loadedPath
              ? "Load a model first..."
              : streaming
                ? "Generating..."
                : "Send a message (Enter to send, Shift+Enter for newline)"
          }
          rows={2}
          disabled={streaming || !loadedPath}
        />
        <button type="submit" disabled={sendDisabled}>
          Send
        </button>
      </form>
    </main>
  );
}

export default App;
