import { useState } from "react";
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
  assistant_turn: "◎",
  tool_start: "▶",
  tool_done: "✓",
  approval_request: "⏸",
  approval_resolved: "✔",
};

const GLYPH_COLORS: Record<TraceEntry["kind"], string> = {
  assistant_turn: "text-cronymax-primary",
  tool_start: "text-amber-400",
  tool_done: "text-green-400",
  approval_request: "text-orange-400",
  approval_resolved: "text-green-300",
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
  const keys = [
    "command",
    "cmd",
    "path",
    "file",
    "file_path",
    "query",
    "url",
    "name",
    "message",
  ];
  for (const k of keys) {
    const v = obj[k];
    if (typeof v === "string" && v) return v;
  }
  // Fallback: stringify the first scalar field we find.
  for (const [k, v] of Object.entries(obj)) {
    if (typeof v === "string" && v) return `${k}=${v}`;
    if (typeof v === "number" || typeof v === "boolean")
      return `${k}=${String(v)}`;
  }
  return "";
}

function truncate(s: string, max = 80): string {
  if (s.length <= max) return s;
  return s.slice(0, max - 1) + "…";
}

function entryLabel(entry: TraceEntry): string {
  switch (entry.kind) {
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
  }
}

function entryDetail(entry: TraceEntry): unknown {
  switch (entry.kind) {
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
  }
}

/** Entries that are "child" rows (indented under their tool_start). */
function isChildEntry(entry: TraceEntry): boolean {
  return entry.kind === "tool_done";
}

function totalDuration(entries: TraceEntry[]): string {
  if (entries.length < 2) return "";
  const first = entries[0]!.ts;
  const last = entries[entries.length - 1]!.ts;
  const ms = last - first;
  if (ms < 1000) return `${ms}ms`;
  return `${(ms / 1000).toFixed(1)}s`;
}

function TraceRow({ entry, base }: { entry: TraceEntry; base: number }) {
  const [open, setOpen] = useState(false);
  const indented = isChildEntry(entry);
  const glyph = GLYPHS[entry.kind];
  const glyphColor = GLYPH_COLORS[entry.kind];
  const label = entryLabel(entry);
  const detail = entryDetail(entry);

  return (
    <div className={indented ? "ml-4" : ""}>
      <button
        type="button"
        onClick={() => setOpen((v) => !v)}
        className="flex w-full items-center gap-1.5 rounded px-1 py-0.5 text-left hover:bg-cronymax-border/30 transition"
      >
        <span
          className={`w-4 shrink-0 text-center font-mono text-[11px] ${glyphColor}`}
        >
          {glyph}
        </span>
        <span className="flex-1 truncate font-mono text-[11px] text-cronymax-caption">
          {label}
        </span>
        <span className="shrink-0 font-mono text-[10px] text-cronymax-caption opacity-60">
          {fmtRelTs(base, entry.ts)}
        </span>
        <span className="shrink-0 text-[10px] text-cronymax-caption opacity-40">
          {open ? "▾" : "▸"}
        </span>
      </button>

      {open && (
        <pre className="ml-6 max-h-[200px] overflow-y-auto rounded bg-cronymax-base px-2 py-1 font-mono text-[10px] text-cronymax-caption whitespace-pre-wrap break-all">
          {JSON.stringify(detail, null, 2)}
        </pre>
      )}
    </div>
  );
}

export function TraceViewer({ entries, startExpanded }: Props) {
  const [collapsed, setCollapsed] = useState(!startExpanded);

  if (entries.length === 0) return null;

  const base = entries[0]!.ts;
  const dur = totalDuration(entries);

  return (
    <div className="rounded border border-cronymax-border bg-cronymax-float text-xs">
      {/* Header / toggle */}
      <button
        type="button"
        onClick={() => setCollapsed((v) => !v)}
        className="flex w-full items-center gap-2 px-2 py-1.5 text-left hover:bg-cronymax-border/20 transition"
      >
        <span className="font-mono text-[10px] text-cronymax-caption">
          {collapsed ? "▶" : "▼"}
        </span>
        <span className="text-[11px] font-semibold text-cronymax-caption">
          Trace
        </span>
        <span className="text-[10px] text-cronymax-caption opacity-60">
          {entries.length} {entries.length === 1 ? "entry" : "entries"}
          {dur ? ` · ${dur}` : ""}
        </span>
      </button>

      {/* Waterfall rows */}
      {!collapsed && (
        <div className="border-t border-cronymax-border px-1 py-1 space-y-0.5">
          {entries.map((entry, i) => (
            <TraceRow key={i} entry={entry} base={base} />
          ))}
        </div>
      )}
    </div>
  );
}
