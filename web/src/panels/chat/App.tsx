import {
  useEffect,
  useRef,
  useCallback,
  useState,
  type FormEvent,
  type KeyboardEvent,
} from "react";
import { bridge } from "@/bridge";
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
  loadSelectedAgent,
  persistSelectedAgent,
  loadChatMode,
  persistChatMode,
  type ChatMode,
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
    } else if (n.type === "agent") {
      // FlowEditor "agent" canvas nodes map to plain LLM conversation nodes.
      // agent_kind: "worker" → helpful assistant; "reviewer" → reviewer persona.
      const kind = (cfg.agent_kind as string) || "worker";
      const system =
        (cfg.system as string) ||
        (kind === "reviewer"
          ? "You are a careful reviewer. Evaluate the previous output and give concise feedback."
          : "You are a helpful assistant.");
      g.addLLMNode(id, { system, model: (cfg.model as string) || "" });
    }
  });

  return g;
}

function buildDefaultGraph(): AgentGraphInstance {
  const g = new window.AgentGraph();
  g.addLLMNode("llm", { system: "You are a helpful assistant." });
  return g;
}

// Build a single-LLM-node graph from a registered agent definition.
// Used by both "agent" mode and by @-mention routing in "flow" mode.
async function buildGraphFromAgent(name: string): Promise<AgentGraphInstance> {
  const def = await bridge.send("agent.registry.load", { name });
  const g = new window.AgentGraph();
  g.addLLMNode("agent", {
    system: def.system_prompt || "You are a helpful assistant.",
    model: def.llm || "",
  });
  return g;
}

// Look up the lead agent for a flow: by convention the node with the
// smallest id (== the first one created, == the seeded "Chat" node for
// the default flow). Returns the referenced agent_name from that node.
function leadAgentOfFlow(flowName: string): string {
  const spec = loadSavedGraph(flowName);
  if (!spec || !spec.nodes.length) return "";
  const lead = spec.nodes
    .slice()
    .sort((a, b) => Number(a.id) - Number(b.id))[0];
  const cfg = (lead?.config ?? {}) as Record<string, unknown>;
  return (cfg.agent_name as string) || lead?.type || "";
}

// Parse a leading @-mention from the input. Returns { agent, body }.
// Agent matching is case-insensitive and against the supplied catalog.
function parseMention(
  text: string,
  agents: string[],
): { agent: string | null; body: string } {
  const m = text.match(/^@([A-Za-z0-9_.-]+)\s*(.*)$/s);
  if (!m) return { agent: null, body: text };
  const want = m[1]!.toLowerCase();
  const hit = agents.find((a) => a.toLowerCase() === want);
  return hit ? { agent: hit, body: m[2] ?? "" } : { agent: null, body: text };
}

// ── components ────────────────────────────────────────────────────────────

function MessageView({ message }: { message: Message }) {
  if (message.role === "trace") {
    return (
      <div className="py-1 font-mono text-[11px] text-cronymax-fg-muted whitespace-pre-wrap">
        {message.content}
      </div>
    );
  }
  if (message.role === "system") {
    return (
      <div className="py-1 text-xs italic text-red-400">{message.content}</div>
    );
  }
  const isUser = message.role === "user";
  const label = isUser
    ? "You"
    : message.agentName
      ? message.agentName
      : "Assistant";
  return (
    <div
      className={
        "border-l-2 py-2 pl-3 " +
        (isUser ? "border-cronymax-accent" : "border-cronymax-border")
      }
    >
      <div
        className={
          "mb-1 text-[10px] font-semibold uppercase tracking-wide " +
          (isUser ? "text-cronymax-accent" : "text-cronymax-fg-muted")
        }
      >
        {label}
      </div>
      <div className="whitespace-pre-wrap break-words text-sm text-cronymax-fg">
        {message.content}
      </div>
    </div>
  );
}

export function App() {
  const [state, dispatch] = useStore();
  const inputRef = useRef<HTMLTextAreaElement>(null);
  const messagesRef = useRef<HTMLDivElement>(null);
  const [agentLoadError, setAgentLoadError] = useState<string | null>(null);

  // Refresh agent catalog from the bridge. If the registry is empty (fresh
  // install / no Space agents yet), seed a default "Chat" agent so p2p
  // mode always has something to talk to.
  const refreshAgents = useCallback(async () => {
    try {
      let res = await bridge.send("agent.registry.list");
      let names = (res.agents ?? []).map((a) => a.name);
      if (names.length === 0) {
        await bridge.send("agent.registry.save", {
          name: "Chat",
          kind: "worker",
          llm: "",
          system_prompt: "You are a helpful assistant.",
          memory_namespace: "",
          tools_csv: "",
        });
        res = await bridge.send("agent.registry.list");
        names = (res.agents ?? []).map((a) => a.name);
      }
      const selected = loadSelectedAgent(names);
      dispatch({
        type: "setAgents",
        agents: res.agents ?? [],
        selected,
      });
      setAgentLoadError(null);
    } catch (err) {
      setAgentLoadError((err as Error).message);
    }
  }, [dispatch]);

  // ── init: load chat + flows + agents ──────────────────────────────────
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
    dispatch({ type: "setChatMode", mode: loadChatMode() });
    void refreshAgents();

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
  }, [dispatch, refreshAgents]);

  // ── auto scroll ────────────────────────────────────────────────────────
  useEffect(() => {
    const el = messagesRef.current;
    if (el) el.scrollTop = el.scrollHeight;
  }, [state.messages]);

  // ── send / run ─────────────────────────────────────────────────────────
  const onRun = useCallback(
    async (rawText: string) => {
      if (state.running || !state.activeChatId) return;
      const id = state.activeChatId;
      dispatch({ type: "setRunning", running: true });

      let speaker = "";
      let body = rawText;
      const agentNames = state.agents.map((a) => a.name);
      if (state.chatMode === "agent") {
        speaker = state.selectedAgent;
      } else {
        const parsed = parseMention(rawText, agentNames);
        if (parsed.agent) {
          speaker = parsed.agent;
          body = parsed.body;
        } else {
          speaker = leadAgentOfFlow(state.selectedFlow) || agentNames[0] || "";
        }
      }

      const newHistory: Array<{
        role: Message["role"];
        content: string;
        agentName?: string;
      }> = state.messages.map((m) => ({
        role: m.role,
        content: m.content,
        ...(m.agentName ? { agentName: m.agentName } : {}),
      }));
      newHistory.push({ role: "user", content: rawText });
      dispatch({ type: "addMessage", role: "user", content: rawText });

      const traceMsgId = state.msgSeq + 1;
      dispatch({
        type: "addMessage",
        role: "trace",
        content: speaker
          ? `▶ ${speaker} (${state.chatMode}) running…`
          : "▶ running…",
      });

      let assistantMsgId: number | null = null;
      let assistantText = "";

      let graph: AgentGraphInstance;
      try {
        // Load provider credentials directly via the typed bridge and apply
        // them to the shared llmClient before building the graph. This is
        // more reliable than calling window.llmClient.loadConfig() which
        // goes through window.aiDesktop and has had JSON double-parse issues.
        try {
          const provRes = await bridge.send("llm.providers.get");
          const providers = JSON.parse(provRes.raw || "[]") as Array<{
            id: string;
            base_url?: string;
            api_key?: string;
            default_model?: string;
          }>;
          const active =
            providers.find((p) => p.id === provRes.active_id) || providers[0];
          if (active) {
            window.llmClient.baseUrl = active.base_url ?? "";
            window.llmClient.apiKey = active.api_key ?? "";
            if (active.default_model)
              window.llmClient.model = active.default_model;
          }
        } catch {
          // non-fatal — llmClient keeps whatever creds it had
        }
        if (speaker && agentNames.includes(speaker)) {
          graph = await buildGraphFromAgent(speaker);
        } else if (state.chatMode === "flow" && state.selectedFlow) {
          const spec = loadSavedGraph(state.selectedFlow);
          graph = spec ? buildGraphFromSpec(spec) : buildDefaultGraph();
        } else {
          graph = buildDefaultGraph();
        }
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
            assistantMsgId = state.msgSeq + 2;
            dispatch({
              type: "addMessage",
              role: "assistant",
              content: "",
              agentName: speaker || undefined,
            });
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
        const result = await graph.run({ task: body });
        if (assistantMsgId === null) {
          assistantText = result.output || "(no output)";
          dispatch({
            type: "addMessage",
            role: "assistant",
            content: assistantText,
            agentName: speaker || undefined,
          });
        }
        newHistory.push({
          role: "assistant",
          content: assistantText,
          ...(speaker ? { agentName: speaker } : {}),
        });
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
      state.selectedAgent,
      state.chatMode,
      state.agents,
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

        {/* Mode toggle: Agent (p2p) vs Flow (multi-agent room) */}
        <div className="flex overflow-hidden rounded border border-cronymax-border text-xs">
          {(
            [
              { id: "agent", label: "Agent" },
              { id: "flow", label: "Flow" },
            ] as const
          ).map((m) => (
            <button
              key={m.id}
              type="button"
              onClick={() => {
                const mode = m.id as ChatMode;
                dispatch({ type: "setChatMode", mode });
                persistChatMode(mode);
              }}
              className={
                "px-2 py-0.5 transition " +
                (state.chatMode === m.id
                  ? "bg-cronymax-accent text-white"
                  : "text-cronymax-fg-muted hover:text-cronymax-fg")
              }
            >
              {m.label}
            </button>
          ))}
        </div>

        {state.chatMode === "agent" ? (
          <label className="flex items-center gap-1 text-xs text-cronymax-fg-muted">
            Agent:
            <select
              value={state.selectedAgent}
              onChange={(e) => {
                dispatch({ type: "setSelectedAgent", name: e.target.value });
                persistSelectedAgent(e.target.value);
              }}
              className="rounded border border-cronymax-border bg-cronymax-surface px-1.5 py-0.5 text-xs text-cronymax-fg"
            >
              {state.agents.length === 0 && (
                <option value="">(no agents)</option>
              )}
              {state.agents.map((a) => (
                <option key={a.name} value={a.name}>
                  {a.name}
                </option>
              ))}
            </select>
          </label>
        ) : (
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
              {state.flows.length === 0 && <option value="">(no flows)</option>}
              {state.flows.map((n) => (
                <option key={n} value={n}>
                  {n}
                </option>
              ))}
            </select>
          </label>
        )}

        <button
          type="button"
          onClick={onClear}
          className="rounded border border-cronymax-border bg-cronymax-surface px-2 py-0.5 text-xs text-cronymax-fg hover:bg-cronymax-surface-2"
        >
          Clear
        </button>
      </header>

      {agentLoadError && (
        <div className="border-b border-red-500/40 bg-red-500/10 px-3 py-1 text-[11px] text-red-300">
          agent.registry.list failed: {agentLoadError}
        </div>
      )}

      <div
        ref={messagesRef}
        className="flex-1 overflow-y-auto divide-y divide-cronymax-border px-4 py-2"
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
          placeholder={
            state.chatMode === "flow"
              ? "Send a message… (use @AgentName to address one)"
              : "Send a message…"
          }
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
