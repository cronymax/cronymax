import { Loader2 } from "lucide-react";
import { Streamdown } from "streamdown";
import { ExtensionContentBlock } from "./ExtensionContentBlock";
import { type ExtensionRenderer, lookupRendererByLang, useExtensionRendererRegistry } from "./extensionRenderers";
import type { ContentSegment } from "./store";
import { ThinkingBlock } from "./ThinkingBlock";
import { ToolCallCard } from "./ToolCallCard";

interface Props {
  segments: ContentSegment[];
  /** True while the run is actively streaming. */
  isStreaming: boolean;
}

/** Loading indicator while the stream is empty and the block is running. */
function LoadingDots() {
  return (
    <div className="flex items-center gap-1.5 text-xs italic text-muted-foreground select-none">
      <Loader2 className="size-3 animate-spin" />
      <span>Working…</span>
    </div>
  );
}

// ── Fenced code dispatch ─────────────────────────────────────────────────
//
// P6.5-T09: walk the rendered text segment and split it on **closed**
// fenced code blocks whose language tag matches an installed content
// renderer. Closed-only is per IDL D12 — an unclosed fence is still
// streaming, and we want Streamdown's default code-block path to render
// the partial source until the closing ``` arrives.
//
// We do NOT try to be a full markdown parser here. Fences inside code
// (e.g. ```` ```` snippets) are an edge case that the current grammar
// handles correctly because the regex looks for line-anchored ``` only.
// Inside an inline-code span (` ... `) a literal ``` would tear off here
// — acceptable for v1 alpha; mermaid blocks in prose work fine.

type Chunk =
  | { kind: "text"; content: string }
  | {
      kind: "extblock";
      lang: string;
      content: string;
      renderer: ExtensionRenderer;
    };

const FENCE_RE = /(^|\n)```([a-zA-Z][a-zA-Z0-9_\-+]*)\n([\s\S]*?)\n```(?=\n|$)/g;

function splitTextOnExtensionFences(text: string, registry: ExtensionRenderer[]): Chunk[] {
  if (registry.length === 0 || !text.includes("```")) {
    return [{ kind: "text", content: text }];
  }
  const out: Chunk[] = [];
  let cursor = 0;
  // Reset state because we're sharing the regex across calls.
  FENCE_RE.lastIndex = 0;
  for (let m = FENCE_RE.exec(text); m !== null; m = FENCE_RE.exec(text)) {
    // Regex has 3 mandatory capturing groups + the `full` match; the type
    // system can't see that, so we narrow here.
    const full = m[0];
    const leading = m[1] ?? "";
    const lang = m[2] ?? "";
    const body = m[3] ?? "";
    if (!lang) continue;
    const fenceStart = m.index + leading.length;
    const renderer = lookupRendererByLang(registry, lang);
    if (!renderer) continue;

    // Emit the literal text before this fence (preserving the leading
    // newline that anchored the fence, since it belongs to the text).
    if (fenceStart > cursor) {
      out.push({ kind: "text", content: text.slice(cursor, fenceStart) });
    }
    out.push({ kind: "extblock", lang, content: body, renderer });
    cursor = m.index + full.length;
  }
  if (cursor === 0) {
    return [{ kind: "text", content: text }];
  }
  if (cursor < text.length) {
    out.push({ kind: "text", content: text.slice(cursor) });
  }
  return out;
}

interface TextSegmentRendererProps {
  content: string;
  isStreaming: boolean;
  isLastText: boolean;
  segmentKey: number;
  messageId?: string;
}

function TextSegmentRenderer({ content, isStreaming, isLastText, segmentKey, messageId }: TextSegmentRendererProps) {
  const registry = useExtensionRendererRegistry();
  const chunks = splitTextOnExtensionFences(content, registry);

  // Fast path: no extension fences matched → emit one Streamdown for the
  // whole segment so streaming animation and Shiki code highlighting are
  // identical to the pre-P6.5 behaviour.
  if (chunks.length === 1 && chunks[0]!.kind === "text") {
    return (
      <Streamdown animated isAnimating={isStreaming && isLastText}>
        {content}
      </Streamdown>
    );
  }

  // Mixed text + extension blocks. Streamdown renders each text chunk on
  // its own; the last text chunk drives the streaming animation iff the
  // segment was the last text segment overall AND the last chunk in this
  // segment is a text chunk.
  const lastChunkIsText = chunks[chunks.length - 1]?.kind === "text";
  return (
    <>
      {chunks.map((chunk, i) => {
        if (chunk.kind === "text") {
          const isTail = isLastText && lastChunkIsText && i === chunks.length - 1;
          return (
            <Streamdown key={`${segmentKey}:t:${i}`} animated isAnimating={isStreaming && isTail}>
              {chunk.content}
            </Streamdown>
          );
        }
        return (
          <ExtensionContentBlock
            // Keying by (extId, rendererId, position) keeps the iframe
            // alive across content updates within the same block, so
            // streaming partial → final transitions are an update, not
            // a remount.
            key={`${segmentKey}:e:${chunk.renderer.extId}:${chunk.renderer.rendererId}:${i}`}
            renderer={chunk.renderer}
            language={chunk.lang}
            content={chunk.content}
            messageId={messageId}
          />
        );
      })}
    </>
  );
}

/**
 * Renders a `ContentSegment[]` in order, delegating to the appropriate
 * child component for each segment kind:
 * - `text`     → `<Streamdown>` (animated when streaming), optionally
 *                interleaved with `<ExtensionContentBlock>` for fenced
 *                code blocks whose language matches an installed
 *                `cronymax.content.renderer` contribution.
 * - `tool_call` → `<ToolCallCard>`
 * - `thinking`  → `<ThinkingBlock>`
 *
 * When the stream is empty and the block is running, shows a loading
 * indicator.
 */
export function ContentStreamView({ segments, isStreaming }: Props) {
  if (segments.length === 0 && isStreaming) {
    return <LoadingDots />;
  }
  if (segments.length === 0) return null;

  let lastTextIdx = -1;
  for (let i = segments.length - 1; i >= 0; i--) {
    if (segments[i]!.kind === "text") {
      lastTextIdx = i;
      break;
    }
  }

  return (
    <div className="flex flex-col gap-1">
      {segments.map((seg, i) => {
        if (seg.kind === "text") {
          return (
            <div key={i} className="text-sm text-foreground">
              <TextSegmentRenderer
                content={seg.content}
                isStreaming={isStreaming}
                isLastText={i === lastTextIdx}
                segmentKey={i}
              />
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
