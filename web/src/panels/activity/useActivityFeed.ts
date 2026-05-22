import { useCallback, useEffect, useMemo, useReducer } from "react";
import { runtime, shells } from "@/shells/bridge";

// ── Types ─────────────────────────────────────────────────────────────────

export interface RunEntry {
  id: string;
  space_id: string;
  session_id: string | null;
  flow_run_id: string | null;
  agent_id: string | null;
  parent_run_id: string | null;
  status: string;
  created_at_ms: number;
  updated_at_ms: number;
  // Stats from assistant_turn traces
  turn_count: number;
  input_tokens: number;
  output_tokens: number;
  total_duration_ms: number;
  // Pending review (set when status = awaiting_review)
  pending_review_id: string | null;
  // Outcome fields (task 16.2)
  goal: string | null;
  produces_count: number;
  file_change_additions: number;
  file_change_deletions: number;
}

export interface ReviewEntry {
  id: string;
  run_id: string;
  request: {
    tool_name?: string;
    arguments?: unknown;
  };
  state: string; // "pending" | "approved" | "rejected" | "deferred"
}

export interface ActivityState {
  runs: Map<string, RunEntry>;
  reviews: Map<string, ReviewEntry>;
  activeSpaceId: string | null;
}

// ── Reducer ───────────────────────────────────────────────────────────────

type Action =
  | { type: "SET_SPACE"; spaceId: string }
  | { type: "HYDRATE"; runs: RunEntry[]; reviews: ReviewEntry[] }
  | { type: "UPSERT_RUN"; run: RunEntry }
  | { type: "UPDATE_RUN_STATUS"; runId: string; status: string }
  | {
      type: "UPDATE_RUN_TRACE";
      runId: string;
      turns: number;
      inputTokens: number;
      outputTokens: number;
      durationMs: number;
    }
  | { type: "UPSERT_REVIEW"; review: ReviewEntry; runId: string }
  | { type: "CLEAR_REVIEW"; reviewId: string; runId: string };

function reducer(state: ActivityState, action: Action): ActivityState {
  switch (action.type) {
    case "SET_SPACE":
      return { ...state, activeSpaceId: action.spaceId };

    case "HYDRATE": {
      // Merge incoming runs with existing ones, preserving live trace stats
      // for runs already tracked (so a periodic re-hydration doesn't reset
      // turn/token counters that arrived via runtime events).
      const runs = new Map<string, RunEntry>(state.runs);
      for (const r of action.runs) {
        const existing = runs.get(r.id);
        if (existing) {
          // Keep live stats; only update fields that come from the snapshot.
          runs.set(r.id, {
            ...r,
            turn_count: existing.turn_count,
            input_tokens: existing.input_tokens,
            output_tokens: existing.output_tokens,
            total_duration_ms: existing.total_duration_ms,
            pending_review_id: existing.pending_review_id,
            // Outcome fields: prefer fresh snapshot data but keep existing if new is empty
            goal: r.goal ?? existing.goal,
            produces_count: r.produces_count > 0 ? r.produces_count : existing.produces_count,
            file_change_additions:
              r.file_change_additions > 0 ? r.file_change_additions : existing.file_change_additions,
            file_change_deletions:
              r.file_change_deletions > 0 ? r.file_change_deletions : existing.file_change_deletions,
          });
        } else {
          runs.set(r.id, r);
        }
      }
      const reviews = new Map<string, ReviewEntry>(state.reviews);
      for (const rv of action.reviews) reviews.set(rv.id, rv);
      return { ...state, runs, reviews };
    }

    case "UPSERT_RUN": {
      // Insert a brand-new run discovered via periodic re-hydration.
      // Skip if we already know this run (avoid clobbering live stats).
      if (state.runs.has(action.run.id)) return state;
      const next = new Map(state.runs);
      next.set(action.run.id, action.run);
      return { ...state, runs: next };
    }

    case "UPDATE_RUN_STATUS": {
      const run = state.runs.get(action.runId);
      if (!run) return state;
      const next = new Map(state.runs);
      next.set(action.runId, { ...run, status: action.status });
      return { ...state, runs: next };
    }

    case "UPDATE_RUN_TRACE": {
      const run = state.runs.get(action.runId);
      if (!run) return state;
      const next = new Map(state.runs);
      next.set(action.runId, {
        ...run,
        turn_count: action.turns,
        input_tokens: run.input_tokens + action.inputTokens,
        output_tokens: run.output_tokens + action.outputTokens,
        total_duration_ms: run.total_duration_ms + action.durationMs,
      });
      return { ...state, runs: next };
    }

    case "UPSERT_REVIEW": {
      const nextReviews = new Map(state.reviews);
      nextReviews.set(action.review.id, action.review);
      // Tag the run with pending_review_id
      const run = state.runs.get(action.runId);
      if (run) {
        const nextRuns = new Map(state.runs);
        nextRuns.set(action.runId, { ...run, pending_review_id: action.review.id });
        return { ...state, runs: nextRuns, reviews: nextReviews };
      }
      return { ...state, reviews: nextReviews };
    }

    case "CLEAR_REVIEW": {
      const nextReviews = new Map(state.reviews);
      nextReviews.delete(action.reviewId);
      const run = state.runs.get(action.runId);
      if (run && run.pending_review_id === action.reviewId) {
        const nextRuns = new Map(state.runs);
        nextRuns.set(action.runId, { ...run, pending_review_id: null });
        return { ...state, runs: nextRuns, reviews: nextReviews };
      }
      return { ...state, reviews: nextReviews };
    }

    default:
      return state;
  }
}

// ── Helpers ───────────────────────────────────────────────────────────────

/**
 * RunStatus is serialised by Rust as an internally-tagged object,
 * e.g. `{"status": "running"}` or `{"status": "failed", "message": "…"}`.
 * Extract the status string value.
 */
function parseRunStatus(raw: unknown): string {
  if (typeof raw === "string") return raw; // defensive: plain string
  if (typeof raw === "object" && raw !== null) {
    const s = (raw as Record<string, unknown>).status;
    if (typeof s === "string") return s;
  }
  return "pending";
}

function parseRunFromSnapshot(raw: Record<string, unknown>): RunEntry {
  return {
    id: (raw.id as string) ?? "",
    space_id: (raw.space_id as string) ?? "",
    session_id: (raw.session_id as string | null) ?? null,
    flow_run_id: (raw.flow_run_id as string | null) ?? null,
    agent_id: (raw.agent_id as string | null) ?? null,
    parent_run_id: (raw.parent_run_id as string | null) ?? null,
    status: parseRunStatus(raw.status),
    created_at_ms: (raw.created_at_ms as number) ?? 0,
    updated_at_ms: (raw.updated_at_ms as number) ?? 0,
    turn_count: 0,
    input_tokens: 0,
    output_tokens: 0,
    total_duration_ms: 0,
    pending_review_id: null,
    goal: (raw.goal as string | null) ?? null,
    produces_count: Array.isArray(raw.produces) ? (raw.produces as unknown[]).length : 0,
    file_change_additions: Array.isArray(raw.file_changes)
      ? (raw.file_changes as Array<{ additions?: number }>).reduce((s, fc) => s + (fc.additions ?? 0), 0)
      : 0,
    file_change_deletions: Array.isArray(raw.file_changes)
      ? (raw.file_changes as Array<{ deletions?: number }>).reduce((s, fc) => s + (fc.deletions ?? 0), 0)
      : 0,
  };
}

function parseReviewFromSnapshot(raw: Record<string, unknown>): ReviewEntry {
  return {
    id: (raw.id as string) ?? "",
    run_id: (raw.run_id as string) ?? "",
    request: (raw.request as ReviewEntry["request"]) ?? {},
    state: (raw.state as string) ?? "pending",
  };
}

// ── Hook ──────────────────────────────────────────────────────────────────

const initialState: ActivityState = {
  runs: new Map(),
  reviews: new Map(),
  activeSpaceId: null,
};

// ── Tree types ─────────────────────────────────────────────────────────────

export interface RunTreeNode {
  run: RunEntry;
  children: RunTreeNode[];
}

export interface ActivityGroups {
  /** Root-level chat/session runs (no flow_run_id) with children nested. */
  chatRoots: RunTreeNode[];
  /** flow_run_id → root nodes for that flow run. */
  flowRoots: Map<string, RunTreeNode[]>;
  pendingCount: number;
}

function buildTree(runs: RunEntry[]): { chatRoots: RunTreeNode[]; flowRoots: Map<string, RunTreeNode[]> } {
  // 1. Create a node for every run.
  const nodeMap = new Map<string, RunTreeNode>();
  for (const run of runs) {
    nodeMap.set(run.id, { run, children: [] });
  }

  const chatRoots: RunTreeNode[] = [];
  const flowRoots = new Map<string, RunTreeNode[]>();

  // 2. Wire up parent→child relationships.
  for (const node of nodeMap.values()) {
    const { run } = node;
    const parentNode = run.parent_run_id ? nodeMap.get(run.parent_run_id) : undefined;

    if (parentNode) {
      // Has a known parent within the visible set → attach as child.
      parentNode.children.push(node);
    } else if (run.flow_run_id) {
      // Root of a flow run.
      const arr = flowRoots.get(run.flow_run_id) ?? [];
      arr.push(node);
      flowRoots.set(run.flow_run_id, arr);
    } else {
      // Root of a chat/session group.
      chatRoots.push(node);
    }
  }

  // 3. Sort children by creation time within each node.
  function sortChildren(node: RunTreeNode) {
    node.children.sort((a, b) => a.run.created_at_ms - b.run.created_at_ms);
    for (const child of node.children) sortChildren(child);
  }
  for (const node of nodeMap.values()) sortChildren(node);

  // Sort roots by creation time too.
  chatRoots.sort((a, b) => a.run.created_at_ms - b.run.created_at_ms);
  for (const arr of flowRoots.values()) {
    arr.sort((a, b) => a.run.created_at_ms - b.run.created_at_ms);
  }

  return { chatRoots, flowRoots };
}

function computeGroups(state: ActivityState, filter: "all" | "live" | "needs_review"): ActivityGroups {
  const visible: RunEntry[] = [];
  let pendingCount = 0;

  for (const run of state.runs.values()) {
    // Only show runs from the active space.
    if (state.activeSpaceId && run.space_id !== state.activeSpaceId) continue;

    if (run.status === "awaiting_review") pendingCount++;

    const show =
      filter === "all" ||
      (filter === "live" && run.status === "running") ||
      (filter === "needs_review" && run.status === "awaiting_review");

    if (!show) continue;
    visible.push(run);
  }

  const { chatRoots, flowRoots } = buildTree(visible);
  return { chatRoots, flowRoots, pendingCount };
}

/** Interval (ms) between periodic snapshot re-hydrations. */
const REFRESH_INTERVAL_MS = 30_000;

export function useActivityFeed(filter: "all" | "live" | "needs_review") {
  const [state, dispatch] = useReducer(reducer, initialState);

  // Resolve active space on mount.
  useEffect(() => {
    shells.browser.space
      .list()
      .then((spaces: Array<{ id: string; active?: boolean }>) => {
        const active = spaces.find((s) => s.active);
        if (active) dispatch({ type: "SET_SPACE", spaceId: active.id });
      })
      .catch(() => undefined);
  }, []);

  // Helper: fetch snapshot and merge into state.
  const fetchSnapshot = useCallback(() => {
    shells.browser.activity
      .snapshot()
      .then((resp: { runs?: unknown[]; pending_reviews?: unknown[] }) => {
        const runs = (resp.runs ?? []).map((r) => parseRunFromSnapshot(r as Record<string, unknown>));
        const reviews = (resp.pending_reviews ?? []).map((r) => parseReviewFromSnapshot(r as Record<string, unknown>));
        dispatch({ type: "HYDRATE", runs, reviews });
      })
      .catch(() => undefined);
  }, []);

  // Initial hydration once we have the space ID.
  useEffect(() => {
    if (!state.activeSpaceId) return;
    fetchSnapshot();
  }, [state.activeSpaceId, fetchSnapshot]);

  // Periodic re-hydration to pick up newly started runs.
  useEffect(() => {
    if (!state.activeSpaceId) return;
    const id = setInterval(fetchSnapshot, REFRESH_INTERVAL_MS);
    return () => clearInterval(id);
  }, [state.activeSpaceId, fetchSnapshot]);

  // Stable event handler for runtime events on individual run topics.
  const handleRuntimeEvent = useCallback((event: unknown) => {
    const ev = event as Record<string, unknown>;
    if (!ev) return;
    const payload = ev.payload as Record<string, unknown> | undefined;
    if (!payload) return;
    const kind = payload.kind as string | undefined;
    const runId = payload.run_id as string | undefined;
    if (!runId) return;

    if (kind === "run_status") {
      const status = payload.status as string | undefined;
      if (status) dispatch({ type: "UPDATE_RUN_STATUS", runId, status });
      return;
    }

    if (kind === "trace") {
      const trace = payload.trace as Record<string, unknown> | undefined;
      if (!trace) return;
      const traceKind = trace.kind as string | undefined;

      if (traceKind === "assistant_turn") {
        const usage = trace.usage as { input_tokens?: number; output_tokens?: number } | undefined;
        const durationMs = (trace.duration_ms as number) ?? 0;
        const turn = (trace.turn as number) ?? 0;
        dispatch({
          type: "UPDATE_RUN_TRACE",
          runId,
          turns: turn,
          inputTokens: usage?.input_tokens ?? 0,
          outputTokens: usage?.output_tokens ?? 0,
          durationMs,
        });
        return;
      }

      if (traceKind === "review_resolved") {
        const reviewId = trace.review_id as string | undefined;
        if (reviewId) dispatch({ type: "CLEAR_REVIEW", reviewId, runId });
        return;
      }
    }

    if (kind === "permission_request") {
      const reviewId = payload.review_id as string | undefined;
      const request = payload.request as ReviewEntry["request"] | undefined;
      if (reviewId) {
        const review: ReviewEntry = {
          id: reviewId,
          run_id: runId,
          request: request ?? {},
          state: "pending",
        };
        dispatch({ type: "UPSERT_REVIEW", review, runId });
      }
      return;
    }
  }, []);

  // Subscribe to run:{runId} for each tracked run for live status/trace updates.
  // Re-subscribe only when the set of run IDs changes (not on value updates).
  const runIdKey = useMemo(() => [...state.runs.keys()].sort().join(","), [state.runs]);
  useEffect(() => {
    if (state.runs.size === 0) return;
    const unsubbers: Array<() => void> = [];
    for (const runId of state.runs.keys()) {
      const off = runtime.on(`run:${runId}`, handleRuntimeEvent);
      if (off) unsubbers.push(off);
    }
    return () => {
      for (const off of unsubbers) off();
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [runIdKey, handleRuntimeEvent]);

  const groups = computeGroups(state, filter);
  return { ...groups, reviews: state.reviews };
}
