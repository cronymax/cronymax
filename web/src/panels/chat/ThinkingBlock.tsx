import { useEffect, useRef, useState } from "react";
import { Streamdown } from "streamdown";

/** Maximum characters of thinking content rendered to avoid layout thrash. */
const MAX_THINKING_CHARS = 4096;

interface ThinkingBlockProps {
  /** Accumulated thinking text. */
  thinkingText: string;
  /** True once the first text token has arrived, sealing the thinking phase. */
  sealed: boolean;
  /** Elapsed milliseconds from first thinking token to first text token. */
  elapsedMs: number;
}

/**
 * Renders the thinking/reasoning block produced by extended-thinking models.
 *
 * While streaming (`sealed === false`):  shows an animated "Thinking…" indicator.
 * Once sealed (`sealed === true`):       shows a collapsible `<details>` element
 *   with a "Thinking (N.Ns)" summary header and the accumulated thinking text
 *   inside, rendered via `<Streamdown>`.
 */
export function ThinkingBlock({
  thinkingText,
  sealed,
  elapsedMs,
}: ThinkingBlockProps) {
  const [expanded, setExpanded] = useState(false);
  const detailsRef = useRef<HTMLDetailsElement>(null);

  // Auto-collapse when sealed.
  useEffect(() => {
    if (sealed && detailsRef.current) {
      detailsRef.current.open = false;
      setExpanded(false);
    }
  }, [sealed]);

  // While streaming (not yet sealed): always show the animated dots indicator,
  // even before any thinking text has arrived.
  if (!sealed) {
    return (
      <div className="mb-2 flex items-center gap-1.5 text-xs text-cronymax-caption italic select-none">
        <span>Thinking</span>
        <ThinkingDots />
      </div>
    );
  }

  // Sealed but no content: nothing to show.
  if (!thinkingText) return null;

  const truncated = thinkingText.length > MAX_THINKING_CHARS;
  const displayText = truncated
    ? thinkingText.slice(0, MAX_THINKING_CHARS) + "\n\n*… (truncated)*"
    : thinkingText;
  const elapsedSec = (elapsedMs / 1000).toFixed(1);

  return (
    <details
      ref={detailsRef}
      className="mb-2 rounded-md border border-cronymax-border bg-cronymax-bg-secondary overflow-hidden"
      onToggle={(e) => setExpanded((e.target as HTMLDetailsElement).open)}
    >
      <summary className="flex cursor-pointer list-none items-center justify-between px-3 py-2 text-xs font-medium text-cronymax-caption hover:text-cronymax-title select-none">
        <span>Thinking ({elapsedSec}s)</span>
        <span
          className="ml-2 text-[10px] transition-transform duration-200"
          style={{ transform: expanded ? "rotate(180deg)" : "rotate(0deg)" }}
          aria-hidden
        >
          ▾
        </span>
      </summary>
      {expanded && (
        <div className="border-t border-cronymax-border px-3 py-2 text-xs text-cronymax-caption">
          <Streamdown animated={false} isAnimating={false}>
            {displayText}
          </Streamdown>
        </div>
      )}
    </details>
  );
}

/** Animated "..." indicator for the streaming thinking state. */
function ThinkingDots() {
  const [frame, setFrame] = useState(0);
  useEffect(() => {
    const id = setInterval(() => setFrame((f) => (f + 1) % 4), 400);
    return () => clearInterval(id);
  }, []);
  const dots = ".".repeat(frame);
  // Fixed width so layout doesn't jump.
  return <span className="inline-block w-4 text-left">{dots}</span>;
}
