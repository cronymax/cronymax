/**
 * Milkdown-based WYSIWYG editor — loaded lazily by WysiwygMarkdown.
 * This module carries the full Milkdown bundle, so it's only fetched
 * when the editor is first rendered in edit mode.
 */
import { useCallback, useEffect, useRef, useState } from "react";
import { Editor, rootCtx, defaultValueCtx } from "@milkdown/core";
import { listener, listenerCtx } from "@milkdown/plugin-listener";
import { commonmark } from "@milkdown/preset-commonmark";
import { Milkdown, MilkdownProvider, useEditor } from "@milkdown/react";

function MilkdownInner({
  initialValue,
  onEmit,
}: {
  initialValue: string;
  onEmit: (v: string) => void;
}) {
  const onEmitRef = useRef(onEmit);
  useEffect(() => {
    onEmitRef.current = onEmit;
  });

  useEditor((root) =>
    Editor.make()
      .config((ctx) => {
        ctx.set(rootCtx, root);
        ctx.set(defaultValueCtx, initialValue);
        ctx.get(listenerCtx).markdownUpdated((_, markdown) => {
          onEmitRef.current(markdown);
        });
      })
      .use(commonmark)
      .use(listener),
  );

  return <Milkdown />;
}

interface Props {
  value: string;
  onChange: (v: string) => void;
}

export default function MilkdownEditor({ value, onChange }: Props) {
  const lastEmitted = useRef<string>(value);
  const [editorKey, setEditorKey] = useState(0);

  useEffect(() => {
    if (value !== lastEmitted.current) {
      lastEmitted.current = value;
      setEditorKey((k) => k + 1);
    }
  }, [value]);

  const handleEmit = useCallback(
    (md: string) => {
      lastEmitted.current = md;
      onChange(md);
    },
    [onChange],
  );

  return (
    <div className="cronymax-wysiwyg rounded border border-cronymax-border bg-cronymax-base overflow-auto">
      <MilkdownProvider key={editorKey}>
        <MilkdownInner initialValue={value} onEmit={handleEmit} />
      </MilkdownProvider>
    </div>
  );
}
