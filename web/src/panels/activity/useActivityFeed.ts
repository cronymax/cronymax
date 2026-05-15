import { useEffect, useReducer } from "react";
import { runtime, shells } from "@/shells/bridge";

// ── Types ─────────────────────────────────────────────────────────────────

export interface RunEntry {
  id: string;
  space_id: string;
  session_id: string | null;
  flow_run_id: string | null;
  agent_id: string | null;
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
      const runs = new Map<string, RunEntry>();
      for (const r of action.runs) runs.set(r.id, r);
      const reviews = new Map<string, ReviewEntry>();
      for (const rv of action.reviews) reviews.set(rv.id, rv);
      return { ...state, runs, reviews };
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
    status: parseRunStatus(raw.status),
    created_at_ms: (raw.created_at_ms as number) ?? 0,
    updated_at_ms: (raw.updated_at_ms as number) ?? 0,
    turn_count: 0,
    input_tokens: 0,
    output_tokens: 0,
    total_duration_ms: 0,
    pending_review_id: null,
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

export interface ActivityGroups {
  chatGroups: Map<string, RunEntry[]>; // session_id → runs
  flowGroups: Map<string, RunEntry[]>; // flow_run_id → runs
  pendingCount: number;
}

function computeGroups(state: ActivityState, filter: "all" | "live" | "needs_review"): ActivityGroups {
  const chatGroups = new Map<string, RunEntry[]>();
  const flowGroups = new Map<string, RunEntry[]>();
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

    if (run.flow_run_id) {
      const arr = flowGroups.get(run.flow_run_id) ?? [];
      arr.push(run);
      flowGroups.set(run.flow_run_id, arr);
    } else {
      const key = run.session_id ?? run.id;
      const arr = chatGroups.get(key) ?? [];
      arr.push(run);
      chatGroups.set(key, arr);
    }
  }

  return { chatGroups, flowGroups, pendingCount };
}

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

  // Hydrate from snapshot once we have the space ID.
  useEffect(() => {
    if (!state.activeSpaceId) return;
    shells.browser.activity
      .snapshot()
      .then((resp: { runs?: unknown[]; pending_reviews?: unknown[] }) => {
        const runs = (resp.runs ?? []).map((r) => parseRunFromSnapshot(r as Record<string, unknown>));
        const reviews = (resp.pending_reviews ?? []).map((r) => parseReviewFromSnapshot(r as Record<string, unknown>));
        dispatch({ type: "HYDRATE", runs, reviews });
      })
      .catch(() => undefined);
  }, [state.activeSpaceId]);

  // Subscribe to all runtime events for live updates.
  useEffect(() => {
    const unsub = runtime.on("*", (event: unknown) => {
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
    });

    return () => {
      if (unsub) unsub();
    };
  }, []);

  const groups = computeGroups(state, filter);
  return { ...groups, reviews: state.reviews };
}
