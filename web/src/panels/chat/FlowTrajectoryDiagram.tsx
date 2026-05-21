/**
 * FlowTrajectoryDiagram — compact agent-trajectory strip that floats above
 * the chat prompt editor whenever a flow is selected and has active or
 * recently-completed runs in the current session.
 *
 * Layout
 * ──────
 * [status] agent-name → [status] agent-name → … (horizontally scrollable)
 *
 * Each chip represents one node in the selected flow's graph.  Nodes are
 * ordered left-to-right by their saved x-position in the FlowEditor
 * localStorage spec (falling back to topological order for YAML-only flows
 * that have never been opened in the editor).  Back-edges (cycles such as
 * QA ↔ RD-patch) are rendered as a dashed loop indicator below the strip
 * rather than breaking the linear layout.
 *
 * Status is derived from the most recent sub-run whose agent_id matches
 * the node's agent_name:
 *   • pending          — muted grey dot
 *   • running          — pulsing primary dot
 *   • awaiting_review  — pulsing amber dot
 *   • succeeded        — green check
 *   • failed/cancelled — red X
 *
 * The strip is collapsible.  A small "#N" badge disambiguates when
 * multiple flow runs exist for the session; clicking the flow-run ID chip
 * cycles through them.
 */

import { Activity, ChevronDown, ChevronUp } from "lucide-react";
import { useCallback, useEffect, useRef, useState } from "react";
import { Collapsible, CollapsibleContent, CollapsibleTrigger } from "@/components/ui/collapsible";
import { cn } from "@/lib/utils";
import { runtime, shells } from "@/shells/bridge";
import type { ContentSegment, NodeConversation, StatusKind, TraceEntry } from "./store";

// ── Types ─────────────────────────────────────────────────────────────────

interface FlowNodeDef {
  /** Node id (equals agent_name in the FlowEditor schema). */
  id: number;
  name: string;
  /** The agent_id that runs on this node. */
  agentName: string;
  /** Canvas x-position — used to produce a stable left-to-right ordering. */
  x: number;
}

interface FlowEdgeDef {
  from_id: number;
  to_id: number;
  port?: string;
}

interface FlowTopology {
  /** Nodes sorted left-to-right by x-position. */
  orderedNodes: FlowNodeDef[];
  /** All edges (including back-edges). */
  edges: FlowEdgeDef[];
  /** Set of (from_id, to_id) pairs that are "back-edges" (right→left). */
  backEdges: Set<string>;
}

interface SubRun {
  id: string;
  agentId: string | null;
  status: string;
}

interface FlowRunEntry {
  flowRunId: string;
  index: number;
  /** Ordered by insertion (temporal order). */
  subRuns: SubRun[];
}

// StatusKind is imported from store.ts

// ── localStorage flow-spec helpers ────────────────────────────────────────

interface StoredNode {
  id: number;
  name: string;
  x: number;
  config?: Record<string, string>;
  produces?: Array<{ doc_type: string; reviewers: string }>;
}

interface StoredEdge {
  from_id: number;
  to_id: number;
  port?: string;
}

interface StoredFlowSpec {
  nodes: StoredNode[];
  edges: StoredEdge[];
}

function loadFlowTopology(flowName: string): FlowTopology | null {
  if (!flowName) return null;
  try {
    const raw = localStorage.getItem("flows");
    if (!raw) return null;
    const all = JSON.parse(raw) as Record<string, StoredFlowSpec>;
    const spec = all[flowName];
    if (!spec || !Array.isArray(spec.nodes) || spec.nodes.length === 0) return null;

    // Sort nodes by x so the strip reads left-to-right.
    const sorted = [...spec.nodes].sort((a, b) => (a.x ?? 0) - (b.x ?? 0));

    const orderedNodes: FlowNodeDef[] = sorted.map((n) => ({
      id: n.id,
      name: n.name || n.config?.agent_name || String(n.id),
      agentName: n.config?.agent_name || n.name || String(n.id),
      x: n.x ?? 0,
    }));

    const edges: FlowEdgeDef[] = (spec.edges ?? []).map((e) => ({
      from_id: e.from_id,
      to_id: e.to_id,
      port: e.port,
    }));

    // Identify back-edges (from_id's x >= to_id's x → right-to-left arc).
    const nodeXMap = new Map(sorted.map((n) => [n.id, n.x ?? 0]));
    const backEdges = new Set<string>();
    for (const e of edges) {
      const fx = nodeXMap.get(e.from_id) ?? 0;
      const tx = nodeXMap.get(e.to_id) ?? 0;
      if (fx >= tx) {
        backEdges.add(`${e.from_id}→${e.to_id}`);
      }
    }

    return { orderedNodes, edges, backEdges };
  } catch {
    return null;
  }
}

// ── Status helpers ────────────────────────────────────────────────────────

function parseStatusKind(s: string): StatusKind {
  switch (s) {
    case "running":
    case "pending":
      return s === "running" ? "running" : "pending";
    case "awaiting_review":
      return "awaiting_review";
    case "succeeded":
      return "succeeded";
    case "failed":
    case "cancelled":
      return "failed";
    default:
      return "pending";
  }
}

/**
 * Derive the "most important" status for a given agent across all its
 * sub-runs in a flow run.  Priority: running > awaiting_review > failed >
 * succeeded > pending.
 */
function aggregateStatus(subRuns: SubRun[], agentId: string): StatusKind {
  const relevant = subRuns.filter((r) => r.agentId === agentId);
  if (relevant.length === 0) return "pending";
  const kinds = relevant.map((r) => parseStatusKind(r.status));
  if (kinds.includes("running")) return "running";
  if (kinds.includes("awaiting_review")) return "awaiting_review";
  if (kinds.includes("failed")) return "failed";
  if (kinds.includes("succeeded")) return "succeeded";
  return "pending";
}

// ── Sub-components ────────────────────────────────────────────────────────

function StatusIndicator({ status }: { status: StatusKind }) {
  if (status === "running") {
    return <span className="inline-flex h-2 w-2 rounded-full bg-primary animate-pulse shrink-0" />;
  }
  if (status === "awaiting_review") {
    return <span className="inline-flex h-2 w-2 rounded-full bg-amber-400 animate-pulse shrink-0" />;
  }
  if (status === "succeeded") {
    return (
      <svg
        className="h-3 w-3 text-green-400 shrink-0"
        viewBox="0 0 24 24"
        fill="none"
        stroke="currentColor"
        strokeWidth={2.5}
        aria-label="Succeeded"
      >
        <polyline points="20 6 9 17 4 12" />
      </svg>
    );
  }
  if (status === "failed") {
    return (
      <svg
        className="h-3 w-3 text-red-400 shrink-0"
        viewBox="0 0 24 24"
        fill="none"
        stroke="currentColor"
        strokeWidth={2.5}
        aria-label="Failed"
      >
        <line x1="18" y1="6" x2="6" y2="18" />
        <line x1="6" y1="6" x2="18" y2="18" />
      </svg>
    );
  }
  // pending
  return (
    <span className="inline-flex h-2 w-2 rounded-full bg-muted-foreground/25 border border-muted-foreground/30 shrink-0" />
  );
}

function NodeChip({
  node,
  status,
  isActive,
  isSelected,
  onClick,
}: {
  node: FlowNodeDef;
  status: StatusKind;
  isActive: boolean;
  isSelected?: boolean;
  onClick?: () => void;
}) {
  return (
    <div
      role={onClick ? "button" : undefined}
      tabIndex={onClick ? 0 : undefined}
      onClick={onClick}
      onKeyDown={
        onClick
          ? (e) => {
              if (e.key === "Enter" || e.key === " ") onClick();
            }
          : undefined
      }
      className={cn(
        "flex items-center gap-1.5 rounded-md border px-2 py-1 text-xs font-medium shrink-0 transition-colors",
        onClick && "cursor-pointer hover:border-primary/50",
        isSelected
          ? "border-primary bg-primary/15 text-foreground ring-1 ring-primary/30"
          : isActive
            ? "border-primary/60 bg-primary/10 text-foreground"
            : status === "succeeded"
              ? "border-green-500/30 bg-green-500/5 text-muted-foreground"
              : status === "failed"
                ? "border-red-500/30 bg-red-500/5 text-muted-foreground"
                : status === "awaiting_review"
                  ? "border-amber-400/50 bg-amber-400/10 text-foreground"
                  : "border-border bg-card text-muted-foreground",
      )}
    >
      <StatusIndicator status={status} />
      <span className="max-w-[96px] truncate">{node.name}</span>
    </div>
  );
}

/** Straight forward-edge arrow between two chips. */
function ArrowSep({ hasBackEdge }: { hasBackEdge?: boolean }) {
  return (
    <div className="flex flex-col items-center shrink-0 self-center">
      {/* Forward arrow */}
      <svg className="h-3 w-5 text-muted-foreground/50" viewBox="0 0 20 12" fill="none" aria-hidden="true">
        <path
          d="M1 6 L16 6 M11 1 L17 6 L11 11"
          stroke="currentColor"
          strokeWidth="1.5"
          strokeLinecap="round"
          strokeLinejoin="round"
        />
      </svg>
      {/* Cycle indicator below the arrow */}
      {hasBackEdge && (
        <svg
          className="h-2.5 w-5 text-amber-500/60 mt-0.5"
          viewBox="0 0 20 10"
          fill="none"
          aria-label="Has back-edge (cycle)"
        >
          <path
            d="M2 1 Q10 8 18 1"
            stroke="currentColor"
            strokeWidth="1.2"
            strokeDasharray="2 2"
            strokeLinecap="round"
          />
        </svg>
      )}
    </div>
  );
}

// ── useFlowNodeConversations hook ─────────────────────────────────────────

/**
 * Subscribes to per-run runtime events for all sub-runs in the given
 * `flowRuns` snapshot. Returns a live `Map<agentId, NodeConversation>` that
 * updates as token / trace / run_status events arrive.
 *
 * Also subscribes to `runtime.on("session:{sessionId}")` to discover new
 * sub-runs that start after the snapshot was taken.
 */
export function useFlowNodeConversations(
  sessionId: string | null | undefined,
  flowRuns: FlowRunEntry[],
  topology: FlowTopology | null,
): Map<string, NodeConversation> {
  const [conversations, setConversations] = useState<Map<string, NodeConversation>>(new Map());

  // runId → agentId
  const runAgentMapRef = useRef<Map<string, string>>(new Map());
  // runId → unsubscribe function
  const unsubsRef = useRef<Map<string, () => void>>(new Map());
  // Seeded flowRunIds (avoids double-seeding topology placeholder nodes)
  const seededFridsRef = useRef<Set<string>>(new Set());

  /** Process a single runtime event for a known agentId. */
  const processRunEvent = useCallback((agentId: string, event: unknown) => {
    const ev = event as { payload?: Record<string, unknown> } | null;
    if (!ev?.payload) return;
    const pl = ev.payload;
    const kind = pl.kind as string | undefined;

    setConversations((prev) => {
      const conv = prev.get(agentId);
      if (!conv) return prev;
      const next = new Map(prev);

      if (kind === "run_status") {
        const newStatus = parseStatusKind((pl.status as string) ?? "pending");
        next.set(agentId, {
          ...conv,
          status: newStatus,
          startedAt: newStatus === "running" && conv.startedAt === null ? Date.now() : conv.startedAt,
        });
        return next;
      }

      if (kind === "token") {
        const delta = (pl.delta as string) ?? "";
        if (!delta) return prev;
        const stream = [...conv.contentStream];
        const last = stream[stream.length - 1];
        if (last?.kind === "thinking" && !(last as { sealed?: boolean }).sealed) {
          stream[stream.length - 1] = { ...last, sealed: true } as ContentSegment;
        }
        const prevLast = stream[stream.length - 1];
        if (prevLast?.kind === "text") {
          stream[stream.length - 1] = { kind: "text", content: prevLast.content + delta };
        } else {
          stream.push({ kind: "text", content: delta });
        }
        next.set(agentId, { ...conv, contentStream: stream });
        return next;
      }

      if (kind === "thinking_token") {
        const delta = (pl.delta as string) ?? "";
        if (!delta) return prev;
        const stream = [...conv.contentStream];
        const last = stream[stream.length - 1];
        if (last?.kind === "thinking" && !(last as { sealed?: boolean }).sealed) {
          stream[stream.length - 1] = {
            kind: "thinking",
            content: (last as { content: string }).content + delta,
            sealed: false,
            elapsedMs: 0,
          };
        } else {
          stream.push({ kind: "thinking", content: delta, sealed: false, elapsedMs: 0 });
        }
        next.set(agentId, { ...conv, contentStream: stream });
        return next;
      }

      if (kind === "trace") {
        const trace = (pl.trace as Record<string, unknown> | undefined) ?? pl;
        const tk = trace.kind as string | undefined;
        const ts = Date.now();

        if (tk === "tool_start") {
          const toolCallId = (trace.tool_call_id as string) ?? "";
          const tool = (trace.tool as string) ?? "";
          const args = trace.arguments ?? trace.args ?? {};
          const entry: TraceEntry = { kind: "tool_start", toolCallId, tool, args, ts };
          const segment: ContentSegment = { kind: "tool_call", toolCallId, tool, args, status: "running" };
          next.set(agentId, {
            ...conv,
            contentStream: [...conv.contentStream, segment],
            traceEntries: [...conv.traceEntries, entry],
          });
          return next;
        }

        if (tk === "tool_done") {
          const toolCallId = (trace.tool_call_id as string) ?? "";
          const tool = (trace.tool as string) ?? "";
          const result = trace.result ?? {};
          const isError = (trace.is_error as boolean | undefined) ?? false;
          const durationMs = trace.duration_ms as number | undefined;
          const entry: TraceEntry = {
            kind: "tool_done",
            toolCallId,
            tool,
            result,
            terminal: (trace.terminal as boolean | undefined) ?? false,
            ts,
            ...(durationMs != null ? { durationMs } : {}),
            ...(isError ? { isError: true } : {}),
          };
          const stream = conv.contentStream.map((seg) => {
            if (seg.kind === "tool_call" && seg.toolCallId === toolCallId) {
              return {
                ...seg,
                status: isError ? ("error" as const) : ("done" as const),
                result,
                ...(durationMs != null ? { durationMs } : {}),
              };
            }
            return seg;
          });
          next.set(agentId, {
            ...conv,
            contentStream: stream,
            traceEntries: [...conv.traceEntries, entry],
          });
          return next;
        }
      }

      return prev;
    });
  }, []);

  /** Subscribe to a run's event stream if not already subscribed. */
  const subscribeToRun = useCallback(
    (runId: string, agentId: string) => {
      if (unsubsRef.current.has(runId) || !runId) return;
      runAgentMapRef.current.set(runId, agentId);
      const off = runtime.on(`run:${runId}`, (event) => processRunEvent(agentId, event));
      if (off) unsubsRef.current.set(runId, off);
    },
    [processRunEvent],
  );

  /** Seed + subscribe when flowRuns change (new snapshot data). */
  useEffect(() => {
    for (const flowRun of flowRuns) {
      // Pre-seed placeholder NodeConversation for every topology node on first
      // encounter of this flow run, so the panel can show pending chips immediately.
      if (!seededFridsRef.current.has(flowRun.flowRunId) && topology) {
        seededFridsRef.current.add(flowRun.flowRunId);
        setConversations((prev) => {
          const next = new Map(prev);
          for (const node of topology.orderedNodes) {
            if (!next.has(node.agentName)) {
              next.set(node.agentName, {
                runId: "",
                agentId: node.agentName,
                flowRunId: flowRun.flowRunId,
                status: "pending",
                contentStream: [],
                traceEntries: [],
                startedAt: null,
              });
            }
          }
          return next;
        });
      }

      // Register snapshot sub-runs and subscribe to their event streams.
      for (const subRun of flowRun.subRuns) {
        if (!subRun.agentId || !subRun.id || runAgentMapRef.current.has(subRun.id)) continue;
        const snapshotStatus = parseStatusKind(subRun.status);
        setConversations((prev) => {
          const next = new Map(prev);
          const existing = next.get(subRun.agentId!);
          next.set(subRun.agentId!, {
            runId: subRun.id,
            agentId: subRun.agentId!,
            flowRunId: flowRun.flowRunId,
            status: snapshotStatus,
            contentStream: existing?.contentStream ?? [],
            traceEntries: existing?.traceEntries ?? [],
            startedAt: existing?.startedAt ?? (snapshotStatus === "running" ? Date.now() : null),
          });
          return next;
        });
        subscribeToRun(subRun.id, subRun.agentId);
      }
    }
  }, [flowRuns, topology, subscribeToRun]);

  /** Subscribe to session-level events to discover new sub-runs. */
  useEffect(() => {
    if (!sessionId) return;
    const off = runtime.on(`session:${sessionId}`, (raw: unknown) => {
      const ev = raw as { payload?: Record<string, unknown> } | null;
      if (!ev?.payload || ev.payload.kind !== "run_status") return;
      const pl = ev.payload;
      const runId = (pl.run_id as string | undefined) ?? "";
      const agentId = (pl.agent_id as string | undefined) ?? null;
      if (!runId || !agentId || runAgentMapRef.current.has(runId)) return;
      // Seed a fresh NodeConversation for this new sub-run.
      setConversations((prev) => {
        const next = new Map(prev);
        const existing = next.get(agentId);
        next.set(agentId, {
          runId,
          agentId,
          flowRunId: (pl.flow_run_id as string | undefined) ?? "",
          status: parseStatusKind((pl.status as string) ?? "pending"),
          contentStream: existing?.contentStream ?? [],
          traceEntries: existing?.traceEntries ?? [],
          startedAt: null,
        });
        return next;
      });
      subscribeToRun(runId, agentId);
    });
    return () => off?.();
  }, [sessionId, subscribeToRun]);

  /** Cleanup when sessionId changes. */
  useEffect(() => {
    return () => {
      for (const off of unsubsRef.current.values()) off();
      unsubsRef.current.clear();
      runAgentMapRef.current.clear();
      seededFridsRef.current.clear();
      setConversations(new Map());
    };
  }, [sessionId]);

  return conversations;
}

// ── Main component ────────────────────────────────────────────────────────

interface Props {
  selectedFlow: string;
  sessionId: string | null | undefined;
  onSelectNode?: (agentName: string | null) => void;
  selectedNodeId?: string | null;
  onConversationsUpdate?: (convs: Map<string, NodeConversation>) => void;
}

export function FlowTrajectoryDiagram({
  selectedFlow,
  sessionId,
  onSelectNode,
  selectedNodeId,
  onConversationsUpdate,
}: Props) {
  const [topology, setTopology] = useState<FlowTopology | null>(null);
  const [flowRuns, setFlowRuns] = useState<FlowRunEntry[]>([]);
  const [activeRunIndex, setActiveRunIndex] = useState(0);
  const [collapsed, setCollapsed] = useState(false);

  // Mutable refs so event handlers never go stale.
  const subRunsRef = useRef<Map<string, Map<string, SubRun>>>(new Map());
  const flowRunOrderRef = useRef<Map<string, number>>(new Map());

  // ── Topology ─────────────────────────────────────────────────────────────
  useEffect(() => {
    if (!selectedFlow) {
      setTopology(null);
      return;
    }
    const topo = loadFlowTopology(selectedFlow);
    setTopology(topo);
  }, [selectedFlow]);

  // Re-check topology when localStorage changes (FlowEditor may have saved).
  useEffect(() => {
    const handler = (e: StorageEvent) => {
      if (e.key === "flows") {
        setTopology(selectedFlow ? loadFlowTopology(selectedFlow) : null);
      }
    };
    window.addEventListener("storage", handler);
    return () => window.removeEventListener("storage", handler);
  }, [selectedFlow]);

  // ── Flow run tracking ─────────────────────────────────────────────────
  const rebuildFlowRuns = useCallback(() => {
    const entries: FlowRunEntry[] = [];
    for (const [flowRunId, runsMap] of subRunsRef.current.entries()) {
      const subRuns = Array.from(runsMap.values());
      const index = flowRunOrderRef.current.get(flowRunId) ?? entries.length + 1;
      entries.push({ flowRunId, index, subRuns });
    }
    entries.sort((a, b) => a.index - b.index);
    setFlowRuns((prev) => {
      // Keep active pointer on the last run (or preserve user choice).
      if (entries.length > 0 && prev.length === 0) setActiveRunIndex(entries.length - 1);
      return entries;
    });
  }, []);

  // Snapshot on mount / sessionId change.
  useEffect(() => {
    if (!sessionId) return;
    subRunsRef.current.clear();
    flowRunOrderRef.current.clear();

    shells.browser.activity
      .snapshot()
      .then((resp: { runs?: unknown[]; pending_reviews?: unknown[] }) => {
        const rawRuns = resp.runs ?? [];
        let seq = 0;
        for (const raw of rawRuns) {
          const r = raw as Record<string, unknown>;
          const runSessionId = (r.session_id as string | null) ?? null;
          const flowRunId = (r.flow_run_id as string | null) ?? null;
          if (runSessionId !== sessionId || !flowRunId) continue;
          if (!flowRunOrderRef.current.has(flowRunId)) {
            seq += 1;
            flowRunOrderRef.current.set(flowRunId, seq);
            subRunsRef.current.set(flowRunId, new Map());
          }
          const runsMap = subRunsRef.current.get(flowRunId)!;
          const runId = (r.id as string) ?? "";
          const spec = (r.spec as Record<string, unknown> | null) ?? {};
          runsMap.set(runId, {
            id: runId,
            agentId: (r.agent_id as string | null) ?? (spec.agent_name as string | null) ?? null,
            status: parseRawStatus(r.status),
          });
        }
        rebuildFlowRuns();
      })
      .catch(() => undefined);
  }, [sessionId, rebuildFlowRuns]);

  // Live run_status events via targeted session subscription.
  useEffect(() => {
    if (!sessionId) return;
    const off = runtime.on(`session:${sessionId}`, (raw: unknown) => {
      const ev = raw as { payload?: Record<string, unknown> } | null;
      if (!ev?.payload || ev.payload.kind !== "run_status") return;
      const pl = ev.payload;

      const runId = (pl.run_id as string | undefined) ?? "";
      const status = (pl.status as string | undefined) ?? "pending";
      const flowRunId = (pl.flow_run_id as string | undefined) ?? null;
      const agentId = (pl.agent_id as string | undefined) ?? null;

      if (!flowRunId || !runId) return;

      if (!subRunsRef.current.has(flowRunId)) {
        const nextSeq = subRunsRef.current.size + 1;
        flowRunOrderRef.current.set(flowRunId, nextSeq);
        subRunsRef.current.set(flowRunId, new Map());
        setActiveRunIndex(nextSeq - 1);
      }
      subRunsRef.current.get(flowRunId)!.set(runId, { id: runId, agentId, status });
      rebuildFlowRuns();
    });
    return () => off?.();
  }, [sessionId, rebuildFlowRuns]);

  // ── Node conversations hook ─────────────────────────────────────────
  const conversations = useFlowNodeConversations(sessionId, flowRuns, topology);
  const onConversationsUpdateRef = useRef(onConversationsUpdate);
  onConversationsUpdateRef.current = onConversationsUpdate;
  useEffect(() => {
    onConversationsUpdateRef.current?.(conversations);
  }, [conversations]);

  // ── Render guard ──────────────────────────────────────────────────────
  if (!selectedFlow || flowRuns.length === 0) return null;

  const activeRun = flowRuns[Math.min(activeRunIndex, flowRuns.length - 1)];
  if (!activeRun) return null;

  const { subRuns, flowRunId } = activeRun;

  // ── Build the chip strip ──────────────────────────────────────────────
  // When we have topology: use orderedNodes.
  // Fallback: derive nodes from unique agent_ids in temporal order.
  let displayNodes: Array<{ key: string; name: string; agentName: string }> = [];
  let hasTopology = false;

  if (topology && topology.orderedNodes.length > 0) {
    hasTopology = true;
    displayNodes = topology.orderedNodes.map((n) => ({
      key: String(n.id),
      name: n.name,
      agentName: n.agentName,
    }));
  } else {
    // Fallback: unique agents in first-seen order.
    const seen = new Set<string>();
    for (const sr of subRuns) {
      const aid = sr.agentId ?? "";
      if (aid && !seen.has(aid)) {
        seen.add(aid);
        displayNodes.push({ key: aid, name: aid, agentName: aid });
      }
    }
  }

  // For each consecutive pair, check if there's a back-edge between them.
  function hasBackEdgeBetween(i: number): boolean {
    if (!topology || !hasTopology || i >= displayNodes.length - 1) return false;
    const fromKey = Number(displayNodes[i]!.key);
    const toKey = Number(displayNodes[i + 1]!.key);
    return topology.backEdges.has(`${fromKey}→${toKey}`) || topology.backEdges.has(`${toKey}→${fromKey}`);
  }

  return (
    <Collapsible
      open={!collapsed}
      onOpenChange={() => setCollapsed((c) => !c)}
      className={cn(
        "mx-0 rounded-lg border border-border/50 bg-card/95 shadow-sm backdrop-blur-sm overflow-hidden",
        "text-xs transition-all",
      )}
    >
      {/* ── Header ────────────────────────────────────────────────────── */}
      <CollapsibleTrigger className="flex w-full items-center gap-2 px-2.5 py-1.5 transition-colors hover:bg-accent/50">
        <Activity className="h-3 w-3 shrink-0 text-muted-foreground" />
        <span className="truncate font-medium text-muted-foreground">{selectedFlow}</span>

        {/* Run selector pills when multiple runs exist */}
        {flowRuns.length > 1 && (
          <div className="flex items-center gap-1">
            {flowRuns.map((run, idx) => (
              <button
                key={run.flowRunId}
                type="button"
                onClick={() => setActiveRunIndex(idx)}
                className={cn(
                  "rounded px-1.5 py-0.5 font-mono text-xs transition",
                  idx === activeRunIndex ? "bg-primary/20 text-primary" : "text-muted-foreground hover:bg-muted",
                )}
              >
                #{run.index}
              </button>
            ))}
          </div>
        )}

        {/* Run ID badge */}
        <span className="font-mono text-xs text-muted-foreground/60">{flowRunId.slice(0, 8)}</span>

        {/* Collapse toggle */}
        {collapsed ? <ChevronDown className="h-3 w-3" /> : <ChevronUp className="h-3 w-3" />}
      </CollapsibleTrigger>

      {/* ── Strip ────────────────────────────────────────────────────── */}
      {!collapsed && (
        <CollapsibleContent className="border-t border-border/50 overflow-x-auto bg-background px-2.5 py-2">
          <div className="flex items-center gap-2 min-w-max">
            {displayNodes.map((node, i) => {
              const status = aggregateStatus(subRuns, node.agentName);
              const isActive = status === "running" || status === "awaiting_review";
              const isLast = i === displayNodes.length - 1;
              const backEdge = hasBackEdgeBetween(i);

              return (
                <div key={node.key} className="flex items-center gap-0.5">
                  <NodeChip
                    node={{ ...node, id: Number(node.key), x: 0 }}
                    status={status}
                    isActive={isActive}
                    isSelected={selectedNodeId === node.agentName}
                    onClick={
                      onSelectNode
                        ? () => onSelectNode(selectedNodeId === node.agentName ? null : node.agentName)
                        : undefined
                    }
                  />
                  {!isLast && <ArrowSep hasBackEdge={backEdge} />}
                </div>
              );
            })}
          </div>

          {/* Cycle legend — only when there are back-edges */}
          {hasTopology && topology && topology.backEdges.size > 0 && (
            <div className="mt-1.5 flex items-center gap-1 text-xs text-muted-foreground/50">
              <svg className="h-2 w-4" viewBox="0 0 16 8" fill="none">
                <path
                  d="M1 1 Q8 7 15 1"
                  stroke="currentColor"
                  strokeWidth="1.2"
                  strokeDasharray="2 2"
                  strokeLinecap="round"
                />
              </svg>
              <span>back-edge (cycle)</span>
            </div>
          )}
        </CollapsibleContent>
      )}
    </Collapsible>
  );
}

// ── Utility ───────────────────────────────────────────────────────────────

function parseRawStatus(raw: unknown): string {
  if (typeof raw === "string") return raw;
  if (typeof raw === "object" && raw !== null) {
    const s = (raw as Record<string, unknown>).status;
    if (typeof s === "string") return s;
  }
  return "pending";
}
