/**
 * PromptPopover
 *
 * Shown when the user clicks a prompt pill above the chat composer.
 * Allows viewing and editing the prompt file content in-place.
 *
 * Positioning: `absolute bottom-full left-0 right-0 z-50` so it floats
 * above the pill row (consistent with SlashPicker placement).
 */
import { useCallback, useEffect, useRef, useState } from "react";
import { WysiwygMarkdown } from "@/components/WysiwygMarkdown";

export interface PromptPill {
  id: string;
  /** Filename base, e.g. "my-instructions" (no .prompt.md suffix) */
  label: string;
  content: string;
}

interface Props {
  prompt: PromptPill;
  /** Called when the user clicks × or outside. */
  onClose: () => void;
  /**
   * Called with the new content when the user saves.
   * When omitted the edit button is hidden (read-only mode).
   */
  onSave?: (label: string, content: string) => Promise<void>;
}

type Mode = "view" | "edit" | "saving";

export function PromptPopover({ prompt, onClose, onSave }: Props) {
  const [mode, setMode] = useState<Mode>("view");
  const [draft, setDraft] = useState(prompt.content);
  const [error, setError] = useState<string | null>(null);
  const containerRef = useRef<HTMLDivElement>(null);

  // Close on click-outside
  useEffect(() => {
    function handleMouseDown(e: MouseEvent) {
      if (containerRef.current && !containerRef.current.contains(e.target as Node)) {
        onClose();
      }
    }
    document.addEventListener("mousedown", handleMouseDown);
    return () => document.removeEventListener("mousedown", handleMouseDown);
  }, [onClose]);

  // Close on Escape
  useEffect(() => {
    function handleKeyDown(e: KeyboardEvent) {
      if (e.key === "Escape") {
        onClose();
      }
    }
    document.addEventListener("keydown", handleKeyDown);
    return () => document.removeEventListener("keydown", handleKeyDown);
  }, [onClose]);

  const handleEdit = useCallback(() => {
    setDraft(prompt.content);
    setError(null);
    setMode("edit");
  }, [prompt.content]);

  const handleCancel = useCallback(() => {
    setDraft(prompt.content);
    setError(null);
    setMode("view");
  }, [prompt.content]);

  const handleSave = useCallback(async () => {
    if (!onSave) return;
    setMode("saving");
    setError(null);
    try {
      await onSave(prompt.label, draft);
      setMode("view");
    } catch (err) {
      setError((err as Error).message ?? "Save failed");
      setMode("edit");
    }
  }, [onSave, prompt.label, draft]);

  const isEditing = mode === "edit" || mode === "saving";

  return (
    <div
      ref={containerRef}
      className="absolute bottom-full left-0 right-0 z-50 mb-1 rounded-lg border border-border bg-card shadow-lg overflow-hidden"
    >
      {/* Header */}
      <div className="flex items-center justify-between border-b border-border px-2.5 py-1.5">
        <span className="font-mono text-xs text-muted-foreground">
          <span className="opacity-50">/</span>
          <span className="text-foreground">{prompt.label}</span>
          <span className="ml-1 opacity-50 text-xs">.prompt.md</span>
        </span>
        <div className="flex items-center gap-1">
          {!isEditing && onSave && (
            <button
              type="button"
              title="Edit"
              onClick={handleEdit}
              className="rounded px-1.5 py-0.5 text-xs text-muted-foreground hover:bg-border/40 hover:text-foreground transition"
            >
              ✎
            </button>
          )}
          <button
            type="button"
            title="Close"
            onClick={onClose}
            className="rounded px-1.5 py-0.5 text-xs text-muted-foreground hover:bg-border/40 hover:text-foreground transition"
          >
            ×
          </button>
        </div>
      </div>

      {/* Content */}
      <div className="max-h-[360px] overflow-y-auto px-2.5 py-2">
        <WysiwygMarkdown
          value={isEditing ? draft : prompt.content}
          onChange={isEditing ? setDraft : undefined}
          readOnly={!isEditing}
          className="min-h-[80px] text-[12px]"
        />
      </div>

      {/* Footer (edit mode only) */}
      {isEditing && (
        <div className="border-t border-border px-2.5 py-1.5 flex items-center justify-between gap-2">
          {error ? <span className="text-xs text-red-400 flex-1">{error}</span> : <span />}
          <div className="flex items-center gap-1.5">
            <button
              type="button"
              onClick={handleCancel}
              disabled={mode === "saving"}
              className="rounded border border-border px-2 py-0.5 text-xs text-muted-foreground hover:bg-border/30 disabled:opacity-50 transition"
            >
              Cancel
            </button>
            <button
              type="button"
              onClick={() => void handleSave()}
              disabled={mode === "saving"}
              className="rounded bg-primary px-2 py-0.5 text-xs font-medium text-white hover:opacity-90 disabled:opacity-50 transition"
            >
              {mode === "saving" ? "Saving…" : "Save to file"}
            </button>
          </div>
        </div>
      )}
    </div>
  );
}
