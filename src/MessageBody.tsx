import { useRef, useState } from "react";
import ReactMarkdown from "react-markdown";
import remarkGfm from "remark-gfm";
import remarkMath from "remark-math";
import rehypeKatex from "rehype-katex";
import rehypeHighlight from "rehype-highlight";

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
      if (i % 2 === 1) return part;
      return part
        .replace(/\\\[([\s\S]+?)\\\]/g, (_m, body) => `\n$$${body}$$\n`)
        .replace(/\\\(([\s\S]+?)\\\)/g, (_m, body) => `$${body}$`);
    })
    .join("");
}

function CopyButton({ getText, label = "Copy" }: { getText: () => string; label?: string }) {
  const [copied, setCopied] = useState(false);
  return (
    <button
      type="button"
      className={`copy-btn${copied ? " copied" : ""}`}
      onClick={() => {
        const text = getText();
        if (!text) return;
        navigator.clipboard
          .writeText(text)
          .then(() => {
            setCopied(true);
            setTimeout(() => setCopied(false), 1200);
          })
          .catch(() => {});
      }}
    >
      {copied ? "Copied" : label}
    </button>
  );
}

function CodeBlock({ children }: { children?: React.ReactNode }) {
  const ref = useRef<HTMLPreElement>(null);
  return (
    <div className="code-wrap">
      <CopyButton getText={() => ref.current?.innerText ?? ""} />
      <pre ref={ref}>{children}</pre>
    </div>
  );
}

export function MessageBody({ content }: { content: string }) {
  const prepared = closeOpenFences(normalizeMathDelimiters(content));
  return (
    <ReactMarkdown
      remarkPlugins={[remarkGfm, remarkMath]}
      rehypePlugins={[
        rehypeKatex,
        [rehypeHighlight, { detect: true, ignoreMissing: true }],
      ]}
      components={{ pre: CodeBlock }}
    >
      {prepared}
    </ReactMarkdown>
  );
}

export { CopyButton };
