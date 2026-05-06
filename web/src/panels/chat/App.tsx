import {
  useEffect,
  useRef,
  useCallback,
  useState,
  useLayoutEffect,
  type FormEvent,
  type KeyboardEvent,
  type ChangeEvent,
  type ClipboardEvent,
} from "react";
import { Streamdown } from "streamdown";
import { bridge } from "@/bridge";
import {
  useStore,
  loadChatData,
  persistChatData,
  ensureChat,
  chatNameFor,
  loadFlowsList,
  loadSavedGraph,
  persistSelectedFlow,
  loadSelectedAgent,
  persistSelectedAgent,
  loadChatMode,
  persistChatMode,
  loadSelectedModel,
  persistSelectedModel,
  type ChatMode,
  type Block,
  type ConversationBlock,
  type ShellBlock,
  type Attachment,
  type Thread,
} from "./store";
import { useSelectionTooltip } from "./useSelectionTooltip";

// ── helpers ────────────────────────────────────────────────────────────

function leadAgentOfFlow(flowName: string): string {
  const spec = loadSavedGraph(flowName);
  if (!spec || !spec.nodes.length) return "";
  const lead = spec.nodes
    .slice()
    .sort((a, b) => Number(a.id) - Number(b.id))[0];
  const cfg = (lead?.config ?? {}) as Record<string, unknown>;
  return (cfg.agent_name as string) || lead?.type || "";
}

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

function fmtDuration(ms: number): string {
  if (ms < 1000) return `${ms}ms`;
  return `${(ms / 1000).toFixed(1)}s`;
}

// ── Block components ────────────────────────────────────────────────────

function ThreadSummary({
  thread,
  onExpand,
}: {
  thread: Thread;
  onExpand: () => void;
}) {
  const lastMsg = thread.messages.at(-1);
  return (
    <div className="mt-2 rounded border border-cronymax-border bg-cronymax-float px-3 py-2 text-xs">
      <div className="flex items-center gap-2">
        <span className="font-semibold capitalize text-cronymax-primary">
          {thread.action}
        </span>
        <span className="text-cronymax-caption">
          {thread.messages.length} message
          {thread.messages.length !== 1 ? "s" : ""}
        </span>
        {thread.running && (
          <span className="text-cronymax-caption italic">running…</span>
        )}
        <button
          type="button"
          onClick={onExpand}
          className="ml-auto text-cronymax-primary hover:underline"
        >
          View thread ⇄
        </button>
      </div>
      {lastMsg && (
        <div className="mt-1 truncate text-cronymax-caption">
          {lastMsg.role === "assistant" ? lastMsg.content.slice(0, 80) : ""}
        </div>
      )}
    </div>
  );
}

function ConversationBlockView({
  block,
  isStreaming,
  isHighlighted,
}: {
  block: ConversationBlock;
  isStreaming: boolean;
  isHighlighted?: boolean;
}) {
  const pinnedComments = block.comments.filter((c) => c.pinnedToPrompt);
  return (
    <div
      className={`py-4 space-y-2 transition-all duration-500${
        isHighlighted ? " rounded-md ring-2 ring-cronymax-primary/40" : ""
      }`}
      data-block-id={block.id}
    >
      {/* User message */}
      <div className="rounded-md bg-cronymax-primary/10 px-3 py-2">
        <div className="mb-1 text-[10px] font-semibold uppercase tracking-wide text-cronymax-primary">
          You
        </div>
        {block.attachments.length > 0 && (
          <div className="mb-1 flex flex-wrap gap-1">
            {block.attachments.map((a) => (
              <span
                key={a.id}
                className="rounded-full bg-cronymax-border px-2 py-0.5 text-[10px] text-cronymax-caption"
              >
                {a.kind === "comment"
                  ? "💬 "
                  : a.kind === "image"
                    ? "🖼 "
                    : "📎 "}
                {a.label}
              </span>
            ))}
          </div>
        )}
        <div className="whitespace-pre-wrap break-words text-sm text-cronymax-title">
          {block.userContent}
        </div>
      </div>

      {/* Trace */}
      {block.traceContent && (
        <div className="py-1 font-mono text-[11px] text-cronymax-caption whitespace-pre-wrap">
          {block.traceContent}
        </div>
      )}

      {/* Assistant response */}
      {(block.assistantContent || block.status === "running") && (
        <div className="px-1">
          <div className="mb-1 text-[10px] font-semibold uppercase tracking-wide text-cronymax-caption">
            {block.agentName || "Assistant"}
          </div>
          <div className="text-sm text-cronymax-title">
            <Streamdown animated isAnimating={isStreaming}>
              {block.assistantContent}
            </Streamdown>
          </div>
        </div>
      )}

      {/* Status error */}
      {block.status === "fail" && !block.assistantContent && (
        <div className="text-xs italic text-red-400">(run failed)</div>
      )}

      {/* Comment annotations */}
      {pinnedComments.map((c) => (
        <div
          key={c.id}
          data-comment-id={c.id}
          className="rounded border-l-2 border-blue-500 bg-blue-500/10 px-2 py-1 text-xs text-blue-300"
        >
          💬 "{c.selectedText.slice(0, 80)}"
        </div>
      ))}

      {/* Thread */}
      {block.thread && (
        <ThreadSummary thread={block.thread} onExpand={() => {}} />
      )}
    </div>
  );
}

function ShellBlockView({
  block,
  onAction,
  isHighlighted,
}: {
  block: ShellBlock;
  onAction: (action: string, b: ShellBlock) => void;
  isHighlighted?: boolean;
}) {
  const [collapsed, setCollapsed] = useState(false);
  const duration =
    block.endedAt && block.startedAt
      ? fmtDuration(block.endedAt - block.startedAt)
      : null;
  const statusColor =
    block.status === "ok"
      ? "text-green-400"
      : block.status === "fail"
        ? "text-red-400"
        : "text-amber-400";
  const statusGlyph =
    block.status === "ok" ? "✓" : block.status === "fail" ? "✗" : "●";

  return (
    <div
      className={`py-4 space-y-1.5 transition-all duration-500${
        isHighlighted ? " rounded-md ring-2 ring-cronymax-primary/40" : ""
      }`}
      data-block-id={block.id}
    >
      {/* Header — highlighted command prompt */}
      <div className="flex items-center gap-2 rounded-md bg-cronymax-primary/10 px-3 py-1.5">
        <span className={`font-mono text-sm font-bold ${statusColor}`}>
          {statusGlyph}
        </span>
        <span className="flex-1 font-mono text-sm text-cronymax-title">
          $ {block.command}
        </span>
        {block.exitCode !== null && block.exitCode !== 0 && (
          <span className="text-[10px] text-red-400">
            exit {block.exitCode}
          </span>
        )}
        {duration && (
          <span className="text-[10px] text-cronymax-caption">{duration}</span>
        )}
        <button
          type="button"
          onClick={() => setCollapsed((c) => !c)}
          className="text-[10px] text-cronymax-caption hover:text-cronymax-title"
        >
          {collapsed ? "▶" : "▼"}
        </button>
      </div>

      {/* Output */}
      {!collapsed && block.output && (
        <pre className="max-h-80 overflow-y-auto rounded bg-cronymax-base px-3 py-1.5 font-mono text-[11px] text-cronymax-caption">
          {block.output}
        </pre>
      )}

      {/* Action bar */}
      {block.status !== "running" && (
        <div className="flex gap-2 pt-0.5">
          {(["Explain", "Fix", "Retry"] as const).map((act) => (
            <button
              key={act}
              type="button"
              onClick={() => onAction(act.toLowerCase(), block)}
              className="rounded border border-cronymax-border bg-cronymax-base px-2 py-0.5 text-[10px] text-cronymax-caption hover:text-cronymax-title"
            >
              {act}
            </button>
          ))}
        </div>
      )}

      {/* Comment annotations */}
      {block.comments
        .filter((c) => c.pinnedToPrompt)
        .map((c) => (
          <div
            key={c.id}
            data-comment-id={c.id}
            className="rounded border-l-2 border-blue-500 bg-blue-500/10 px-2 py-1 text-xs text-blue-300"
          >
            💬 "{c.selectedText.slice(0, 80)}"
          </div>
        ))}

      {/* Thread */}
      {block.thread && (
        <ThreadSummary thread={block.thread} onExpand={() => {}} />
      )}
    </div>
  );
}

function BlockView({
  block,
  isStreaming,
  onShellAction,
  isHighlighted,
}: {
  block: Block;
  isStreaming: boolean;
  onShellAction: (action: string, b: ShellBlock) => void;
  isHighlighted?: boolean;
}) {
  if (block.kind === "conversation") {
    return (
      <ConversationBlockView
        block={block}
        isStreaming={isStreaming}
        isHighlighted={isHighlighted}
      />
    );
  }
  return (
    <ShellBlockView
      block={block}
      onAction={onShellAction}
      isHighlighted={isHighlighted}
    />
  );
}

// ── Attachment tray ─────────────────────────────────────────────────────

function AttachmentTray({
  attachments,
  onRemove,
  onCommentClick,
}: {
  attachments: Attachment[];
  onRemove: (id: string) => void;
  onCommentClick?: (a: Attachment) => void;
}) {
  if (attachments.length === 0) return null;
  const comments = attachments.filter((a) => a.kind === "comment");
  const files = attachments.filter((a) => a.kind === "file");
  const images = attachments.filter((a) => a.kind === "image");

  const Pill = ({ a }: { a: Attachment }) => (
    <span className="flex items-center gap-1 rounded-full bg-cronymax-float border border-cronymax-border px-2 py-0.5 text-[10px] text-cronymax-caption">
      {a.kind === "comment" ? "💬" : a.kind === "image" ? "🖼" : "📎"}
      <span
        className={`max-w-[100px] truncate${
          a.kind === "comment" && onCommentClick
            ? " cursor-pointer hover:text-cronymax-title"
            : ""
        }`}
        onClick={
          a.kind === "comment" && onCommentClick
            ? () => onCommentClick(a)
            : undefined
        }
      >
        {a.label}
      </span>
      <button
        type="button"
        onClick={() => onRemove(a.id)}
        className="ml-0.5 text-cronymax-caption hover:text-red-400"
      >
        ×
      </button>
    </span>
  );

  return (
    <div className="flex flex-wrap gap-1.5 border-t border-cronymax-border px-2 py-1.5">
      {comments.length > 0 && (
        <div className="flex flex-wrap gap-1">
          {comments.map((a) => (
            <Pill key={a.id} a={a} />
          ))}
        </div>
      )}
      {files.length > 0 && (
        <div className="flex flex-wrap gap-1">
          {files.map((a) => (
            <Pill key={a.id} a={a} />
          ))}
        </div>
      )}
      {images.length > 0 && (
        <div className="flex flex-wrap gap-1">
          {images.map((a) => (
            <Pill key={a.id} a={a} />
          ))}
        </div>
      )}
    </div>
  );
}

// ── Main App component ─────────────────────────────────────────────────

export function App() {
  const [state, dispatch] = useStore();
  const inputRef = useRef<HTMLTextAreaElement>(null);
  const timelineRef = useRef<HTMLDivElement>(null);
  const fileInputRef = useRef<HTMLInputElement>(null);
  const [agentLoadError, setAgentLoadError] = useState<string | null>(null);
  const [inputMode, setInputMode] = useState<"chat" | "shell" | "command">(
    "chat",
  );
  const [commentDraft, setCommentDraft] = useState("");

  // Selection tooltip — freeze when comment input is focused so it doesn't
  // disappear when the browser clears the selection on input focus.
  const selectionInfo = useSelectionTooltip(timelineRef);
  const [frozenSelection, setFrozenSelection] = useState<
    import("./useSelectionTooltip").SelectionInfo | null
  >(null);
  const activeSelection = frozenSelection ?? selectionInfo;

  // ── comment attachment → scroll & highlight ────────────────────────────
  const [highlightedBlockId, setHighlightedBlockId] = useState<string | null>(
    null,
  );
  const onCommentAttachmentClick = useCallback(
    (a: Attachment) => {
      if (!a.commentId) return;
      // Find the block that owns this comment
      let blockId: string | undefined;
      for (const blk of state.blocks) {
        if (blk.comments.find((c) => c.id === a.commentId)) {
          blockId = blk.id;
          break;
        }
      }
      if (!blockId) return;
      // Scroll to the comment annotation, or the block if annotation not rendered yet
      const target =
        timelineRef.current?.querySelector(
          `[data-comment-id="${a.commentId}"]`,
        ) ?? timelineRef.current?.querySelector(`[data-block-id="${blockId}"]`);
      target?.scrollIntoView({ behavior: "smooth", block: "nearest" });
      setHighlightedBlockId(blockId);
      setTimeout(() => setHighlightedBlockId(null), 1600);
    },
    [state.blocks],
  );

  // ── agent catalog ─────────────────────────────────────────────────────
  const refreshAgents = useCallback(async () => {
    try {
      let res = await bridge.send("agent.registry.list");
      let names = (res.agents ?? []).map((a) => a.name);
      if (names.length === 0) {
        await bridge.send("agent.registry.save", {
          name: "Chat",
          llm: "",
          system_prompt: "You are a helpful assistant.",
          memory_namespace: "",
          tools_csv: "",
        });
        res = await bridge.send("agent.registry.list");
        names = (res.agents ?? []).map((a) => a.name);
      }
      const selected = loadSelectedAgent(names);
      dispatch({ type: "setAgents", agents: res.agents ?? [], selected });
      setAgentLoadError(null);
    } catch (err) {
      setAgentLoadError((err as Error).message);
    }
  }, [dispatch]);

  // ── ensure terminal session for this chat tab ─────────────────────────
  const ensureChatTerminal = useCallback(
    async (currentTid: string | null, chatId: string) => {
      // Validate the cached terminal ID: it won't survive an app restart,
      // so check whether it's still present in the C++ process before reusing.
      if (currentTid) {
        try {
          const { items } = await bridge.send("terminal.list");
          if (items.some((t) => t.id === currentTid)) return currentTid;
        } catch {
          // Fall through to create a new terminal.
        }
      }
      try {
        const newTid = await bridge.send("terminal.new");
        const tid =
          typeof newTid === "string" ? newTid : (newTid as { id: string }).id;
        await bridge.send("terminal.start", { id: tid });
        dispatch({ type: "setTerminalTid", tid });
        // Persist immediately
        const { data } = loadChatData(chatId);
        persistChatData(chatId, { ...data, terminalTid: tid });
        return tid;
      } catch {
        return null;
      }
    },
    [dispatch],
  );

  // ── init ────────────────────────────────────────────────────────────────
  useEffect(() => {
    const init = async () => {
      // Ask the native shell which tab we are, so we can restore the same
      // chatId that was bound to this tab in a previous session.
      try {
        const tabInfo = await bridge.send("shell.this_tab_id");
        if (tabInfo?.meta?.chat_id) {
          // Seed sessionStorage so ensureChat() picks up the persisted chatId.
          sessionStorage.setItem("cronymax_chat_tab_id", tabInfo.meta.chat_id);
        }
      } catch {
        // Bridge may not be ready on first run or in dev; fall through.
      }

      const { id, name } = ensureChat();

      // Register this tab's chatId with the native shell so it survives
      // the next app restart (no-op if already registered with same value).
      void bridge
        .send("shell.tab_set_meta", { key: "chat_id", value: id })
        .catch(() => {});

      const { data, migrationNotice } = loadChatData(id);
      const model = data.model || loadSelectedModel();
      dispatch({
        type: "loadChat",
        id,
        name,
        blocks: data.blocks,
        terminalTid: data.terminalTid,
        model,
        migrationNotice,
      });
      const { flows, selected } = loadFlowsList();
      dispatch({ type: "setFlows", flows, selected });
      dispatch({ type: "setChatMode", mode: loadChatMode() });
      void refreshAgents();
      void ensureChatTerminal(data.terminalTid, id);
    };

    void init();

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
        if (state.activeChatId) {
          const { data: d, migrationNotice: mn } = loadChatData(
            state.activeChatId,
          );
          dispatch({
            type: "loadChat",
            id: state.activeChatId,
            name: chatNameFor(state.activeChatId),
            blocks: d.blocks,
            terminalTid: d.terminalTid,
            model: d.model || loadSelectedModel(),
            migrationNotice: mn,
          });
        }
      }
    };
    window.addEventListener("storage", onStorage);
    return () => window.removeEventListener("storage", onStorage);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [dispatch, refreshAgents]);

  // Use refs so the listener always reads latest values without re-subscribing
  const runningBlockIdRef = useRef<string | null>(null);
  const terminalTidRef = useRef<string | null>(null);
  runningBlockIdRef.current = state.runningBlockId;
  terminalTidRef.current = state.terminalTid;

  // ── terminal.output → ShellBlock accumulation ─────────────────────────
  // Subscribe once (stable); read live values from refs to avoid stale closures.
  useEffect(() => {
    const off = bridge.on("terminal.output", (raw: unknown) => {
      const p = raw as { id?: string; data?: string } | null;
      if (!p || !p.id || !p.data) return;
      if (p.id !== terminalTidRef.current) return;
      const blockId = runningBlockIdRef.current;
      if (!blockId) return;

      const info = shellSentinelRef.current.get(blockId);
      if (!info) return;

      let data = p.data;

      // Phase 1: waiting for start marker — discard all preamble
      // (shell prompts, echoed command line, etc.)
      if (!info.capturing) {
        const startIdx = data.indexOf(info.start);
        if (startIdx === -1) return; // still preamble — discard chunk
        // Skip to the line after the start marker line
        const nl = data.indexOf("\n", startIdx);
        data = nl !== -1 ? data.slice(nl + 1) : "";
        info.capturing = true;
        if (!data) return;
      }

      // Phase 2: capturing — look for end marker
      const endRe = new RegExp(`${info.end}:(\\d+)`);
      const match = endRe.exec(data);
      if (match) {
        const exitCode = parseInt(match[1]!, 10);
        // Deliver output before the end-marker line (strip trailing partial line)
        const cleanData = data.slice(0, match.index).replace(/[^\n]*$/, "");
        if (cleanData) {
          dispatch({
            type: "appendShellOutput",
            id: blockId,
            chunk: cleanData,
            now: Date.now(),
          });
        }
        dispatch({
          type: "finalizeShellBlock",
          id: blockId,
          exitCode,
          now: Date.now(),
        });
        dispatch({ type: "setRunning", running: false });
        dispatch({ type: "setRunningBlockId", id: null });
        runningBlockIdRef.current = null;
        shellSentinelRef.current.delete(blockId);
        return;
      }

      dispatch({
        type: "appendShellOutput",
        id: blockId,
        chunk: data,
        now: Date.now(),
      });
    });
    return off;
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [dispatch]);

  // ── auto scroll ──────────────────────────────────────────────────────
  useLayoutEffect(() => {
    const el = timelineRef.current;
    if (el) el.scrollTop = el.scrollHeight;
  }, [state.blocks]);

  // ── input mode detection + prefix auto-strip ─────────────────────────
  const onInputChange = useCallback((e: ChangeEvent<HTMLTextAreaElement>) => {
    const v = e.currentTarget.value;
    if (v.startsWith("$")) {
      setInputMode("shell");
      // Strip the $ (and optional space) so textarea shows only the command
      e.currentTarget.value = v.startsWith("$ ") ? v.slice(2) : v.slice(1);
    } else if (v.startsWith("/")) {
      setInputMode("command");
      e.currentTarget.value = v.startsWith("/ ") ? v.slice(2) : v.slice(1);
    } else if (v === "") {
      setInputMode("chat");
    }
  }, []);

  // ── paste → attach ────────────────────────────────────────────────────
  const onPaste = useCallback(
    (e: ClipboardEvent<HTMLTextAreaElement>) => {
      const items = Array.from(e.clipboardData.items);
      const imageItem = items.find((i) => i.type.startsWith("image/"));
      if (imageItem) {
        e.preventDefault();
        const file = imageItem.getAsFile();
        if (!file) return;
        const reader = new FileReader();
        reader.onload = (ev) => {
          const dataUrl = ev.target?.result as string;
          dispatch({
            type: "addAttachment",
            attachment: {
              id: crypto.randomUUID(),
              kind: "image",
              label: file.name || "pasted-image",
              content: dataUrl,
            },
          });
        };
        reader.readAsDataURL(file);
      }
    },
    [dispatch],
  );

  // ── file picker ────────────────────────────────────────────────────────
  const onFileChange = useCallback(
    (e: ChangeEvent<HTMLInputElement>) => {
      const files = Array.from(e.target.files ?? []);
      for (const file of files) {
        const reader = new FileReader();
        reader.onload = (ev) => {
          const content = ev.target?.result as string;
          dispatch({
            type: "addAttachment",
            attachment: {
              id: crypto.randomUUID(),
              kind: file.type.startsWith("image/") ? "image" : "file",
              label: file.name,
              content,
            },
          });
        };
        if (file.type.startsWith("image/")) {
          reader.readAsDataURL(file);
        } else {
          reader.readAsText(file);
        }
      }
      e.target.value = "";
    },
    [dispatch],
  );

  // Per-block sentinel info: start marker, end marker, and whether the start
  // marker has already been seen (so preamble / echo lines are discarded).
  const shellSentinelRef = useRef<
    Map<string, { start: string; end: string; capturing: boolean }>
  >(new Map());

  // ── send / run ─────────────────────────────────────────────────────────
  const onRun = useCallback(
    async (rawText: string) => {
      if (state.running || !state.activeChatId) return;
      const chatId = state.activeChatId;

      // Detect shell mode: rawText starts with "$" OR inputMode is "shell"
      const isShellCmd =
        rawText.trimStart().startsWith("$") || inputMode === "shell";
      if (isShellCmd) {
        // Strip leading "$ " or "$" if present (user may have typed it or mode stripped it)
        const command = rawText.replace(/^\s*\$\s*/, "").trim();
        const tid = await ensureChatTerminal(state.terminalTid, chatId);
        if (!tid) return;

        const blockId = crypto.randomUUID();
        // Start/end sentinels — only hex chars, safe through JSON serialization.
        // Anything before the start marker (shell prompt, command echo) is discarded;
        // only what appears between start and end is shown as output.
        const nonce = Array.from(crypto.getRandomValues(new Uint8Array(8)))
          .map((b) => b.toString(16).padStart(2, "0"))
          .join("");
        const startMarker = `__cx_s_${nonce}__`;
        const endMarker = `__cx_e_${nonce}__`;
        shellSentinelRef.current.set(blockId, {
          start: startMarker,
          end: endMarker,
          capturing: false,
        });

        const shellBlock: import("./store").ShellBlock = {
          kind: "shell",
          id: blockId,
          command,
          output: "",
          rawBuf: "",
          status: "running",
          exitCode: null,
          startedAt: Date.now(),
          endedAt: null,
          comments: [],
        };
        dispatch({ type: "createBlock", block: shellBlock });
        dispatch({ type: "setRunningBlockId", id: blockId });
        dispatch({ type: "setRunning", running: true });
        dispatch({ type: "clearPinnedComments" });
        // Eagerly update refs so the terminal.output listener is ready
        // immediately — before React re-renders and updates from state.
        runningBlockIdRef.current = blockId;
        terminalTidRef.current = tid;

        // Bracket the command with start/end markers so preamble (prompt, echo)
        // is automatically discarded. Single-quoted start (no expansion needed).
        const wrapped = `echo '${startMarker}'; ${command}; _ec=$?; echo "${endMarker}:$_ec"`;
        try {
          await bridge.send("terminal.run", { id: tid, command: wrapped });
        } catch (err) {
          dispatch({
            type: "finalizeShellBlock",
            id: blockId,
            exitCode: 1,
            now: Date.now(),
          });
          dispatch({ type: "setRunning", running: false });
          dispatch({ type: "setRunningBlockId", id: null });
          shellSentinelRef.current.delete(blockId);
          return;
        }

        // Safety timeout 5 min
        setTimeout(
          () => {
            if (runningBlockIdRef.current === blockId) {
              dispatch({ type: "setRunning", running: false });
              dispatch({ type: "setRunningBlockId", id: null });
              shellSentinelRef.current.delete(blockId);
            }
          },
          5 * 60 * 1000,
        );
        return;
      }

      // ── Normal conversation run ─────────────────────────────────────
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

      const blockId = crypto.randomUUID();
      const block: import("./store").ConversationBlock = {
        kind: "conversation",
        id: blockId,
        userContent: rawText,
        attachments: state.attachments.slice(),
        assistantContent: "",
        agentName: speaker || undefined,
        traceContent: speaker
          ? `▶ ${speaker} (${state.chatMode}) running…`
          : "▶ running…",
        status: "running",
        comments: [],
        createdAt: Date.now(),
      };
      dispatch({ type: "createBlock", block });
      dispatch({ type: "setRunningBlockId", id: blockId });
      // Clear attachments from tray after capturing snapshot into block
      dispatch({ type: "clearAttachments" });
      dispatch({ type: "clearPinnedComments" });

      let assistantText = "";
      const seenSeqs = new Set<number>();
      let runId = "";

      const off = bridge.on("event", (raw: unknown) => {
        const ev = raw as Record<string, unknown> | null;
        if (!ev) return;

        if (ev.tag === "event") {
          const inner = (ev.event as Record<string, unknown> | undefined) ?? {};
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
              assistantText += content;
              dispatch({
                type: "setAssistantContent",
                id: blockId,
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
              const finalStatus = status === "succeeded" ? "ok" : "fail";
              dispatch({
                type: "appendToTrace",
                id: blockId,
                chunk: status === "succeeded" ? "\n✓ done" : `\n✗ ${status}`,
              });
              if (!assistantText) {
                assistantText =
                  status === "succeeded" ? "(completed)" : "(no output)";
                dispatch({
                  type: "setAssistantContent",
                  id: blockId,
                  content: assistantText,
                });
              }
              dispatch({
                type: "finalizeBlock",
                id: blockId,
                status: finalStatus,
                agentName: speaker || undefined,
              });

              // Persist
              const { data } = loadChatData(chatId);
              persistChatData(chatId, { ...data, blocks: [...data.blocks] });

              off();
              dispatch({ type: "setRunning", running: false });
              dispatch({ type: "setRunningBlockId", id: null });
              inputRef.current?.focus();
            }
          } else if (kind === "log") {
            dispatch({
              type: "appendToTrace",
              id: blockId,
              chunk: `\n→ ${pl.message ?? ""}`,
            });
          }
          return;
        }

        if (typeof ev.run_id === "string" && ev.run_id !== runId) return;
        if (ev.kind === "error") {
          const pl2 = (ev.payload as Record<string, unknown> | undefined) ?? {};
          dispatch({
            type: "appendToTrace",
            id: blockId,
            chunk: `\n✗ ${pl2.message ?? "error"}`,
          });
        }
      });

      try {
        runId = await bridge.send("agent.run", { task: body });
        if (!runId) throw new Error("runtime did not return run_id");
        await bridge
          .send("events.subscribe", { run_id: runId })
          .catch(() => {});
      } catch (err) {
        off();
        dispatch({
          type: "finalizeBlock",
          id: blockId,
          status: "fail",
        });
        dispatch({
          type: "appendToTrace",
          id: blockId,
          chunk: "\n✗ Failed to start: " + (err as Error).message,
        });
        dispatch({ type: "setRunning", running: false });
        dispatch({ type: "setRunningBlockId", id: null });
        return;
      }

      setTimeout(
        () => {
          off();
          if (state.running) {
            dispatch({ type: "setRunning", running: false });
            dispatch({ type: "setRunningBlockId", id: null });
          }
        },
        5 * 60 * 1000,
      );
    },
    [
      state.running,
      state.activeChatId,
      state.terminalTid,
      state.selectedFlow,
      state.selectedAgent,
      state.chatMode,
      state.agents,
      state.attachments,
      inputMode,
      dispatch,
      ensureChatTerminal,
    ],
  );

  // Finalize shell block when OSC 133 D sets status away from "running"
  useEffect(() => {
    if (!state.runningBlockId) return;
    const blk = state.blocks.find((b) => b.id === state.runningBlockId);
    if (!blk || blk.kind !== "shell") return;
    if (blk.status !== "running") {
      dispatch({ type: "setRunning", running: false });
      dispatch({ type: "setRunningBlockId", id: null });
      if (state.activeChatId) {
        const { data } = loadChatData(state.activeChatId);
        persistChatData(state.activeChatId, { ...data, blocks: state.blocks });
      }
    }
  }, [state.blocks, state.runningBlockId, state.activeChatId, dispatch]);

  // Persist blocks after each conversation finalization (state.running transitions)
  useEffect(() => {
    if (!state.running && state.activeChatId && state.blocks.length > 0) {
      persistChatData(state.activeChatId, {
        blocks: state.blocks,
        terminalTid: state.terminalTid,
        model: state.model,
      });
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [state.running]);

  const onShellAction = useCallback(
    (action: string, block: ShellBlock) => {
      // Spawn a thread on the block by running a prompt with context
      const prompt = `${action}: \`\`\`\n$ ${block.command}\n${block.output}\n\`\`\``;
      void onRun(prompt);
    },
    [onRun],
  );

  const onSubmit = useCallback(
    (e: FormEvent<HTMLFormElement>) => {
      e.preventDefault();
      const v = inputRef.current?.value.trim() || "";
      if (!v) return;
      if (inputRef.current) inputRef.current.value = "";
      setInputMode("chat");
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
    if (state.activeChatId) {
      persistChatData(state.activeChatId, {
        blocks: [],
        terminalTid: state.terminalTid,
        model: state.model,
      });
    }
  }, [dispatch, state.activeChatId, state.terminalTid, state.model]);

  const runningBlockId = state.runningBlockId;

  // Copilot-like textarea auto-height
  const onTextareaInput = useCallback(
    (e: React.FormEvent<HTMLTextAreaElement>) => {
      const el = e.currentTarget;
      el.style.height = "auto";
      el.style.height = Math.min(el.scrollHeight, 200) + "px";
    },
    [],
  );

  return (
    <main className="flex h-screen flex-col bg-cronymax-base text-cronymax-title">
      {/* Header */}
      <header className="flex items-center gap-3 border-b border-cronymax-border bg-cronymax-float px-3 py-2 text-sm">
        <span className="flex-1 truncate font-semibold">{state.chatName}</span>

        {/* Mode toggle */}
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

      {/* Migration notice */}
      {state.migrationNotice && (
        <div className="flex items-center gap-2 border-b border-amber-500/40 bg-amber-500/10 px-3 py-1 text-[11px] text-amber-300">
          <span className="flex-1">{state.migrationNotice}</span>
          <button
            type="button"
            onClick={() => dispatch({ type: "clearMigrationNotice" })}
            className="text-amber-300 hover:text-amber-100"
          >
            ×
          </button>
        </div>
      )}

      {/* Agent load error */}
      {agentLoadError && (
        <div className="border-b border-red-500/40 bg-red-500/10 px-3 py-1 text-[11px] text-red-300">
          agent.registry.list failed: {agentLoadError}
        </div>
      )}

      {/* Block timeline */}
      <div
        ref={timelineRef}
        className="flex-1 overflow-y-auto divide-y divide-cronymax-border px-4 py-2"
      >
        {state.blocks.map((b) => (
          <BlockView
            key={b.id}
            block={b}
            isStreaming={b.id === runningBlockId && b.kind === "conversation"}
            onShellAction={onShellAction}
            isHighlighted={b.id === highlightedBlockId}
          />
        ))}
      </div>

      {/* ── Floating selection tooltip ─────────────────────────────── */}
      {activeSelection && (
        <div
          className="fixed z-50 rounded-lg border border-cronymax-border bg-cronymax-float shadow-xl"
          style={{
            top: activeSelection.anchorRect.top - 8,
            left:
              activeSelection.anchorRect.left +
              activeSelection.anchorRect.width / 2,
            transform: "translateX(-50%) translateY(-100%)",
          }}
          onMouseDown={(e) => {
            // Always prevent default to keep text selection alive.
            // Manually focus inputs so they still receive keyboard events.
            e.preventDefault();
            if (e.target instanceof HTMLInputElement) {
              (e.target as HTMLInputElement).focus();
            }
          }}
        >
          {/* Quick actions row */}
          <div className="flex items-center gap-0.5 px-1.5 pt-1.5">
            <button
              type="button"
              className="rounded px-2 py-0.5 text-[11px] text-cronymax-caption hover:text-cronymax-title hover:bg-cronymax-border transition"
              onClick={() =>
                navigator.clipboard.writeText(activeSelection.selectedText)
              }
            >
              Copy
            </button>
            <button
              type="button"
              className="rounded px-2 py-0.5 text-[11px] text-cronymax-primary hover:bg-cronymax-primary/20 transition"
              onClick={() => {
                const commentId = crypto.randomUUID();
                dispatch({
                  type: "pinComment",
                  comment: {
                    id: commentId,
                    blockId: activeSelection.blockId,
                    selectedText: activeSelection.selectedText,
                    text: commentDraft.trim() || undefined,
                    pinnedToPrompt: true,
                  },
                });
                setCommentDraft("");
                setFrozenSelection(null);
                window.getSelection()?.removeAllRanges();
              }}
            >
              Pin ↑
            </button>
          </div>
          {/* Comment input */}
          <div className="px-2 pb-2 pt-1">
            <input
              type="text"
              value={commentDraft}
              onChange={(e) => setCommentDraft(e.target.value)}
              placeholder="Add a comment… (Enter to pin)"
              className="w-52 rounded border border-cronymax-border bg-cronymax-base px-2 py-1 text-[11px] text-cronymax-title placeholder:text-cronymax-caption outline-none focus:border-cronymax-primary"
              onFocus={() =>
                setFrozenSelection(selectionInfo ?? frozenSelection)
              }
              onBlur={() => setFrozenSelection(null)}
              onKeyDown={(e) => {
                if (e.key === "Enter") {
                  e.preventDefault();
                  const commentId = crypto.randomUUID();
                  dispatch({
                    type: "pinComment",
                    comment: {
                      id: commentId,
                      blockId: activeSelection.blockId,
                      selectedText: activeSelection.selectedText,
                      text: commentDraft.trim() || undefined,
                      pinnedToPrompt: true,
                    },
                  });
                  setCommentDraft("");
                  setFrozenSelection(null);
                  window.getSelection()?.removeAllRanges();
                }
                if (e.key === "Escape") {
                  setCommentDraft("");
                  setFrozenSelection(null);
                  window.getSelection()?.removeAllRanges();
                }
              }}
            />
          </div>
        </div>
      )}

      {/* ── Copilot-like composer ──────────────────────────────────── */}
      <form onSubmit={onSubmit} className="px-3 pb-3 pt-1">
        {/* Attachment tray sits above the editor box */}
        <AttachmentTray
          attachments={state.attachments}
          onRemove={(id) => dispatch({ type: "removeAttachment", id })}
          onCommentClick={onCommentAttachmentClick}
        />

        {/* Editor card */}
        <div
          className={
            "flex flex-col rounded-xl border bg-cronymax-float transition-colors " +
            (inputMode === "shell"
              ? "border-amber-500/70 bg-amber-500/5"
              : "border-cronymax-border focus-within:border-cronymax-primary/60")
          }
        >
          {/* Prefix badge row (shown when mode ≠ chat) */}
          {inputMode !== "chat" && (
            <div className="flex items-center gap-1.5 px-3 pt-2 pb-0">
              <span
                className={
                  "rounded px-1.5 py-0.5 text-[10px] font-mono font-semibold " +
                  (inputMode === "shell"
                    ? "bg-amber-500/20 text-amber-300"
                    : "bg-cronymax-primary/20 text-cronymax-primary")
                }
              >
                {inputMode === "shell" ? "$ shell" : "/ command"}
              </span>
              <button
                type="button"
                className="text-[10px] text-cronymax-caption hover:text-cronymax-title ml-auto"
                onClick={() => {
                  if (inputRef.current) inputRef.current.value = "";
                  setInputMode("chat");
                }}
              >
                ×
              </button>
            </div>
          )}

          {/* Textarea */}
          <textarea
            ref={inputRef}
            rows={1}
            autoFocus
            placeholder={
              inputMode === "shell"
                ? "shell command…"
                : inputMode === "command"
                  ? "command…"
                  : state.chatMode === "flow"
                    ? "Ask anything… (@AgentName to address one)"
                    : "Ask anything… ($ for shell, / for commands)"
            }
            onKeyDown={onKeyDown}
            onChange={onInputChange}
            onInput={onTextareaInput}
            onPaste={onPaste}
            className="w-full resize-none bg-transparent px-3 py-2.5 text-sm text-cronymax-title outline-none placeholder:text-cronymax-caption"
          />

          {/* Bottom toolbar row */}
          <div className="flex items-center gap-1.5 px-2 pb-2">
            {/* Add button */}
            <button
              type="button"
              onClick={() => fileInputRef.current?.click()}
              className="flex items-center gap-1 rounded-md border border-cronymax-border bg-cronymax-base px-2 py-1 text-[11px] text-cronymax-caption hover:text-cronymax-title hover:bg-cronymax-float transition"
              title="Add file / image"
            >
              <span className="text-sm leading-none">+</span>
              <span>Add</span>
            </button>
            <input
              ref={fileInputRef}
              type="file"
              className="hidden"
              multiple
              onChange={onFileChange}
            />

            {/* Model dropdown */}
            <select
              value={state.model}
              onChange={(e) => {
                dispatch({ type: "setModel", model: e.target.value });
                persistSelectedModel(e.target.value);
              }}
              className="rounded-md border border-cronymax-border bg-cronymax-base px-1.5 py-1 text-[11px] text-cronymax-caption hover:text-cronymax-title transition max-w-[120px] truncate"
              title="LLM model"
            >
              <option value="">{state.model || "default"}</option>
              {state.model && (
                <option value={state.model}>{state.model}</option>
              )}
              <option value="">default</option>
            </select>

            <div className="flex-1" />

            {/* Send button */}
            <button
              type="submit"
              disabled={state.running}
              className="flex items-center justify-center rounded-md bg-cronymax-primary w-7 h-7 text-white transition hover:opacity-90 disabled:cursor-not-allowed disabled:opacity-50"
              title="Send (Enter)"
            >
              {state.running ? (
                <span className="text-xs">…</span>
              ) : (
                <svg width="14" height="14" viewBox="0 0 14 14" fill="none">
                  <path
                    d="M7 1L7 13M1 7L7 1L13 7"
                    stroke="currentColor"
                    strokeWidth="1.8"
                    strokeLinecap="round"
                    strokeLinejoin="round"
                  />
                </svg>
              )}
            </button>
          </div>
        </div>
      </form>
    </main>
  );
}
