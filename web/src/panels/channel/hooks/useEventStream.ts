/**
 * useEventStream — subscribe to AppEvents over the bridge for a given scope
 * and project them into typed UI state.
 *
 * Workflow:
 *   1. On mount: call events.list (paginated newest-first → reversed) to load
 *      the recent backlog.
 *   2. Concurrently call events.subscribe (bridge multicasts every Append on
 *      channel "event"; renderer filters by scope).
 *   3. bridge.on("event", …) feeds each AppEvent into the reducer.
 *
 * The projection groups events into:
 *   - linear timeline (everything in order)
 *   - thread map keyed by doc_id (text|document_event|review_event)
 *   - run state (latest system subkind + active flag)
 *   - error banners (un-dismissed `error` events)
 */

import { useEffect, useReducer, useRef } from "react";
import { browser } from "@/shells/bridge";
import type { AppEvent } from "@/types/events";

export interface Scope {
  flow_id?: string;
  run_id?: string;
}

export interface ThreadState {
  doc_id: string;
  doc_path?: string;
  doc_type?: string;
  revision: number;
  producer?: string;
  reviews: Array<Extract<AppEvent, { kind: "review_event" }>>;
  comments: Array<Extract<AppEvent, { kind: "text" }>>;
  last_ts: number;
}

export interface RunState {
  active: boolean;
  last_subkind: string | null;
  run_id: string | null;
}

export interface StreamState {
  timeline: AppEvent[];
  threads: Map<string, ThreadState>;
  run: RunState;
  errors: Array<Extract<AppEvent, { kind: "error" }>>;
  loading: boolean;
  error: string | null;
}

const INIT: StreamState = {
  timeline: [],
  threads: new Map(),
  run: { active: false, last_subkind: null, run_id: null },
  errors: [],
  loading: true,
  error: null,
};

type Action =
  | { type: "loaded"; events: AppEvent[] }
  | { type: "appended"; event: AppEvent }
  | { type: "load_error"; message: string }
  | { type: "dismiss_error"; id: string };

function dedupePush(arr: AppEvent[], e: AppEvent): AppEvent[] {
  if (arr.some((x) => x.id === e.id)) return arr;
  return [...arr, e];
}

function applyEvent(state: StreamState, e: AppEvent): StreamState {
  const timeline = dedupePush(state.timeline, e);
  let threads = state.threads;
  let run = state.run;
  let errors = state.errors;

  if (e.kind === "document_event") {
    const next = new Map(threads);
    const prev = next.get(e.payload.doc_id);
    next.set(e.payload.doc_id, {
      doc_id: e.payload.doc_id,
      doc_path: e.payload.doc_path,
      doc_type: e.payload.doc_type,
      revision: e.payload.revision,
      producer: e.payload.producer,
      reviews: prev?.reviews ?? [],
      comments: prev?.comments ?? [],
      last_ts: e.ts_ms,
    });
    threads = next;
  } else if (e.kind === "review_event") {
    const next = new Map(threads);
    const prev = next.get(e.payload.doc_id) ?? {
      doc_id: e.payload.doc_id,
      revision: 0,
      reviews: [],
      comments: [],
      last_ts: e.ts_ms,
    };
    next.set(e.payload.doc_id, {
      ...prev,
      reviews: [...prev.reviews, e as Extract<AppEvent, { kind: "review_event" }>],
      last_ts: e.ts_ms,
    });
    threads = next;
  } else if (e.kind === "text" && e.payload.doc_id) {
    const docId = e.payload.doc_id;
    const next = new Map(threads);
    const prev = next.get(docId) ?? {
      doc_id: docId,
      revision: 0,
      reviews: [],
      comments: [],
      last_ts: e.ts_ms,
    };
    next.set(docId, {
      ...prev,
      comments: [...prev.comments, e as Extract<AppEvent, { kind: "text" }>],
      last_ts: e.ts_ms,
    });
    threads = next;
  } else if (e.kind === "system") {
    const sub = e.payload.subkind;
    const active = sub === "run_started" || sub === "run_paused";
    run = { active, last_subkind: sub, run_id: e.run_id ?? null };
  } else if (e.kind === "error") {
    errors = [...errors, e as Extract<AppEvent, { kind: "error" }>];
  }

  return { ...state, timeline, threads, run, errors };
}

function reducer(state: StreamState, action: Action): StreamState {
  switch (action.type) {
    case "loaded": {
      let next = { ...INIT, loading: false };
      for (const e of action.events) next = applyEvent(next, e);
      return next;
    }
    case "appended":
      return applyEvent(state, action.event);
    case "load_error":
      return { ...state, loading: false, error: action.message };
    case "dismiss_error":
      return {
        ...state,
        errors: state.errors.filter((e) => e.id !== action.id),
      };
  }
}

function inScope(e: AppEvent, scope: Scope): boolean {
  if (scope.flow_id && e.flow_id !== scope.flow_id) return false;
  if (scope.run_id && e.run_id !== scope.run_id) return false;
  return true;
}

export function useEventStream(scope: Scope) {
  const [state, dispatch] = useReducer(reducer, INIT);
  const scopeRef = useRef(scope);
  scopeRef.current = scope;

  useEffect(() => {
    let cancelled = false;

    (async () => {
      try {
        const res = await browser.send("events.list", {
          flow_id: scope.flow_id,
          run_id: scope.run_id,
          limit: 200,
        });
        if (cancelled) return;
        // Bridge returns newest-first; fold oldest-first into UI.
        const ordered = [...res.events].reverse();
        dispatch({ type: "loaded", events: ordered });
      } catch (err) {
        if (!cancelled) {
          dispatch({
            type: "load_error",
            message: err instanceof Error ? err.message : String(err),
          });
        }
      }

      try {
        await browser.send("events.subscribe", {
          flow_id: scope.flow_id,
          run_id: scope.run_id,
        });
      } catch {
        // best-effort; broadcasts may still arrive
      }
    })();

    const off = browser.on("event", (payload) => {
      const e = payload as AppEvent;
      if (!inScope(e, scopeRef.current)) return;
      dispatch({ type: "appended", event: e });
    });

    return () => {
      cancelled = true;
      off();
    };
  }, [scope.flow_id, scope.run_id]);

  return {
    state,
    dismissError: (id: string) => dispatch({ type: "dismiss_error", id }),
  };
}
