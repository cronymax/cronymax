/**
 * Shared WysiwygMarkdown component.
 *
 * - `readOnly={true}` (or no `onChange`): renders via Streamdown (lightweight
 *   read-only markdown, consistent with chat message rendering).
 * - `readOnly={false}` + `onChange`: renders the Milkdown WYSIWYG editor.
 *   Milkdown is lazily loaded so it doesn't enter the initial bundle of entry
 *   points that only need read-only rendering (e.g. the chat panel).
 */
import { lazy, Suspense } from "react";
import { Streamdown } from "streamdown";

// Lazy-load the editable inner editor so Milkdown stays out of the initial
// bundle for entry points that only use read-only rendering.
const MilkdownEditor = lazy(() => import("./MilkdownEditor"));

interface Props {
  value: string;
  onChange?: (v: string) => void;
  /** When true (or when onChange is absent), render read-only via Streamdown. */
  readOnly?: boolean;
  className?: string;
}

export function WysiwygMarkdown({ value, onChange, readOnly, className }: Props) {
  const isReadOnly = readOnly !== false || !onChange;

  if (isReadOnly) {
    return (
      <div className={className}>
        <Streamdown>{value}</Streamdown>
      </div>
    );
  }

  return (
    <div className={className}>
      <Suspense
        fallback={<div className="text-[11px] text-cronymax-caption opacity-60 px-2 py-1">Loading editor…</div>}
      >
        <MilkdownEditor value={value} onChange={onChange!} />
      </Suspense>
    </div>
  );
}
