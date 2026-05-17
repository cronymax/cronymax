import { ChevronDown, Loader2 } from "lucide-react";
import { useEffect, useState } from "react";
import { Streamdown } from "streamdown";
import { Button } from "@/components/ui/button";
import { Collapsible, CollapsibleContent, CollapsibleTrigger } from "@/components/ui/collapsible";

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
 * Once sealed (`sealed === true`):       shows a collapsible block with a
 *   "Thinking (N.Ns)" summary header and the accumulated thinking text inside,
 *   rendered via `<Streamdown>`.
 */
export function ThinkingBlock({ thinkingText, sealed, elapsedMs }: ThinkingBlockProps) {
  const [open, setOpen] = useState(false);

  // Auto-collapse when sealed.
  useEffect(() => {
    if (sealed) setOpen(false);
  }, [sealed]);

  // While streaming (not yet sealed): always show the animated indicator,
  // even before any thinking text has arrived.
  if (!sealed) {
    return (
      <div className="mb-2 flex items-center gap-1.5 text-xs italic text-muted-foreground select-none">
        <Loader2 className="size-3 animate-spin" />
        <span>Thinking…</span>
      </div>
    );
  }

  // Sealed but no content: nothing to show.
  if (!thinkingText) return null;

  const truncated = thinkingText.length > MAX_THINKING_CHARS;
  const displayText = truncated ? `${thinkingText.slice(0, MAX_THINKING_CHARS)}\n\n*… (truncated)*` : thinkingText;
  const elapsedSec = (elapsedMs / 1000).toFixed(1);

  return (
    <Collapsible
      open={open}
      onOpenChange={setOpen}
      className="mb-2 overflow-hidden rounded-md border border-border bg-secondary"
    >
      <CollapsibleTrigger asChild>
        <Button
          variant="ghost"
          className="flex h-auto w-full items-center justify-between rounded-none px-3 py-2 text-xs font-medium text-muted-foreground hover:bg-transparent hover:text-foreground"
        >
          <span>Thinking ({elapsedSec}s)</span>
          <ChevronDown
            className="size-3 transition-transform data-[state=closed]:-rotate-90"
            data-state={open ? "open" : "closed"}
          />
        </Button>
      </CollapsibleTrigger>
      <CollapsibleContent>
        <div className="border-t border-border px-3 py-2 text-xs text-muted-foreground">
          <Streamdown animated={false} isAnimating={false}>
            {displayText}
          </Streamdown>
        </div>
      </CollapsibleContent>
    </Collapsible>
  );
}
