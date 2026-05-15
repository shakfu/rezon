// [[wikilink]] support for Milkdown.
//
// Three pieces:
//   1. remarkWikilink: a remark plugin that walks the mdast tree and
//      splits text nodes on `[[target|alias]]` matches, replacing the
//      run with mdast nodes of type "wikilink".
//   2. wikilinkSchema: a Milkdown inline NodeSchema bound to that
//      mdast type, with parseMarkdown / toMarkdown round-tripping.
//   3. wikilinkPlugin: the combined plugin export consumed by the
//      editor.

import { $nodeSchema, $remark } from "@milkdown/utils";

// Match [[target]] or [[target|alias]]. No nested brackets.
const WIKILINK_RE = /\[\[([^\[\]\n|]+?)(?:\|([^\[\]\n]+?))?\]\]/g;

interface MdastNode {
  type: string;
  value?: string;
  children?: MdastNode[];
  data?: { target?: string; alias?: string };
  [k: string]: unknown;
}

function transformTextNode(node: MdastNode): MdastNode[] | null {
  const text = node.value ?? "";
  if (!text || !text.includes("[[")) return null;
  const out: MdastNode[] = [];
  let last = 0;
  WIKILINK_RE.lastIndex = 0;
  let m: RegExpExecArray | null;
  while ((m = WIKILINK_RE.exec(text)) !== null) {
    if (m.index > last) {
      out.push({ type: "text", value: text.slice(last, m.index) });
    }
    out.push({
      type: "wikilink",
      data: {
        target: m[1].trim(),
        alias: m[2] ? m[2].trim() : undefined,
      },
    });
    last = m.index + m[0].length;
  }
  if (out.length === 0) return null;
  if (last < text.length) {
    out.push({ type: "text", value: text.slice(last) });
  }
  return out;
}

function walk(node: MdastNode) {
  if (!node.children) return;
  const next: MdastNode[] = [];
  for (const child of node.children) {
    if (child.type === "text") {
      const replaced = transformTextNode(child);
      if (replaced) {
        next.push(...replaced);
        continue;
      }
    } else {
      walk(child);
    }
    next.push(child);
  }
  node.children = next;
}

export const remarkWikilink = $remark("remark-wikilink", () => () => (tree) => {
  walk(tree as unknown as MdastNode);
});

export const wikilinkSchema = $nodeSchema("wikilink", () => ({
  group: "inline",
  inline: true,
  atom: true,
  selectable: true,
  marks: "",
  attrs: {
    target: { default: "" },
    alias: { default: null },
  },
  parseDOM: [
    {
      tag: "a.wikilink",
      getAttrs: (dom) => {
        const el = dom as HTMLElement;
        return {
          target: el.getAttribute("data-target") ?? "",
          alias: el.getAttribute("data-alias") || null,
        };
      },
    },
  ],
  toDOM: (node) => {
    const target = (node.attrs.target as string) ?? "";
    const alias = (node.attrs.alias as string | null) ?? null;
    return [
      "a",
      {
        class: "wikilink",
        href: "#",
        "data-target": target,
        "data-alias": alias ?? "",
        title: `Cmd-click to open [[${target}]]`,
      },
      alias ?? target,
    ];
  },
  parseMarkdown: {
    match: (node) => node.type === "wikilink",
    runner: (state, node, type) => {
      const data = (node.data ?? {}) as { target?: string; alias?: string };
      state.addNode(type, {
        target: data.target ?? "",
        alias: data.alias ?? null,
      });
    },
  },
  toMarkdown: {
    match: (node) => node.type.name === "wikilink",
    runner: (state, node) => {
      const target = (node.attrs.target as string) ?? "";
      const alias = (node.attrs.alias as string | null) ?? null;
      const text = alias ? `[[${target}|${alias}]]` : `[[${target}]]`;
      state.addNode("text", undefined, text);
    },
  },
}));

export const wikilinkPlugin = [wikilinkSchema, remarkWikilink].flat();
