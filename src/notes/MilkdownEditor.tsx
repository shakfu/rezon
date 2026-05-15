import { useEffect, useRef } from "react";
import { Editor, rootCtx, defaultValueCtx } from "@milkdown/core";
import { Milkdown, MilkdownProvider, useEditor } from "@milkdown/react";
import { commonmark } from "@milkdown/preset-commonmark";
import { gfm } from "@milkdown/preset-gfm";
import { listener, listenerCtx } from "@milkdown/plugin-listener";
import { wikilinkPlugin } from "./wikilink";

type Props = {
  initialMarkdown: string;
  onChange: (markdown: string) => void;
  onWikilinkClick: (target: string) => void;
};

function EditorInner({ initialMarkdown, onChange, onWikilinkClick }: Props) {
  const changeRef = useRef(onChange);
  const clickRef = useRef(onWikilinkClick);
  changeRef.current = onChange;
  clickRef.current = onWikilinkClick;

  useEditor(
    (root) =>
      Editor.make()
        .config((ctx) => {
          ctx.set(rootCtx, root);
          ctx.set(defaultValueCtx, initialMarkdown);
          ctx.get(listenerCtx).markdownUpdated((_ctx, markdown) => {
            changeRef.current(markdown);
          });
        })
        .use(commonmark)
        .use(gfm)
        .use(listener)
        .use(wikilinkPlugin),
    // Recreate the editor when the open file changes. The parent
    // keys the wrapping element by path, so this dep array is
    // effectively a single-shot init per mount.
    [],
  );

  // Cmd/Ctrl-click on wikilinks in the rendered output. ProseMirror
  // owns the DOM under the .milkdown root, but ordinary click events
  // still bubble; we attach a capturing listener so we run before
  // ProseMirror's own handling.
  useEffect(() => {
    function onClick(ev: MouseEvent) {
      if (!(ev.metaKey || ev.ctrlKey)) return;
      const target = ev.target as HTMLElement | null;
      const link = target?.closest("a.wikilink") as HTMLElement | null;
      if (!link) return;
      const t = link.getAttribute("data-target");
      if (!t) return;
      ev.preventDefault();
      ev.stopPropagation();
      clickRef.current(t);
    }
    document.addEventListener("click", onClick, true);
    return () => document.removeEventListener("click", onClick, true);
  }, []);

  return <Milkdown />;
}

export function MilkdownEditor(props: Props) {
  return (
    <MilkdownProvider>
      <EditorInner {...props} />
    </MilkdownProvider>
  );
}
