import { useEffect, useState } from "react";
import { Streamdown } from "streamdown";
import type { ContentSegment } from "./store";
import { ThinkingBlock } from "./ThinkingBlock";
import { ToolCallCard } from "./ToolCallCard";

interface Props {
  segments: ContentSegment[];
  /** True while the run is actively streaming. */
  isStreaming: boolean;
}

/** Animated loading dots while the stream is empty and the block is running. */
function LoadingDots() {
  const [frame, setFrame] = useState(0);
  useEffect(() => {
    const id = setInterval(() => setFrame((f) => (f + 1) % 4), 400);
    return () => clearInterval(id);
  }, []);
  const dots = ".".repeat(frame);
  return (
    <div className="flex items-center gap-1.5 text-xs text-muted-foreground italic select-none">
      <span>Working</span>
      <span className="inline-block w-4 text-left">{dots}</span>
    </div>
  );
}

/**
 * Renders a `ContentSegment[]` in order, delegating to the appropriate
 * child component for each segment kind:
 * - `text`     → `<Streamdown>` (animated when streaming)
 * - `tool_call` → `<ToolCallCard>`
 * - `thinking`  → `<ThinkingBlock>`
 *
 * When the stream is empty and the block is running, shows a loading indicator.
 */
export function ContentStreamView({ segments, isStreaming }: Props) {
  // Loading indicator: stream empty + run in progress
  if (segments.length === 0 && isStreaming) {
    return <LoadingDots />;
  }

  if (segments.length === 0) return null;

  // Determine which text segment is the last one (for streaming animation).
  // Only the last text segment should be animated while isStreaming is true.
  let lastTextIdx = -1;
  for (let i = segments.length - 1; i >= 0; i--) {
    if (segments[i]!.kind === "text") {
      lastTextIdx = i;
      break;
    }
  }

  return (
    <div className="space-y-1">
      {segments.map((seg, i) => {
        if (seg.kind === "text") {
          const isLastText = i === lastTextIdx;
          return (
            <div key={i} className="text-sm text-foreground">
              <Streamdown animated isAnimating={isStreaming && isLastText}>
                {seg.content}
              </Streamdown>
            </div>
          );
        }

        if (seg.kind === "tool_call") {
          return <ToolCallCard key={i} segment={seg} />;
        }

        if (seg.kind === "thinking") {
          return <ThinkingBlock key={i} thinkingText={seg.content} sealed={seg.sealed} elapsedMs={seg.elapsedMs} />;
        }

        return null;
      })}
    </div>
  );
}
