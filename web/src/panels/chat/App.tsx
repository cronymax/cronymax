import {
  useEffect,
  useRef,
  useCallback,
  type FormEvent,
  type KeyboardEvent,
} from "react";
import type { AgentGraphInstance, AgentTraceDetail } from "@/agent_runtime";
import {
  useStore,
  loadHistory,
  persistHistory,
  ensureChat,
  chatNameFor,
  loadFlowsList,
  loadSavedGraph,
  persistSelectedFlow,
  type Message,
} from "./store";

interface SavedFlowSpec {
  nodes: Array<{
    id: string | number;
    type: string;
    config?: Record<string, unknown>;
  }>;
  edges?: Array<{ from_id: string | number; to_id: string | number }>;
}

function buildGraphFromSpec(spec: SavedFlowSpec): AgentGraphInstance {
  const g = new window.AgentGraph();
  const byId = new Map(spec.nodes.map((n) => [String(n.id), n]));
  const outMap = new Map<string, string[]>();
  (spec.edges || []).forEach((e) => {
    const f = String(e.from_id);
    const t = String(e.to_id);
    if (!outMap.has(f)) outMap.set(f, []);
    outMap.get(f)!.push(t);
  });

  const ordered: SavedFlowSpec["nodes"] = [];
  const seen = new Set<string>();
  const start = spec.nodes.find((n) => n.type === "start");

  function pushNode(id: string): void {
    if (seen.has(id)) return;
    seen.add(id);
    const node = byId.get(id);
    if (!node || node.type === "start") return;
    ordered.push(node);
    (outMap.get(id) || []).forEach(pushNode);
  }

  if (start) (outMap.get(String(start.id)) || []).forEach(pushNode);
  spec.nodes
    .filter((n) => n.type !== "start")
    .forEach((n) => {
      if (!seen.has(String(n.id))) ordered.push(n);
    });

  ordered.forEach((n) => {
    const id = String(n.id);
    const cfg = n.config ?? {};
    if (n.type === "llm") {
      g.addLLMNode(id, {
        system: (cfg.system as string) || "You are a helpful assistant.",
        model: (cfg.model as string) || "",
      });
    } else if (n.type === "tool") {
      g.addToolNode(id, {
        tool_name: (cfg.tool_name as string) || "terminal_exec",
      });
    } else if (n.type === "human") {
      g.addHumanNode(id, { prompt: (cfg.prompt as string) || "Approve?" });
    } else if (n.type === "cond") {
      g.addConditionNode(id, (run) => {
        const expr = ((cfg.condition as string) || "").trim();
        const trueNext = ((cfg.true_next as string) || "").trim();
        const falseNext = ((cfg.false_next as string) || "").trim();
        const has =
          (Array.isArray(run.tool_calls) && run.tool_calls.length > 0) ||
          run.finish_reason === "tool_calls";
        let pass = false;
        if (!expr || expr === "always") pass = true;
        else if (expr === "has_tool_call" || expr === "has_tool_calls")
          pass = has;
        else if (expr === "no_tool_call" || expr === "no_tool_calls")
          pass = !has;
        else pass = has;
        if (pass && trueNext) return trueNext;
        if (!pass && falseNext) return falseNext;
        return null;
      });
    }
  });

  return g;
}

function buildDefaultGraph(): AgentGraphInstance {
  const g = new window.AgentGraph();
  g.addLLMNode("llm", { system: "You are a helpful assistant." });
  return g;
}

// ── components ────────────────────────────────────────────────────────────

function MessageView({ message }: { message: Message }) {
  const palette: Record<Message["role"], string> = {
    user: "self-end bg-cronymax-accent text-white",
    assistant: "self-start bg-cronymax-surface-2 text-cronymax-fg",
    system: "self-center text-[#f87171] text-xs italic",
    trace:
      "self-start text-[#9ca3af] font-mono text-[11px] whitespace-pre-wrap",
  };
  return (
    <div
      className={
        "max-w-[85%] rounded-md px-3 py-1.5 text-sm whitespace-pre-wrap break-words " +
        (palette[message.role] ?? "")
      }
    >
      {message.content}
    </div>
  );
}

export function App() {
  const [state, dispatch] = useStore();
  const inputRef = useRef<HTMLTextAreaElement>(null);
  const messagesRef = useRef<HTMLDivElement>(null);

  // ── init: load chat + flows ───────────────────────────────────────────
  useEffect(() => {
    const { id, name } = ensureChat();
    dispatch({
      type: "loadChat",
      id,
      name,
      history: loadHistory(id),
    });
    const { flows, selected } = loadFlowsList();
    dispatch({ type: "setFlows", flows, selected });

    // React to flow editor / chat-list updates from other tabs.
    const onStorage = (e: StorageEvent) => {
      if (e.key === "flows" || e.key === "active_flow") {
        const refreshed = loadFlowsList();
        dispatch({
          type: "setFlows",
          flows: refreshed.flows,
          selected: refreshed.selected,
        });
      }
      if (e.key === "chats") {
        // Title may have changed.
        if (state.activeChatId) {
          dispatch({
            type: "loadChat",
            id: state.activeChatId,
            name: chatNameFor(state.activeChatId),
            history: loadHistory(state.activeChatId),
          });
        }
      }
    };
    window.addEventListener("storage", onStorage);
    return () => window.removeEventListener("storage", onStorage);
  }, [dispatch]);

  // ── auto scroll ────────────────────────────────────────────────────────
  useEffect(() => {
    const el = messagesRef.current;
    if (el) el.scrollTop = el.scrollHeight;
  }, [state.messages]);

  // ── send / run ─────────────────────────────────────────────────────────
  const onRun = useCallback(
    async (text: string) => {
      if (state.running || !state.activeChatId) return;
      const id = state.activeChatId;
      dispatch({ type: "setRunning", running: true });

      const newHistory: Array<{ role: Message["role"]; content: string }> =
        state.messages.map((m) => ({ role: m.role, content: m.content }));
      newHistory.push({ role: "user", content: text });
      dispatch({ type: "addMessage", role: "user", content: text });

      const traceMsgId = state.msgSeq + 1;
      dispatch({
        type: "addMessage",
        role: "trace",
        content: "▶ running graph…",
      });

      let assistantMsgId: number | null = null;
      let assistantText = "";

      let graph: AgentGraphInstance;
      try {
        const spec = loadSavedGraph(state.selectedFlow);
        graph = spec ? buildGraphFromSpec(spec) : buildDefaultGraph();
      } catch (err) {
        dispatch({
          type: "addMessage",
          role: "system",
          content: "Failed to build graph: " + (err as Error).message,
        });
        dispatch({ type: "setRunning", running: false });
        return;
      }

      graph.addEventListener("trace", (e) => {
        const d: AgentTraceDetail = e.detail;
        if (d.type === "llm_delta" && typeof d.content === "string") {
          if (assistantMsgId === null) {
            // We must allocate via dispatch; do it first.
            assistantMsgId = state.msgSeq + 2;
            dispatch({ type: "addMessage", role: "assistant", content: "" });
          }
          assistantText += d.content;
          dispatch({
            type: "updateMessage",
            id: assistantMsgId,
            content: assistantText,
          });
        } else if (d.type === "node_enter") {
          dispatch({
            type: "appendToMessage",
            id: traceMsgId,
            chunk: `\n→ ${d.node_type ?? "?"} ${d.node_id ?? ""}`,
          });
        } else if (d.type === "error") {
          dispatch({
            type: "appendToMessage",
            id: traceMsgId,
            chunk: `\n✗ ${d.message ?? ""}`,
          });
        }
      });

      try {
        const result = await graph.run({ task: text });
        if (assistantMsgId === null) {
          assistantText = result.output || "(no output)";
          dispatch({
            type: "addMessage",
            role: "assistant",
            content: assistantText,
          });
        }
        newHistory.push({ role: "assistant", content: assistantText });
        persistHistory(id, newHistory);
        dispatch({
          type: "appendToMessage",
          id: traceMsgId,
          chunk: "\n✓ done",
        });
      } catch (err) {
        dispatch({
          type: "addMessage",
          role: "system",
          content: "Error: " + ((err as Error)?.message || String(err)),
        });
      } finally {
        dispatch({ type: "setRunning", running: false });
        inputRef.current?.focus();
      }
    },
    [
      state.running,
      state.activeChatId,
      state.messages,
      state.msgSeq,
      state.selectedFlow,
      dispatch,
    ],
  );

  const onSubmit = useCallback(
    (e: FormEvent<HTMLFormElement>) => {
      e.preventDefault();
      const v = inputRef.current?.value.trim() || "";
      if (!v) return;
      if (inputRef.current) inputRef.current.value = "";
      void onRun(v);
    },
    [onRun],
  );

  const onKeyDown = useCallback((e: KeyboardEvent<HTMLTextAreaElement>) => {
    if (e.key === "Enter" && !e.shiftKey) {
      e.preventDefault();
      e.currentTarget.form?.requestSubmit();
    }
  }, []);

  const onClear = useCallback(() => {
    dispatch({ type: "clearHistory" });
    if (state.activeChatId) persistHistory(state.activeChatId, []);
  }, [dispatch, state.activeChatId]);

  return (
    <main className="flex h-screen flex-col bg-cronymax-surface text-cronymax-fg">
      <header className="flex items-center gap-3 border-b border-cronymax-border bg-cronymax-surface-2 px-3 py-2 text-sm">
        <span className="flex-1 truncate font-semibold">{state.chatName}</span>
        <label className="flex items-center gap-1 text-xs text-cronymax-fg-muted">
          Flow:
          <select
            value={state.selectedFlow}
            onChange={(e) => {
              dispatch({ type: "setSelectedFlow", name: e.target.value });
              persistSelectedFlow(e.target.value);
            }}
            className="rounded border border-cronymax-border bg-cronymax-surface px-1.5 py-0.5 text-xs text-cronymax-fg"
          >
            <option value="">(default ReAct)</option>
            {state.flows.map((n) => (
              <option key={n} value={n}>
                {n}
              </option>
            ))}
          </select>
        </label>
        <button
          type="button"
          onClick={onClear}
          className="rounded border border-cronymax-border bg-cronymax-surface px-2 py-0.5 text-xs text-cronymax-fg hover:bg-cronymax-surface-2"
        >
          Clear
        </button>
      </header>

      <div
        ref={messagesRef}
        className="flex flex-1 flex-col gap-1.5 overflow-y-auto px-3 py-2"
      >
        {state.messages.map((m) => (
          <MessageView key={m.id} message={m} />
        ))}
      </div>

      <form
        onSubmit={onSubmit}
        className="flex gap-2 border-t border-cronymax-border bg-cronymax-surface-2 p-2"
      >
        <textarea
          ref={inputRef}
          rows={2}
          autoFocus
          placeholder="Send a message…"
          onKeyDown={onKeyDown}
          className="flex-1 resize-none rounded border border-cronymax-border bg-cronymax-surface px-2 py-1.5 text-sm text-cronymax-fg outline-none focus:border-cronymax-accent"
        />
        <button
          type="submit"
          disabled={state.running}
          className="rounded bg-cronymax-accent px-3 py-1.5 text-sm font-medium text-white transition hover:opacity-90 disabled:cursor-not-allowed disabled:opacity-50"
        >
          {state.running ? "…" : "Send"}
        </button>
      </form>
    </main>
  );
}
