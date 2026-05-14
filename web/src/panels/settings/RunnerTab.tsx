import { type KeyboardEvent, useCallback, useEffect, useRef } from "react";
import { Icon } from "../../components/Icon";
import { useBridgeEvent } from "../../hooks/useBridgeEvent";
import { browser, shells } from "../../shells/bridge";
import { agentRun } from "../../shells/runtime";
import { useStore } from "./store";

// ── Runner tab ────────────────────────────────────────────────────────────
// ── Runner tab (was here) ──────────────────────────────────────────────────
function SpaceRow({
  space,
  active,
  onActivate,
  onDelete,
}: {
  space: { id: string; name: string };
  active: boolean;
  onActivate: () => void;
  onDelete: () => void;
}) {
  return (
    <li
      onClick={onActivate}
      className={
        "group flex h-7 cursor-pointer items-center gap-1.5 rounded px-2 text-xs " +
        (active
          ? "bg-cronymax-float text-cronymax-title"
          : "text-cronymax-caption hover:bg-cronymax-float hover:text-cronymax-title")
      }
    >
      <span className="flex-1 truncate">{space.name}</span>
      <button
        type="button"
        onClick={(e) => {
          e.stopPropagation();
          onDelete();
        }}
        className="opacity-0 transition group-hover:opacity-100"
        title="Delete space"
        aria-label="Delete space"
      >
        <Icon name="close" size={12} aria-hidden="true" />
      </button>
    </li>
  );
}
export function RunnerTab() {
  const [state, dispatch] = useStore();
  const taskRef = useRef<HTMLTextAreaElement>(null);

  const loadSpaces = useCallback(async () => {
    try {
      const spaces = await shells.browser.space.list();
      dispatch({ type: "setSpaces", spaces });
    } catch (e) {
      console.warn("space.list failed", e);
    }
  }, [dispatch]);

  useEffect(() => {
    void loadSpaces();
  }, [loadSpaces]);
  useBridgeEvent("space.created", () => void loadSpaces());
  useBridgeEvent("space.deleted", () => void loadSpaces());

  const switchSpace = useCallback(
    async (id: string) => {
      try {
        await shells.browser.space.switch({ space_id: id });
        dispatch({ type: "setActiveSpace", id });
      } catch (e) {
        console.warn("space.switch failed", e);
      }
    },
    [dispatch],
  );

  const deleteSpace = useCallback(
    async (id: string, name: string) => {
      // eslint-disable-next-line no-alert
      if (!confirm(`Delete space "${name}"?`)) return;
      try {
        await shells.browser.space.delete({ space_id: id });
        await loadSpaces();
      } catch (e) {
        console.warn("space.delete failed", e);
      }
    },
    [loadSpaces],
  );

  const newSpace = useCallback(async () => {
    // eslint-disable-next-line no-alert
    const root = prompt("Root path:", "/");
    if (!root) return;
    try {
      await shells.browser.space.create({
        root_path: root,
        profile_id: "default",
      });
      await loadSpaces();
    } catch (e) {
      console.warn("space.create failed", e);
    }
  }, [loadSpaces]);

  const runTask = useCallback(async () => {
    const text = state.task.trim();
    if (!text) {
      taskRef.current?.focus();
      return;
    }
    dispatch({ type: "setStatus", status: "running" });
    dispatch({ type: "resetResult" });

    let runId = "";
    try {
      runId = await agentRun(text);
      if (!runId) throw new Error("runtime did not return run_id");
      await shells.browser.events.subscribe({ run_id: runId }).catch(() => {
        /* ignore */
      });
    } catch (err) {
      dispatch({ type: "appendResult", chunk: `\n${(err as Error).message}` });
      dispatch({ type: "setStatus", status: "failed" });
      return;
    }

    const off = browser.on("event", (raw: unknown) => {
      const ev = raw as Record<string, unknown> | null;
      if (!ev) return;
      if (ev.tag === "event") {
        const inner = (ev.event as Record<string, unknown> | undefined) ?? {};
        const pl = (inner.payload as Record<string, unknown> | undefined) ?? {};
        const pRunId = (inner as Record<string, unknown>).run_id as string | undefined;
        if (pRunId && pRunId !== runId) return;
        const kind = pl.kind as string | undefined;
        if (kind === "token" && pl.content) {
          dispatch({ type: "appendResult", chunk: pl.content as string });
        } else if (kind === "run_status") {
          const status = pl.status as string | undefined;
          if (status === "succeeded") {
            dispatch({ type: "setStatus", status: "done" });
            off();
          } else if (status === "failed" || status === "cancelled") {
            dispatch({ type: "appendResult", chunk: `\n[${status}]` });
            dispatch({ type: "setStatus", status: "failed" });
            off();
          }
        } else if (kind === "log") {
          dispatch({
            type: "appendResult",
            chunk: `\n[log] ${pl.message ?? ""}`,
          });
        }
      }
    });
  }, [state.task, dispatch]);

  useBridgeEvent("agent.task_from_command", (data) => {
    const action = data.action || "Explain";
    const cmd = data.command || "";
    const out = (data.output || "").slice(0, 1000);
    const ec = data.exit_code ?? -1;
    const text = `${action} the following terminal command and its output.\n\nCommand: ${cmd}\nExit code: ${ec}\nOutput:\n${out}`;
    dispatch({ type: "setTask", task: text });
    setTimeout(() => void runTask(), 0);
  });

  const onTaskKeyDown = useCallback(
    (e: KeyboardEvent<HTMLTextAreaElement>) => {
      if ((e.metaKey || e.ctrlKey) && e.key === "Enter") {
        e.preventDefault();
        void runTask();
      }
    },
    [runTask],
  );

  return (
    <div className="flex h-full flex-col">
      <section className="border-b border-cronymax-border px-3 py-2">
        <div className="mb-1 flex items-center justify-between text-xs text-cronymax-caption">
          <span>Spaces</span>
          <button
            type="button"
            onClick={() => void newSpace()}
            className="rounded bg-cronymax-base px-1.5 text-cronymax-title hover:bg-cronymax-float"
          >
            +
          </button>
        </div>
        <ul className="flex flex-col gap-px">
          {state.spaces.map((sp) => (
            <SpaceRow
              key={sp.id}
              space={sp}
              active={sp.id === state.activeSpaceId}
              onActivate={() => void switchSpace(sp.id)}
              onDelete={() => void deleteSpace(sp.id, sp.name)}
            />
          ))}
        </ul>
      </section>
      <textarea
        ref={taskRef}
        value={state.task}
        onChange={(e) => dispatch({ type: "setTask", task: e.target.value })}
        onKeyDown={onTaskKeyDown}
        spellCheck={false}
        placeholder="Ask the agent…  (⌘/Ctrl+Enter to run)"
        className="m-3 min-h-[80px] resize-y rounded border border-cronymax-border bg-cronymax-float p-2 text-sm text-cronymax-title outline-none focus:border-cronymax-primary"
      />
      <div className="flex justify-end gap-2 px-3">
        <button
          type="button"
          onClick={() => void runTask()}
          disabled={state.status === "running"}
          className="rounded bg-cronymax-primary px-3 py-1 text-sm font-medium text-white transition hover:opacity-90 disabled:cursor-not-allowed disabled:opacity-50"
        >
          Run
        </button>
      </div>
      <pre className="m-3 flex-1 overflow-auto whitespace-pre-wrap break-words rounded border border-cronymax-border bg-cronymax-float p-2 text-xs text-cronymax-title">
        {state.result}
      </pre>
    </div>
  );
}
