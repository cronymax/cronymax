import {
  useEffect,
  useRef,
  useCallback,
  useState,
  type FormEvent,
  type KeyboardEvent,
} from "react";
import { Streamdown } from "streamdown";
import { bridge } from "@/bridge";
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

// ── components ────────────────────────────────────────────────────────────

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

function MessageView({
  message,
  isStreaming,
}: {
  message: Message;
  isStreaming: boolean;
}) {
  if (message.role === "trace") {
    return (
      <div className="py-1 font-mono text-[11px] text-cronymax-caption whitespace-pre-wrap">
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
        (isUser ? "border-cronymax-primary" : "border-cronymax-border")
      }
    >
      <div
        className={
          "mb-1 text-[10px] font-semibold uppercase tracking-wide " +
          (isUser ? "text-cronymax-primary" : "text-cronymax-caption")
        }
      >
        {label}
      </div>
      {isUser ? (
        <div className="whitespace-pre-wrap break-words text-sm text-cronymax-title">
          {message.content}
        </div>
      ) : (
        <div className="text-sm text-cronymax-title">
          <Streamdown animated isAnimating={isStreaming}>
            {message.content}
          </Streamdown>
        </div>
      )}
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

      // Register the event listener BEFORE sending the run request so we
      // never miss events from a run that fails or completes quickly. The
      // listener is safe to install early: it filters by tag and run_id.
      let runId = "";
      // Deduplicate events by sequence number. Even after fixing the C++
      // subscription count, multiple C++ event_subs_ entries (Lambda S +
      // accumulated Lambda A per run) can broadcast the same event repeatedly.
      // The sequence number in the envelope is unique per event; duplicates
      // carry the same number and are dropped here.
      const seenSeqs = new Set<number>();
      const off = bridge.on("event", (raw: unknown) => {
        const ev = raw as Record<string, unknown> | null;
        if (!ev) return;

        // RuntimeToClient::Event shape: {tag:"event", subscription, event:{sequence,payload:{kind,...}}}
        if (ev.tag === "event") {
          const inner = (ev.event as Record<string, unknown> | undefined) ?? {};
          // Drop duplicate broadcasts of the same event (same sequence number).
          const seq = inner.sequence as number | undefined;
          if (typeof seq === "number") {
            if (seenSeqs.has(seq)) return;
            seenSeqs.add(seq);
          }
          const pl =
            (inner.payload as Record<string, unknown> | undefined) ?? {};
          const pRunId =
            (pl.run_id as string | undefined) ??
            ((inner as Record<string, unknown>).run_id as string | undefined);
          if (pRunId && runId && pRunId !== runId) return;
          const kind = pl.kind as string | undefined;
          if (kind === "token") {
            const content = (pl.delta ?? pl.content) as string | undefined;
            if (content) {
              if (assistantMsgId === null) {
                assistantMsgId = state.msgSeq + 2;
                dispatch({
                  type: "addMessage",
                  role: "assistant",
                  content: "",
                  agentName: speaker || undefined,
                });
              }
              assistantText += content;
              dispatch({
                type: "updateMessage",
                id: assistantMsgId,
                content: assistantText,
              });
            }
          } else if (kind === "run_status") {
            const status = pl.status as string | undefined;
            if (
              status === "succeeded" ||
              status === "failed" ||
              status === "cancelled"
            ) {
              dispatch({
                type: "appendToMessage",
                id: traceMsgId,
                chunk: status === "succeeded" ? "\n✓ done" : `\n✗ ${status}`,
              });
              if (assistantMsgId === null && assistantText === "") {
                assistantText =
                  status === "succeeded" ? "(completed)" : `(no output)`;
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
              off();
              dispatch({ type: "setRunning", running: false });
              inputRef.current?.focus();
            }
          } else if (kind === "log") {
            dispatch({
              type: "appendToMessage",
              id: traceMsgId,
              chunk: `\n→ ${pl.message ?? ""}`,
            });
          }
          return;
        }

        // AppEvent shape: {kind:"agent_status"|"text"|..., run_id?}
        if (typeof ev.run_id === "string" && ev.run_id !== runId) return;
        if (ev.kind === "error") {
          const pl2 = (ev.payload as Record<string, unknown> | undefined) ?? {};
          dispatch({
            type: "appendToMessage",
            id: traceMsgId,
            chunk: `\n✗ ${pl2.message ?? "error"}`,
          });
        }
      });

      try {
        runId = await bridge.send("agent.run", { task: body });
        if (!runId) throw new Error("runtime did not return run_id");

        // Subscribe to runtime events for this run so tokens/trace stream in.
        await bridge
          .send("events.subscribe", { run_id: runId })
          .catch(() => {});
      } catch (err) {
        off();
        dispatch({
          type: "addMessage",
          role: "system",
          content: "Failed to start run: " + (err as Error).message,
        });
        dispatch({ type: "setRunning", running: false });
        return;
      }

      // Safety timeout: clean up if the run takes more than 5 minutes.
      setTimeout(
        () => {
          off();
          if (state.running) dispatch({ type: "setRunning", running: false });
        },
        5 * 60 * 1000,
      );
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
    <main className="flex h-screen flex-col bg-cronymax-base text-cronymax-title">
      <header className="flex items-center gap-3 border-b border-cronymax-border bg-cronymax-float px-3 py-2 text-sm">
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
                  ? "bg-cronymax-primary text-white"
                  : "text-cronymax-caption hover:text-cronymax-title")
              }
            >
              {m.label}
            </button>
          ))}
        </div>

        {state.chatMode === "agent" ? (
          <label className="flex items-center gap-1 text-xs text-cronymax-caption">
            Agent:
            <select
              value={state.selectedAgent}
              onChange={(e) => {
                dispatch({ type: "setSelectedAgent", name: e.target.value });
                persistSelectedAgent(e.target.value);
              }}
              className="rounded border border-cronymax-border bg-cronymax-base px-1.5 py-0.5 text-xs text-cronymax-title"
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
          <label className="flex items-center gap-1 text-xs text-cronymax-caption">
            Flow:
            <select
              value={state.selectedFlow}
              onChange={(e) => {
                dispatch({ type: "setSelectedFlow", name: e.target.value });
                persistSelectedFlow(e.target.value);
              }}
              className="rounded border border-cronymax-border bg-cronymax-base px-1.5 py-0.5 text-xs text-cronymax-title"
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
          className="rounded border border-cronymax-border bg-cronymax-base px-2 py-0.5 text-xs text-cronymax-title hover:bg-cronymax-float"
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
          <MessageView
            key={m.id}
            message={m}
            isStreaming={
              state.running &&
              m.role === "assistant" &&
              m.id ===
                state.messages.filter((x) => x.role === "assistant").at(-1)?.id
            }
          />
        ))}
      </div>

      <form
        onSubmit={onSubmit}
        className="flex gap-2 border-t border-cronymax-border bg-cronymax-float p-2"
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
          className="flex-1 resize-none rounded border border-cronymax-border bg-cronymax-base px-2 py-1.5 text-sm text-cronymax-title outline-none focus:border-cronymax-primary"
        />
        <button
          type="submit"
          disabled={state.running}
          className="rounded bg-cronymax-primary px-3 py-1.5 text-sm font-medium text-white transition hover:opacity-90 disabled:cursor-not-allowed disabled:opacity-50"
        >
          {state.running ? "…" : "Send"}
        </button>
      </form>
    </main>
  );
}
