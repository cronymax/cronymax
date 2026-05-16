import { useState } from "react";
import { Collapsible, CollapsibleContent, CollapsibleTrigger } from "@/components/ui/collapsible";
import { Separator } from "@/components/ui/separator";
import type { TraceEntry } from "./store";

interface Props {
  entries: TraceEntry[];
  /** Start the viewer expanded (e.g. while the run is streaming). */
  startExpanded?: boolean;
}

function fmtRelTs(base: number, ts: number): string {
  const delta = ts - base;
  if (delta < 1000) return `+${delta}ms`;
  return `+${(delta / 1000).toFixed(1)}s`;
}

const GLYPHS: Record<TraceEntry["kind"], string> = {
  run_start: "◉",
  assistant_turn: "◎",
  tool_start: "▶",
  tool_done: "✓",
  approval_request: "⏸",
  approval_resolved: "✔",
  reflection: "🪞",
  memory_write: "💾",
};

const GLYPH_COLORS: Record<TraceEntry["kind"], string> = {
  run_start: "text-muted-foreground",
  assistant_turn: "text-primary",
  tool_start: "text-amber-400",
  tool_done: "text-green-400",
  approval_request: "text-orange-400",
  approval_resolved: "text-green-300",
  reflection: "text-purple-400",
  memory_write: "text-sky-400",
};

/**
 * Pull the most informative one-liner out of a tool-call args object
 * so the trace row shows what *actually* happened without having to
 * click each entry open. Examples: shell command, file path, query.
 */
function summarizeArgs(args: unknown): string {
  // tool_start "arguments" arrives from the runtime as the raw JSON
  // string the model produced; try to parse so we can pull useful keys.
  let value: unknown = args;
  if (typeof value === "string") {
    const trimmed = value.trim();
    if (trimmed.startsWith("{")) {
      try {
        value = JSON.parse(trimmed);
      } catch {
        return value as string;
      }
    } else {
      return value as string;
    }
  }
  if (!value || typeof value !== "object") return "";
  const obj = value as Record<string, unknown>;
  // Common keys ordered from most to least specific.
  const keys = ["command", "cmd", "path", "file", "file_path", "query", "url", "name", "message"];
  for (const k of keys) {
    const v = obj[k];
    if (typeof v === "string" && v) return v;
  }
  // Fallback: stringify the first scalar field we find.
  for (const [k, v] of Object.entries(obj)) {
    if (typeof v === "string" && v) return `${k}=${v}`;
    if (typeof v === "number" || typeof v === "boolean") return `${k}=${String(v)}`;
  }
  return "";
}

function truncate(s: string, max = 80): string {
  if (s.length <= max) return s;
  return s.slice(0, max - 1) + "…";
}

function entryLabel(entry: TraceEntry): string {
  switch (entry.kind) {
    case "run_start":
      return `${entry.model} · ${entry.tools.length} tools · max ${entry.turnsLimit} turns`;
    case "assistant_turn":
      return `turn ${entry.turnId}${entry.finishReason ? ` (${entry.finishReason})` : ""}`;
    case "tool_start": {
      const tool = entry.tool || "tool";
      const summary = truncate(summarizeArgs(entry.args));
      return summary ? `${tool}: ${summary}` : tool;
    }
    case "tool_done":
      return `${entry.tool || "tool"} done`;
    case "approval_request":
      return `approval: ${entry.tool}`;
    case "approval_resolved":
      return `${entry.decision === "approve" ? "approved" : "denied"}: ${entry.reviewId.slice(0, 8)}`;
    case "reflection":
      return `reflection · turn ${entry.turn}`;
    case "memory_write":
      return `memory → ${entry.namespace}/${entry.key} (${entry.source})`;
  }
}

function entryDetail(entry: TraceEntry): unknown {
  switch (entry.kind) {
    case "run_start":
      return { systemPrompt: entry.systemPrompt, tools: entry.tools };
    case "assistant_turn":
      return { text: entry.text };
    case "tool_start":
      return entry.args;
    case "tool_done":
      return entry.result;
    case "approval_request":
      return entry.args;
    case "approval_resolved":
      return { reviewId: entry.reviewId, decision: entry.decision };
    case "reflection":
      return { text: entry.text };
    case "memory_write":
      return {
        namespace: entry.namespace,
        key: entry.key,
        source: entry.source,
      };
  }
}

/** Entries that are "child" rows (indented under their tool_start). */
function isChildEntry(entry: TraceEntry): boolean {
  return entry.kind === "tool_done";
}

/** Sort entries so run_start always appears first. */
function sortedEntries(entries: TraceEntry[]): TraceEntry[] {
  const runStart = entries.filter((e) => e.kind === "run_start");
  const rest = entries.filter((e) => e.kind !== "run_start");
  return [...runStart, ...rest];
}

function totalDuration(entries: TraceEntry[]): string {
  if (entries.length < 2) return "";
  const first = entries[0]!.ts;
  const last = entries[entries.length - 1]!.ts;
  const ms = last - first;
  if (ms < 1000) return `${ms}ms`;
  return `${(ms / 1000).toFixed(1)}s`;
}

function totalUsageSummary(entries: TraceEntry[]): { inputTokens: number; outputTokens: number } | null {
  let inputTokens = 0;
  let outputTokens = 0;
  for (const e of entries) {
    if (e.kind === "assistant_turn" && e.usage) {
      inputTokens += e.usage.inputTokens;
      outputTokens += e.usage.outputTokens;
    }
  }
  return inputTokens > 0 || outputTokens > 0 ? { inputTokens, outputTokens } : null;
}

function fmtTokenCount(n: number): string {
  if (n >= 1000) return `${(n / 1000).toFixed(1)}k`;
  return String(n);
}

function TraceRow({ entry, base }: { entry: TraceEntry; base: number }) {
  const [open, setOpen] = useState(false);
  const indented = isChildEntry(entry);
  const glyph = GLYPHS[entry.kind];
  const glyphColor = GLYPH_COLORS[entry.kind];
  const label = entryLabel(entry);

  // run_start shows system prompt as plain text + tool list, not JSON
  const isRunStart = entry.kind === "run_start";
  const detail = isRunStart ? null : entryDetail(entry);

  return (
    <Collapsible open={open} onOpenChange={setOpen} className={indented ? "ml-4" : ""}>
      <CollapsibleTrigger asChild>
        <button
          type="button"
          className="flex w-full items-center gap-1.5 rounded px-1 py-0.5 text-left hover:bg-border/30 transition"
        >
          <span className={`w-4 shrink-0 text-center font-mono text-xs ${glyphColor}`}>{glyph}</span>
          <span className="flex-1 truncate font-mono text-xs text-muted-foreground">{label}</span>
          {!isRunStart && (
            <span className="shrink-0 font-mono text-xs text-muted-foreground opacity-60">
              {fmtRelTs(base, entry.ts)}
            </span>
          )}
          <span className="shrink-0 text-xs text-muted-foreground opacity-40">{open ? "▾" : "▸"}</span>
        </button>
      </CollapsibleTrigger>

      <CollapsibleContent>
        {isRunStart && entry.kind === "run_start" ? (
          <div className="ml-6 mt-0.5 space-y-1">
            <div className="text-xs text-muted-foreground opacity-70 font-mono">
              Tools: {entry.tools.join(", ") || "(none)"}
            </div>
            <pre className="max-h-[240px] overflow-y-auto rounded bg-background px-2 py-1 font-mono text-xs text-muted-foreground whitespace-pre-wrap break-all">
              {entry.systemPrompt || "(no system prompt)"}
            </pre>
            {entry.userInput && (
              <>
                <div className="text-xs text-muted-foreground opacity-70 font-mono mt-1">User message:</div>
                <pre className="max-h-[240px] overflow-y-auto rounded bg-background px-2 py-1 font-mono text-xs text-muted-foreground whitespace-pre-wrap break-all">
                  {entry.userInput}
                </pre>
              </>
            )}
          </div>
        ) : (
          <pre className="ml-6 max-h-[200px] overflow-y-auto rounded bg-background px-2 py-1 font-mono text-xs text-muted-foreground whitespace-pre-wrap break-all">
            {JSON.stringify(detail, null, 2)}
          </pre>
        )}
      </CollapsibleContent>
    </Collapsible>
  );
}

export function TraceViewer({ entries, startExpanded }: Props) {
  const [collapsed, setCollapsed] = useState(!startExpanded);

  if (entries.length === 0) return null;

  const base = entries[0]!.ts;
  const dur = totalDuration(entries);
  const usage = totalUsageSummary(entries);
  const inputTokens = usage?.inputTokens ?? 0;
  const outputTokens = usage?.outputTokens ?? 0;

  return (
    <Collapsible
      open={!collapsed}
      onOpenChange={(o) => setCollapsed(!o)}
      className="rounded border border-border bg-card text-xs"
    >
      {/* Header / toggle */}
      <CollapsibleTrigger asChild>
        <button
          type="button"
          className="flex w-full items-center gap-2 px-2 py-1.5 text-left hover:bg-border/20 transition"
        >
          <span className="font-mono text-xs text-muted-foreground">{collapsed ? "▶" : "▼"}</span>
          <span className="text-xs font-semibold text-muted-foreground">Trace</span>
          <span className="text-xs text-muted-foreground opacity-60">
            {entries.length} {entries.length === 1 ? "entry" : "entries"}
            {dur ? ` · ${dur}` : ""}
            {usage
              ? ` · ${fmtTokenCount(inputTokens + outputTokens)} tok (↑${fmtTokenCount(inputTokens)} ↓${fmtTokenCount(outputTokens)})`
              : ""}
          </span>
        </button>
      </CollapsibleTrigger>

      {/* Waterfall rows */}
      <CollapsibleContent>
        <Separator />
        <div className="px-1 py-1 space-y-0.5">
          {sortedEntries(entries).map((entry, i) => (
            <TraceRow key={i} entry={entry} base={base} />
          ))}
        </div>
      </CollapsibleContent>
    </Collapsible>
  );
}
