/**
 * Agent panel — ReAct runner for the terminal "Explain / Fix / Retry" flow.
 *
 * All configuration (Providers, Agents, Workspace, Flows) has moved to the
 * Settings panel (opened via the title-bar gear icon). This panel is kept as
 * a standalone tab so that the runner is always accessible without opening a
 * popover.
 */
import { useEffect, useCallback, useRef, type KeyboardEvent } from "react";
import { bridge } from "@/bridge";
import { useBridgeEvent } from "@/hooks/useBridgeEvent";
import type {
  AgentGraphInstance,
  AgentTraceDetail,
  AgentRunSnapshot,
} from "@/agent_runtime";
import { useStore, type PermissionRequest } from "./store";

// ── ReAct graph builder ───────────────────────────────────────────────────
function buildReActGraph(maxIters: number): AgentGraphInstance {
  const g = new window.AgentGraph();
  g.addLLMNode("llm", {
    system:
      "You are a helpful agent. Use tools when necessary. Reply with clear, concise text.",
  });
  g.addToolNode("tool", {});
  g.addConditionNode("cond", (run: AgentRunSnapshot) => {
    const has =
      (Array.isArray(run.tool_calls) && run.tool_calls.length > 0) ||
      run.finish_reason === "tool_calls";
    return has ? "llm" : null;
  });
  void maxIters;
  return g;
}

// ── SpaceRow ──────────────────────────────────────────────────────────────
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
          ? "bg-cronymax-surface-2 text-cronymax-fg"
          : "text-cronymax-fg-muted hover:bg-cronymax-surface-2 hover:text-cronymax-fg")
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
      >
        ×
      </button>
    </li>
  );
}

// ── Permission overlay ────────────────────────────────────────────────────
function PermissionOverlay({
  perm,
  onResolve,
}: {
  perm: PermissionRequest;
  onResolve: (allow: boolean) => void;
}) {
  return (
    <div className="absolute inset-0 z-50 flex items-center justify-center bg-black/40">
      <div className="w-[340px] rounded-md border border-cronymax-border bg-cronymax-surface-2 p-4 text-sm text-cronymax-fg shadow-lg">
        <p className="mb-3 whitespace-pre-wrap">{perm.prompt}</p>
        <div className="flex justify-end gap-2">
          <button
            type="button"
            onClick={() => onResolve(true)}
            className="rounded bg-cronymax-accent px-3 py-1 text-xs font-medium text-white hover:opacity-90"
          >
            Allow
          </button>
          <button
            type="button"
            onClick={() => onResolve(false)}
            className="rounded border border-cronymax-border bg-cronymax-surface px-3 py-1 text-xs text-cronymax-fg hover:bg-cronymax-surface-2"
          >
            Deny
          </button>
        </div>
      </div>
    </div>
  );
}

// ── App ───────────────────────────────────────────────────────────────────
export function App() {
  const [state, dispatch] = useStore();
  const taskRef = useRef<HTMLTextAreaElement>(null);

  // Load LLM config once on mount.
  useEffect(() => {
    void (async () => {
      try {
        await window.llmClient.loadConfig();
        dispatch({
          type: "setLlmConfig",
          baseUrl: window.llmClient.baseUrl,
          apiKey: window.llmClient.apiKey,
          model: window.llmClient.model,
        });
      } catch {
        /* ignore */
      }
    })();
  }, [dispatch]);

  // Permission gate.
  useEffect(() => {
    window.__getPermission = (prompt: string, requestId: string) =>
      new Promise<boolean>((resolve) => {
        dispatch({
          type: "requestPermission",
          req: { prompt, requestId, resolve },
        });
      });
    return () => {
      window.__getPermission = undefined;
    };
  }, [dispatch]);

  const onResolvePermission = useCallback(
    (allow: boolean) => {
      const perm = state.permission;
      if (!perm) return;
      if (perm.requestId) {
        bridge
          .send("permission.respond", {
            request_id: perm.requestId,
            decision: allow ? "allow" : "deny",
          })
          .catch(() => undefined);
      }
      perm.resolve?.(allow);
      dispatch({ type: "clearPermission" });
    },
    [state.permission, dispatch],
  );

  // Spaces
  const loadSpaces = useCallback(async () => {
    try {
      const spaces = await bridge.send("space.list");
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
        await bridge.send("space.switch", { space_id: id });
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
        await bridge.send("space.delete", { space_id: id });
        await loadSpaces();
      } catch (e) {
        console.warn("space.delete failed", e);
      }
    },
    [loadSpaces],
  );

  const newSpace = useCallback(async () => {
    // eslint-disable-next-line no-alert
    const name = prompt("Space name:");
    if (!name) return;
    // eslint-disable-next-line no-alert
    const root = prompt("Root path:", "/");
    if (!root) return;
    try {
      await bridge.send("space.create", { name, root_path: root });
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
    const graph = buildReActGraph(10);
    graph.addEventListener("trace", (e) => {
      const d: AgentTraceDetail = e.detail;
      if (d.type === "llm_delta" && d.content) {
        dispatch({ type: "appendResult", chunk: d.content });
      } else if (d.type === "tool_start") {
        dispatch({ type: "appendResult", chunk: `\n[tool: ${d.tool}]\n` });
      } else if (d.type === "tool_done" && d.output) {
        dispatch({ type: "appendResult", chunk: d.output + "\n" });
      } else if (d.type === "error") {
        dispatch({
          type: "appendResult",
          chunk: `\n[error] ${d.message ?? ""}\n`,
        });
        dispatch({ type: "setStatus", status: "failed" });
      } else if (d.type === "human_request" && d.prompt) {
        void window.__getPermission?.(d.prompt, d.request_id ?? "");
      } else if (d.type === "done") {
        dispatch({ type: "setStatus", status: "done" });
      }
    });
    try {
      await graph.run({ task: text, getPermission: window.__getPermission });
    } catch (err) {
      dispatch({ type: "appendResult", chunk: "\n" + (err as Error).message });
      dispatch({ type: "setStatus", status: "failed" });
    }
  }, [state.task, dispatch]);

  // Inbound: terminal "Explain/Fix/Retry" → fill task and run.
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

  const openSettings = useCallback(() => {
    bridge.send("shell.settings_popover_open").catch(() => {});
  }, []);

  return (
    <main className="relative flex h-screen flex-col bg-cronymax-surface text-cronymax-fg">
      <header className="flex items-center gap-3 border-b border-cronymax-border bg-cronymax-surface-2 px-3 py-1.5">
        <h1 className="flex-1 text-sm font-semibold">Runner</h1>
        <span
          className={
            "rounded px-2 py-0.5 text-xs " +
            (state.status === "running"
              ? "bg-yellow-500/20 text-yellow-300"
              : state.status === "done"
                ? "bg-green-500/20 text-green-300"
                : state.status === "failed"
                  ? "bg-red-500/20 text-red-300"
                  : "bg-cronymax-surface text-cronymax-fg-muted")
          }
        >
          {state.status}
        </span>
        <button
          type="button"
          onClick={openSettings}
          className="rounded border border-cronymax-border bg-cronymax-surface px-2 py-0.5 text-xs hover:bg-cronymax-surface-2"
          title="Open Settings"
        >
          ⚙
        </button>
      </header>

      <section className="border-b border-cronymax-border px-3 py-2">
        <div className="mb-1 flex items-center justify-between text-xs text-cronymax-fg-muted">
          <span>Spaces</span>
          <button
            type="button"
            onClick={() => void newSpace()}
            className="rounded bg-cronymax-surface px-1.5 text-cronymax-fg hover:bg-cronymax-surface-2"
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
        className="m-3 min-h-[80px] resize-y rounded border border-cronymax-border bg-cronymax-surface-2 p-2 text-sm text-cronymax-fg outline-none focus:border-cronymax-accent"
      />
      <div className="flex justify-end gap-2 px-3">
        <button
          type="button"
          onClick={() => void runTask()}
          disabled={state.status === "running"}
          className="rounded bg-cronymax-accent px-3 py-1 text-sm font-medium text-white transition hover:opacity-90 disabled:cursor-not-allowed disabled:opacity-50"
        >
          Run
        </button>
      </div>
      <pre className="m-3 flex-1 overflow-auto whitespace-pre-wrap break-words rounded border border-cronymax-border bg-cronymax-surface-2 p-2 text-xs text-cronymax-fg">
        {state.result}
      </pre>

      {state.permission && (
        <PermissionOverlay
          perm={state.permission}
          onResolve={onResolvePermission}
        />
      )}
    </main>
  );
}
