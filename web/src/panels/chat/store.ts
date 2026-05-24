/**
 * Chat panel store — block-based conversation history, run state, agent selection.
 *
 * Block types:
 *   ConversationBlock  – LLM prompt/response pair
 *   ShellBlock         – `$`-prefixed shell command with OSC 133 output
 *
 * Storage key: `chat_history_v2:<chatId>`
 */
import { createPanelStore } from "@/hooks/usePanelStore";

// ── ANSI stripper (ported from terminal/store.ts) ─────────────────────
export function stripAnsi(str: string): string {
  return str
    .replace(/\x1b\[[\x30-\x3f]*[\x20-\x2f]*[\x40-\x7e]/g, "")
    .replace(/\x1b\][\s\S]*?(?:\x07|\x1b\\)/g, "")
    .replace(/\x1b[PX^_][\s\S]*?(?:\x07|\x1b\\)/g, "")
    .replace(/\x1b[()][\x20-\x7e]/g, "")
    .replace(/\x1b[=>78MEDH]/g, "")
    .replace(/\x1b/g, "")
    .replace(/\r(?!\n)/g, "\n")
    .replace(/\x07/g, "");
}

// ── Types ──────────────────────────────────────────────────────────────

export type BlockStatus = "running" | "ok" | "fail";

// ── ContentSegment union type ──────────────────────────────────────────

export type ContentSegment =
  | { kind: "text"; content: string }
  | {
      kind: "tool_call";
      toolCallId: string;
      tool: string;
      args: unknown;
      status: "running" | "done" | "error";
      result?: unknown;
      durationMs?: number;
    }
  | { kind: "thinking"; content: string; sealed: boolean; elapsedMs: number };

export interface Comment {
  id: string;
  blockId: string;
  selectedText: string;
  /** User-typed comment body (optional) */
  text?: string;
  /** True while pinned to the prompt attachment tray. */
  pinnedToPrompt: boolean;
}

export interface Attachment {
  id: string;
  kind: "comment" | "file" | "image";
  /** For "comment": selected text snippet. For "file"/"image": file name. */
  label: string;
  /** For "file"/"image": file content/data-url. */
  content?: string;
  /** Comment reference for kind==="comment" */
  commentId?: string;
  /** Full selected text (kind==="comment"). Label is truncated; this is the original. */
  selectedText?: string;
  /** User-typed annotation on the comment (kind==="comment"). */
  commentText?: string;
}

export interface ThreadMessage {
  id: string;
  role: "user" | "assistant";
  content: string;
  agentName?: string;
}

export interface Thread {
  id: string;
  /** Action that spawned the thread (e.g. "explain", "fix", "retry") */
  action: string;
  messages: ThreadMessage[];
  running: boolean;
  /** true = show inline in timeline; false = collapsed summary */
  expanded: boolean;
}

export type TraceEntry =
  | {
      kind: "run_start";
      model: string;
      systemPrompt: string;
      userInput: string;
      tools: string[];
      turnsLimit: number;
      ts: number;
    }
  | {
      kind: "assistant_turn";
      turnId: number;
      text: string;
      finishReason: string;
      ts: number;
      /** Token usage emitted by agent-run-middleware (optional). */
      usage?: { inputTokens: number; outputTokens: number };
      /** Turn duration from agent-run-middleware (optional). */
      durationMs?: number;
    }
  | {
      kind: "tool_start";
      toolCallId: string;
      tool: string;
      args: unknown;
      ts: number;
    }
  | {
      kind: "tool_done";
      toolCallId: string;
      tool: string;
      result: unknown;
      terminal: boolean;
      ts: number;
      /** Actual wall-clock duration from the Rust runtime (ms). */
      durationMs?: number;
      /** True when the tool returned an error outcome. */
      isError?: boolean;
    }
  | {
      kind: "approval_request";
      reviewId: string;
      tool: string;
      args: unknown;
      ts: number;
    }
  | {
      kind: "approval_resolved";
      reviewId: string;
      decision: "approve" | "reject";
      ts: number;
    }
  | {
      kind: "reflection";
      turn: number;
      text: string;
      ts: number;
    }
  | {
      kind: "memory_write";
      namespace: string;
      key: string;
      source: string;
      ts: number;
    };

export type FileChangeOperation = "created" | "modified" | "deleted" | "moved";

export interface FileChange {
  path: string;
  operation: FileChangeOperation;
  blockId: string;
  ts: number;
}

export interface ConversationBlock {
  kind: "conversation";
  id: string;
  /** The prompt sent by user (including @-mentions, comments, etc.) */
  userContent: string;
  /** Attachment snapshots included in this prompt */
  attachments: Attachment[];
  /** Ordered content stream (primary rendering model). */
  contentStream: ContentSegment[];
  /** Derived from text segments in contentStream; kept for backwards compat / search. */
  assistantContent: string;
  agentName?: string;
  traceEntries: TraceEntry[];
  /** Workspace file mutations recorded during this block's run. */
  fileChanges: FileChange[];
  /** "running" while streaming, "ok" or "fail" after final run_status */
  status: "running" | "ok" | "fail";
  comments: Comment[];
  thread?: Thread;
  /** Attached flow thread when this block triggered a flow run. */
  flowThread?: FlowThread;
  createdAt: number;
  /** Accumulated thinking/reasoning content from an extended-thinking model. */
  thinkingText: string;
  /** True once the model emits its first text token, sealing the thinking phase. */
  thinkingSealed: boolean;
  /** Timestamp (Date.now()) of the first thinking token; null if no thinking. */
  thinkingStartedAt: number | null;
  /** Elapsed ms from first thinking token to first text token (set on seal). */
  thinkingElapsedMs: number;
}

export interface ShellBlock {
  kind: "shell";
  id: string;
  command: string;
  output: string;
  /** Buffer for partial OSC sequences across chunks */
  rawBuf: string;
  status: BlockStatus;
  exitCode: number | null;
  startedAt: number;
  endedAt: number | null;
  comments: Comment[];
  thread?: Thread;
}

/** Lightweight flow-activity notification shown inline in the chat timeline. */
export interface FlowNotificationBlock {
  kind: "flow-notification";
  id: string;
  message: string;
  /** "success" = approval/completion, "info" = status update */
  variant: "success" | "info";
  ts: number;
  /** Kept for type compatibility with block iterators that access comments. */
  comments: Comment[];
}

// ── Thread card blocks ─────────────────────────────────────────────────────
// Added by supervisor-session-ux: standalone top-level timeline blocks that
// represent an in-flight or completed agent / flow thread. Clicking the card
// navigates into the thread view (two-level navigation).

/** Status of a supervisor-dispatched child task. */
export type TaskStatus = "running" | "succeeded" | "failed" | "pending" | "cancelled" | "awaiting_review";

/** Inline card block for a supervisor-dispatched agent sub-task. */
export interface AgentThreadBlock {
  kind: "agent_thread";
  id: string;
  /** Task id from the `TaskStarted` runtime event, used for navigation. */
  taskId: string;
  /** The parent run_id that spawned this agent task. */
  parentRunId: string;
  agentName: string;
  status: TaskStatus;
  /** First line summary of the task prompt. */
  summary: string;
  startedAt: number;
  endedAt: number | null;
  /** Kept for type compatibility with block iterators. */
  comments: Comment[];
  /** Child run_id once the sub-agent starts. Used to correlate critic_result events. */
  subRunId: string | null;
  /** Critic pass results received while the sub-agent was running (task 9.3). */
  criticResults: Array<{
    passed: boolean;
    summary: string;
    revision: number;
    maxRevisions: number;
    ts: number;
  }>;
}

/** Inline card block for a supervisor-dispatched flow sub-task. */
export interface FlowThreadBlock {
  kind: "flow_thread";
  id: string;
  /** Task id used for navigation. */
  taskId: string;
  /** The parent run_id that triggered the flow invocation. */
  parentRunId: string;
  flowId: string;
  /** Backend-generated flow run id (filled in once run starts). */
  flowRunId: string;
  /** Frontend-generated child session id for subscribing to flow events. */
  childSessionId: string;
  status: TaskStatus;
  /** Plain-English description of what the flow does (from invoke_flow context). */
  description: string;
  startedAt: number;
  endedAt: number | null;
  /** Kept for type compatibility with block iterators. */
  comments: Comment[];
}

export type Block = ConversationBlock | ShellBlock | FlowNotificationBlock | AgentThreadBlock | FlowThreadBlock;

// ── NodeConversation ──────────────────────────────────────────────────
// Represents the per-node agent conversation stream for a flow sub-run.
// Keyed by agentId (node name, e.g. "pm-design") in the conversations Map.

export type StatusKind = "pending" | "running" | "awaiting_review" | "succeeded" | "failed";

export interface NodeConversation {
  /** Authority run_id for this sub-run. */
  runId: string;
  /** Node agent name, e.g. "pm-design". */
  agentId: string;
  /** Flow run this sub-run belongs to. */
  flowRunId: string;
  status: StatusKind;
  contentStream: ContentSegment[];
  traceEntries: TraceEntry[];
  startedAt: number | null;
  /** Wall-clock timestamp (Date.now()) when the agent reached a terminal state. */
  endedAt: number | null;
}

// ── FlowThread ──────────────────────────────────────────────────────────────
// Inline thread attached to the ConversationBlock that triggered a flow run.
// Populated as flow node sub-runs stream events into the child session.

/** A single event from any flow node agent, keyed by sequence number. */
export interface FlowThreadEvent {
  agentId: string;
  kind: "token" | "tool_call" | "trace" | "status";
  segment?: ContentSegment;
  entry?: TraceEntry;
  /** Monotonically increasing within the child session. */
  seqNum: number;
  ts: number;
}

/** The flow thread attached to a fork block. */
export interface FlowThread {
  /** Backend-generated flow run id ("run-<uuid>"). Empty string until the first run_status event confirms it. */
  flowRunId: string;
  /** Frontend-generated child session id (crypto.randomUUID()). Known before any event arrives. */
  childSessionId: string;
  /** The flow name / flow_id (e.g. "feature-pipeline"). Known at dispatch time from state.selectedFlow. */
  flowId?: string;
  /** Chronologically ordered events from all node agents in this flow run. */
  events: FlowThreadEvent[];
  /**
   * Per-agent-run conversation streams, keyed by the authority run_id
   * (e.g. "run:79c5ac83-…"). Populated as token/trace/status events
   * arrive via the child-session subscription.
   */
  nodeConversations: Record<string, NodeConversation>;
  /**
   * Blackboard keys that were human-injected in this flow run (task 8.7).
   * Populated when the user manually injects a blackboard entry via the UI.
   */
  humanInjectedKeys: string[];
}

export type ActiveView = { kind: "main" } | { kind: "thread"; blockId: string; threadId: string };

/** Thread navigation destination for the two-level supervisor-session-ux nav. */
export type ThreadViewTarget = { taskId: string; kind: "agent" | "flow" };

export interface AgentSummary {
  name: string;
  kind: string;
  llm: string;
  label?: string;
  owning_ext?: string;
  description?: string;
  supports_models?: boolean;
  supports_modes?: boolean;
  supports_mcp?: boolean;
  /** ContributionKind for this agent — drives run-dispatch in the Rust runtime.
   * One of `cronymax.agents.builtin` / `cronymax.agents.workspace` /
   * `cronymax.agents.provider`. Omitted on legacy AgentSummary rows produced
   * before Phase 4. */
  contribution_kind?: string;
}

export function agentPickerDescription(agent: AgentSummary): string | undefined {
  if (agent.kind === "extension_provider") {
    const label = agent.label || agent.name;
    return agent.owning_ext ? `extension: ${label} · ${agent.owning_ext}` : `extension: ${label}`;
  }
  return agent.kind ? `kind: ${agent.kind}` : undefined;
}

export interface State {
  activeChatId: string | null;
  chatName: string;
  blocks: Block[];
  running: boolean;
  /** UUID of the block currently being streamed/run */
  runningBlockId: string | null;
  /** run_id of the currently active agent run — used by the Stop button */
  currentRunId: string | null;
  /** Terminal session id allocated for this chat tab */
  terminalTid: string | null;
  /** Pending prompt attachments (cleared on send) */
  attachments: Attachment[];
  /** Navigation state: main timeline or a specific thread */
  activeView: ActiveView;
  /** Supervisor-session-ux: thread view navigation (agent or flow sub-task). */
  threadView: ThreadViewTarget | null;
  /** Supervisor-session-ux: block id to keep pinned in the sticky active-thread bar. */
  pinnedBlockId: string | null;
  /** Selected model for new runs */
  model: string;
  /** Selected agent id for new runs */
  agentId: string;
  agents: AgentSummary[];
  flows: string[];
  selectedFlow: string;
  /** One-time migration notice when old v1 history was detected */
  migrationNotice: string | null;
  /** Non-null when the running agent is waiting for a tool approval decision */
  awaitingApproval: {
    runId: string;
    reviewId: string;
    toolName: string;
    args: unknown;
  } | null;
  /** True while crony is restarting and space subscriptions are being restored. */
  isReconnecting: boolean;
}

export type Action =
  | {
      type: "loadChat";
      id: string;
      name: string;
      blocks: Block[];
      terminalTid: string | null;
      model: string;
      agentId?: string;
      migrationNotice?: string;
    }
  | { type: "createBlock"; block: Block }
  | { type: "setAssistantContent"; id: string; content: string }
  | { type: "appendContentText"; id: string; delta: string }
  | { type: "appendToolCallSegment"; id: string; toolCallId: string; tool: string; args: unknown }
  | {
      type: "updateToolCallSegment";
      id: string;
      toolCallId: string;
      status: "done" | "error";
      result: unknown;
      durationMs?: number;
    }
  | { type: "appendThinkingSegment"; id: string; delta: string }
  | { type: "sealThinkingSegment"; id: string; elapsedMs: number }
  | { type: "appendTraceEntry"; id: string; entry: TraceEntry }
  | {
      type: "finalizeBlock";
      id: string;
      status: "ok" | "fail";
      agentName?: string;
    }
  | { type: "appendShellOutput"; id: string; chunk: string; now: number }
  | {
      type: "finalizeShellBlock";
      id: string;
      exitCode: number;
      now: number;
    }
  | { type: "setRunning"; running: boolean }
  | { type: "setRunningBlockId"; id: string | null }
  | { type: "setTerminalTid"; tid: string }
  | { type: "clearAttachments" }
  | { type: "clearPinnedComments" }
  | { type: "addAttachment"; attachment: Attachment }
  | { type: "removeAttachment"; id: string }
  | { type: "pinComment"; comment: Comment }
  | { type: "unpinComment"; commentId: string }
  | { type: "setModel"; model: string }
  | { type: "setAgentId"; agentId: string }
  | { type: "setActiveView"; view: ActiveView }
  | { type: "setThreadView"; target: ThreadViewTarget | null }
  | { type: "setPinnedBlockId"; blockId: string | null }
  | { type: "setAgents"; agents: AgentSummary[] }
  | { type: "setFlows"; flows: string[]; selected: string }
  | { type: "setSelectedFlow"; name: string }
  | { type: "clearMigrationNotice" }
  | {
      type: "setAwaitingApproval";
      runId: string;
      reviewId: string;
      toolName: string;
      args: unknown;
    }
  | { type: "clearAwaitingApproval" }
  | { type: "clearHistory" }
  | { type: "appendThinkingDelta"; id: string; delta: string; now: number }
  | { type: "sealThinkingBlock"; id: string; elapsedMs: number }
  | { type: "setReconnecting"; reconnecting: boolean }
  | { type: "setCurrentRunId"; runId: string | null }
  | { type: "appendFileChange"; id: string; change: FileChange }
  | { type: "restoreToBlock"; blockId: string }
  | {
      type: "createFlowNotification";
      id: string;
      message: string;
      variant: "success" | "info";
    }
  /** Attach a new FlowThread to a ConversationBlock (called when flow starts). */
  | { type: "attachFlowThread"; id: string; flowThread: FlowThread }
  /** Fill in the flowRunId once the first run_status event reveals it. */
  | { type: "setFlowThreadRunId"; id: string; flowRunId: string }
  /** Append an event from a flow node agent into the block's flowThread. */
  | { type: "appendFlowThreadEvent"; id: string; event: FlowThreadEvent }
  /** Create or update the status of a node conversation in the block's flowThread. */
  | {
      type: "flowNodeStatus";
      blockId: string;
      runId: string;
      agentId: string;
      status: StatusKind;
      flowRunId?: string;
      startedAt?: number;
    }
  /** Append a text token to a node conversation's content stream. */
  | { type: "flowNodeToken"; blockId: string; runId: string; delta: string }
  /** Append a thinking delta to a node conversation's content stream. */
  | { type: "flowNodeThinkingDelta"; blockId: string; runId: string; delta: string }
  /** Seal the open thinking segment of a node conversation. */
  | { type: "flowNodeSealThinking"; blockId: string; runId: string; elapsedMs: number }
  /** Add a tool_call segment to a node conversation's content stream. */
  | { type: "flowNodeToolCall"; blockId: string; runId: string; toolCallId: string; tool: string; args: unknown }
  /** Update the result/status of a tool_call in a node conversation. */
  | {
      type: "flowNodeToolDone";
      blockId: string;
      runId: string;
      toolCallId: string;
      status: "done" | "error";
      result: unknown;
      durationMs?: number;
    }
  /** Append a trace entry to a node conversation. */
  | { type: "flowNodeTrace"; blockId: string; runId: string; entry: TraceEntry }
  /** Update the status of an AgentThreadBlock or FlowThreadBlock. */
  | { type: "updateThreadBlockStatus"; id: string; status: TaskStatus; endedAt?: number }
  /** Fill in the flowRunId on a FlowThreadBlock once the run starts. */
  | { type: "setFlowThreadBlockRunId"; id: string; flowRunId: string }
  /** Insert a new AgentThreadBlock when invoke_agent fires (task 5.6). */
  | { type: "insertAgentThreadBlock"; block: AgentThreadBlock }
  /** Insert a new FlowThreadBlock when invoke_flow fires (task 5.6). */
  | { type: "insertFlowThreadBlock"; block: FlowThreadBlock }
  /** Record a human-injected blackboard key for a flow thread block (task 8.7). */
  | { type: "recordBlackboardInjection"; blockId: string; key: string }
  /** Link an agent thread block to its sub-run id (task 9.3). */
  | { type: "setAgentSubRunId"; agentName: string; subRunId: string }
  /** Append a critic result to a specific agent thread block (task 9.3). */
  | { type: "appendAgentCriticResult"; subRunId: string; result: AgentThreadBlock["criticResults"][number] }
  | { type: "_unused"; _placeholder?: never };

// ── Shell output processor ────────────────────────────────────────────
//
// Strips ANSI codes and accumulates clean text.
// Completion is detected in App.tsx via a nonce sentinel echoed after
// every wrapped command — no OSC 133 markers needed.

function applyShellOutput(block: ShellBlock, chunk: string, now: number): ShellBlock {
  const clean = stripAnsi(chunk);
  const output = block.output + clean;
  return { ...block, output, rawBuf: "", endedAt: block.endedAt ?? now };
}

// ── Initial state ──────────────────────────────────────────────────────

const initial: State = {
  activeChatId: null,
  chatName: "Chat",
  blocks: [],
  running: false,
  runningBlockId: null,
  currentRunId: null,
  terminalTid: null,
  attachments: [],
  activeView: { kind: "main" },
  threadView: null,
  pinnedBlockId: null,
  model: "",
  agentId: "",
  agents: [],
  flows: [],
  selectedFlow: "",
  migrationNotice: null,
  awaitingApproval: null,
  isReconnecting: false,
};

// ── Reducer ────────────────────────────────────────────────────────────

function reducer(state: State, action: Action): State {
  switch (action.type) {
    case "loadChat": {
      // Any block persisted with status:"running" belongs to a session that
      // was interrupted (runtime restart / renderer crash). Finalize them now
      // so the UI never shows a block stuck in "Thinking" forever.
      const sanitizedBlocks = action.blocks.map((b) => {
        if (b.kind === "conversation") {
          const conv = b as ConversationBlock;
          // Ensure contentStream exists (absent in blocks loaded before v4 migration).
          const withStream: ConversationBlock = conv.contentStream
            ? conv
            : {
                ...conv,
                contentStream: conv.assistantContent ? [{ kind: "text" as const, content: conv.assistantContent }] : [],
              };
          // Ensure fileChanges exists (absent in blocks created before this field was added).
          const withFileChanges: ConversationBlock = withStream.fileChanges
            ? withStream
            : { ...withStream, fileChanges: [] };
          if (withFileChanges.status === "running") {
            return {
              ...withFileChanges,
              status: "fail" as const,
              assistantContent: withFileChanges.assistantContent || "(session was interrupted — runtime restarted)",
            };
          }
          return withFileChanges;
        }
        if (b.kind === "shell" && b.status === "running") {
          return { ...b, status: "fail" as const, endedAt: Date.now() };
        }
        if (b.kind === "agent_thread") {
          // Ensure criticResults exists — field was added later; old persisted
          // blocks won't have it, causing `.length` to crash on render.
          return { ...b, criticResults: b.criticResults ?? [] };
        }
        return b;
      });
      return {
        ...state,
        activeChatId: action.id,
        chatName: action.name,
        blocks: sanitizedBlocks,
        // Reset in-flight run state — any previous run is gone after reload.
        running: false,
        runningBlockId: null,
        currentRunId: null,
        awaitingApproval: null,
        terminalTid: action.terminalTid,
        model: action.model || state.model,
        agentId: action.agentId ?? state.agentId,
        migrationNotice: action.migrationNotice ?? null,
        activeView: { kind: "main" },
      };
    }

    case "createBlock":
      return { ...state, blocks: [...state.blocks, action.block] };

    case "setAssistantContent": {
      const idx = state.blocks.findIndex((b) => b.id === action.id);
      if (idx < 0) return state;
      const blk = state.blocks[idx] as ConversationBlock;
      const next = state.blocks.slice();
      next[idx] = { ...blk, assistantContent: action.content };
      return { ...state, blocks: next };
    }

    case "appendContentText": {
      const idx = state.blocks.findIndex((b) => b.id === action.id);
      if (idx < 0) return state;
      const blk = state.blocks[idx] as ConversationBlock;
      const stream = blk.contentStream.slice();
      const last = stream[stream.length - 1];
      if (last?.kind === "text") {
        stream[stream.length - 1] = { ...last, content: last.content + action.delta };
      } else {
        stream.push({ kind: "text", content: action.delta });
      }
      const assistantContent = stream
        .filter((s): s is { kind: "text"; content: string } => s.kind === "text")
        .map((s) => s.content)
        .join("");
      const next = state.blocks.slice();
      next[idx] = { ...blk, contentStream: stream, assistantContent };
      return { ...state, blocks: next };
    }

    case "appendToolCallSegment": {
      const idx = state.blocks.findIndex((b) => b.id === action.id);
      if (idx < 0) return state;
      const blk = state.blocks[idx] as ConversationBlock;
      const segment: ContentSegment = {
        kind: "tool_call",
        toolCallId: action.toolCallId,
        tool: action.tool,
        args: action.args,
        status: "running",
      };
      const next = state.blocks.slice();
      next[idx] = { ...blk, contentStream: [...blk.contentStream, segment] };
      return { ...state, blocks: next };
    }

    case "updateToolCallSegment": {
      const idx = state.blocks.findIndex((b) => b.id === action.id);
      if (idx < 0) return state;
      const blk = state.blocks[idx] as ConversationBlock;
      const stream = blk.contentStream.map((s) => {
        if (s.kind === "tool_call" && s.toolCallId === action.toolCallId) {
          return {
            ...s,
            status: action.status,
            result: action.result,
            ...(action.durationMs != null ? { durationMs: action.durationMs } : {}),
          };
        }
        return s;
      });
      const next = state.blocks.slice();
      next[idx] = { ...blk, contentStream: stream };
      return { ...state, blocks: next };
    }

    case "appendThinkingSegment": {
      const idx = state.blocks.findIndex((b) => b.id === action.id);
      if (idx < 0) return state;
      const blk = state.blocks[idx] as ConversationBlock;
      const stream = blk.contentStream.slice();
      const last = stream[stream.length - 1];
      if (last?.kind === "thinking" && !last.sealed) {
        stream[stream.length - 1] = { ...last, content: last.content + action.delta };
      } else {
        stream.push({ kind: "thinking", content: action.delta, sealed: false, elapsedMs: 0 });
      }
      const next = state.blocks.slice();
      next[idx] = { ...blk, contentStream: stream };
      return { ...state, blocks: next };
    }

    case "sealThinkingSegment": {
      const idx = state.blocks.findIndex((b) => b.id === action.id);
      if (idx < 0) return state;
      const blk = state.blocks[idx] as ConversationBlock;
      let found = false;
      const stream = blk.contentStream
        .slice()
        .reverse()
        .map((s) => {
          if (!found && s.kind === "thinking" && !s.sealed) {
            found = true;
            return { ...s, sealed: true, elapsedMs: action.elapsedMs };
          }
          return s;
        })
        .reverse();
      const next = state.blocks.slice();
      next[idx] = { ...blk, contentStream: stream };
      return { ...state, blocks: next };
    }

    case "appendTraceEntry": {
      const idx = state.blocks.findIndex((b) => b.id === action.id);
      if (idx < 0) return state;
      const blk = state.blocks[idx] as ConversationBlock;
      const next = state.blocks.slice();
      next[idx] = { ...blk, traceEntries: [...blk.traceEntries, action.entry] };
      return { ...state, blocks: next };
    }

    case "finalizeBlock": {
      const idx = state.blocks.findIndex((b) => b.id === action.id);
      if (idx < 0) return state;
      const blk = state.blocks[idx] as ConversationBlock;
      const next = state.blocks.slice();
      next[idx] = {
        ...blk,
        status: action.status,
        ...(action.agentName ? { agentName: action.agentName } : {}),
      };
      return { ...state, blocks: next };
    }

    case "appendShellOutput": {
      const idx = state.blocks.findIndex((b) => b.id === action.id);
      if (idx < 0) return state;
      const blk = state.blocks[idx] as ShellBlock;
      const next = state.blocks.slice();
      next[idx] = applyShellOutput(blk, action.chunk, action.now);
      return { ...state, blocks: next };
    }

    case "finalizeShellBlock": {
      const idx = state.blocks.findIndex((b) => b.id === action.id);
      if (idx < 0) return state;
      const blk = state.blocks[idx] as ShellBlock;
      const next = state.blocks.slice();
      next[idx] = {
        ...blk,
        status: action.exitCode === 0 ? "ok" : "fail",
        exitCode: action.exitCode,
        endedAt: action.now,
      };
      return { ...state, blocks: next };
    }

    case "setRunning":
      return { ...state, running: action.running };

    case "setRunningBlockId":
      return { ...state, runningBlockId: action.id };

    case "setTerminalTid":
      return { ...state, terminalTid: action.tid };

    case "addAttachment":
      return {
        ...state,
        attachments: [...state.attachments, action.attachment],
      };

    case "removeAttachment":
      return {
        ...state,
        attachments: state.attachments.filter((a) => a.id !== action.id),
      };

    case "clearAttachments":
      return { ...state, attachments: [] };

    case "pinComment": {
      const { comment } = action;
      const attachment: Attachment = {
        id: `att-${comment.id}`,
        kind: "comment",
        label: comment.selectedText.slice(0, 60),
        commentId: comment.id,
        selectedText: comment.selectedText,
        commentText: comment.text,
      };
      // Add comment to its block
      const idx = state.blocks.findIndex((b) => b.id === comment.blockId);
      let nextBlocks = state.blocks;
      if (idx >= 0) {
        const blk = state.blocks[idx]!;
        const nextBlk = {
          ...blk,
          comments: [...(blk.comments ?? []), comment],
        };
        nextBlocks = state.blocks.slice();
        nextBlocks[idx] = nextBlk;
      }
      return {
        ...state,
        blocks: nextBlocks,
        attachments: [...state.attachments, attachment],
      };
    }

    case "unpinComment": {
      const nextAttachments = state.attachments.filter((a) => a.commentId !== action.commentId);
      const nextBlocks = state.blocks.map((blk) => ({
        ...blk,
        comments: blk.comments.map((c) => (c.id === action.commentId ? { ...c, pinnedToPrompt: false } : c)),
      }));
      return { ...state, blocks: nextBlocks, attachments: nextAttachments };
    }

    case "clearPinnedComments": {
      const nextAttachments = state.attachments.filter((a) => a.kind !== "comment");
      const nextBlocks = state.blocks.map((blk) => ({
        ...blk,
        comments: blk.comments.map((c) => ({ ...c, pinnedToPrompt: false })),
      }));
      return { ...state, blocks: nextBlocks, attachments: nextAttachments };
    }

    case "setModel":
      return { ...state, model: action.model };

    case "setAgentId":
      return { ...state, agentId: action.agentId };

    case "setActiveView":
      return { ...state, activeView: action.view };

    case "setThreadView":
      return { ...state, threadView: action.target };

    case "setPinnedBlockId":
      return { ...state, pinnedBlockId: action.blockId };

    case "setAgents":
      return {
        ...state,
        agents: action.agents,
      };

    case "setFlows":
      return { ...state, flows: action.flows, selectedFlow: action.selected };

    case "setSelectedFlow":
      return { ...state, selectedFlow: action.name };

    case "clearMigrationNotice":
      return { ...state, migrationNotice: null };

    case "setAwaitingApproval":
      return {
        ...state,
        awaitingApproval: {
          runId: action.runId,
          reviewId: action.reviewId,
          toolName: action.toolName,
          args: action.args,
        },
      };

    case "clearAwaitingApproval":
      return { ...state, awaitingApproval: null };

    case "setReconnecting":
      return { ...state, isReconnecting: action.reconnecting };

    case "appendThinkingDelta": {
      const idx = state.blocks.findIndex((b) => b.id === action.id);
      if (idx < 0) return state;
      const blk = state.blocks[idx] as ConversationBlock;
      const next = state.blocks.slice();
      next[idx] = {
        ...blk,
        thinkingText: blk.thinkingText + action.delta,
        thinkingStartedAt: blk.thinkingStartedAt ?? action.now,
      };
      return { ...state, blocks: next };
    }

    case "sealThinkingBlock": {
      const idx = state.blocks.findIndex((b) => b.id === action.id);
      if (idx < 0) return state;
      const blk = state.blocks[idx] as ConversationBlock;
      const next = state.blocks.slice();
      next[idx] = {
        ...blk,
        thinkingSealed: true,
        thinkingElapsedMs: action.elapsedMs,
      };
      return { ...state, blocks: next };
    }

    case "clearHistory":
      return { ...state, blocks: [], activeView: { kind: "main" } };

    case "setCurrentRunId":
      return { ...state, currentRunId: action.runId };

    case "appendFileChange": {
      const idx = state.blocks.findIndex((b) => b.id === action.id);
      if (idx < 0) return state;
      const blk = state.blocks[idx] as ConversationBlock;
      const next = state.blocks.slice();
      next[idx] = { ...blk, fileChanges: [...(blk.fileChanges ?? []), action.change] };
      return { ...state, blocks: next };
    }

    case "restoreToBlock": {
      const idx = state.blocks.findIndex((b) => b.id === action.blockId);
      if (idx < 0) return state;
      return { ...state, blocks: state.blocks.slice(0, idx + 1), activeView: { kind: "main" } };
    }

    case "createFlowNotification": {
      const notifBlock: FlowNotificationBlock = {
        kind: "flow-notification",
        id: action.id,
        message: action.message,
        variant: action.variant,
        ts: Date.now(),
        comments: [],
      };
      return { ...state, blocks: [...state.blocks, notifBlock] };
    }

    case "attachFlowThread": {
      return {
        ...state,
        blocks: state.blocks.map((b) =>
          b.id === action.id && b.kind === "conversation" ? { ...b, flowThread: action.flowThread } : b,
        ),
      };
    }

    case "setFlowThreadRunId": {
      return {
        ...state,
        blocks: state.blocks.map((b) => {
          if (b.id !== action.id || b.kind !== "conversation" || !b.flowThread) return b;
          return { ...b, flowThread: { ...b.flowThread, flowRunId: action.flowRunId } };
        }),
      };
    }

    case "appendFlowThreadEvent": {
      return {
        ...state,
        blocks: state.blocks.map((b) => {
          if (b.id !== action.id || b.kind !== "conversation" || !b.flowThread) return b;
          return {
            ...b,
            flowThread: {
              ...b.flowThread,
              events: [...b.flowThread.events, action.event],
            },
          };
        }),
      };
    }

    // ── Per-node-conversation helpers ──────────────────────────────────

    case "flowNodeStatus": {
      return {
        ...state,
        blocks: state.blocks.map((b) => {
          if (b.id !== action.blockId || b.kind !== "conversation" || !b.flowThread) return b;
          const existing = b.flowThread.nodeConversations[action.runId];
          const isTerminal = action.status === "succeeded" || action.status === "failed";
          const updated: NodeConversation = existing
            ? {
                ...existing,
                status: action.status,
                endedAt: isTerminal ? (existing.endedAt ?? Date.now()) : existing.endedAt,
              }
            : {
                runId: action.runId,
                agentId: action.agentId,
                flowRunId: action.flowRunId ?? b.flowThread.flowRunId,
                status: action.status,
                contentStream: [],
                traceEntries: [],
                startedAt: action.startedAt ?? Date.now(),
                endedAt: isTerminal ? Date.now() : null,
              };
          return {
            ...b,
            flowThread: {
              ...b.flowThread,
              nodeConversations: { ...b.flowThread.nodeConversations, [action.runId]: updated },
            },
          };
        }),
      };
    }

    case "flowNodeToken": {
      return {
        ...state,
        blocks: state.blocks.map((b) => {
          if (b.id !== action.blockId || b.kind !== "conversation" || !b.flowThread) return b;
          const nc = b.flowThread.nodeConversations[action.runId];
          if (!nc) return b;
          const stream = nc.contentStream.slice();
          const last = stream[stream.length - 1];
          if (last?.kind === "text") {
            stream[stream.length - 1] = { ...last, content: last.content + action.delta };
          } else {
            stream.push({ kind: "text", content: action.delta });
          }
          return {
            ...b,
            flowThread: {
              ...b.flowThread,
              nodeConversations: {
                ...b.flowThread.nodeConversations,
                [action.runId]: { ...nc, contentStream: stream },
              },
            },
          };
        }),
      };
    }

    case "flowNodeThinkingDelta": {
      return {
        ...state,
        blocks: state.blocks.map((b) => {
          if (b.id !== action.blockId || b.kind !== "conversation" || !b.flowThread) return b;
          const nc = b.flowThread.nodeConversations[action.runId];
          if (!nc) return b;
          const stream = nc.contentStream.slice();
          const last = stream[stream.length - 1];
          if (last?.kind === "thinking" && !last.sealed) {
            stream[stream.length - 1] = { ...last, content: last.content + action.delta };
          } else {
            stream.push({ kind: "thinking", content: action.delta, sealed: false, elapsedMs: 0 });
          }
          return {
            ...b,
            flowThread: {
              ...b.flowThread,
              nodeConversations: {
                ...b.flowThread.nodeConversations,
                [action.runId]: { ...nc, contentStream: stream },
              },
            },
          };
        }),
      };
    }

    case "flowNodeSealThinking": {
      return {
        ...state,
        blocks: state.blocks.map((b) => {
          if (b.id !== action.blockId || b.kind !== "conversation" || !b.flowThread) return b;
          const nc = b.flowThread.nodeConversations[action.runId];
          if (!nc) return b;
          let found = false;
          const stream = nc.contentStream
            .slice()
            .reverse()
            .map((s) => {
              if (!found && s.kind === "thinking" && !s.sealed) {
                found = true;
                return { ...s, sealed: true, elapsedMs: action.elapsedMs };
              }
              return s;
            })
            .reverse();
          return {
            ...b,
            flowThread: {
              ...b.flowThread,
              nodeConversations: {
                ...b.flowThread.nodeConversations,
                [action.runId]: { ...nc, contentStream: stream },
              },
            },
          };
        }),
      };
    }

    case "flowNodeToolCall": {
      return {
        ...state,
        blocks: state.blocks.map((b) => {
          if (b.id !== action.blockId || b.kind !== "conversation" || !b.flowThread) return b;
          const nc = b.flowThread.nodeConversations[action.runId];
          if (!nc) return b;
          const seg: ContentSegment = {
            kind: "tool_call",
            toolCallId: action.toolCallId,
            tool: action.tool,
            args: action.args,
            status: "running",
          };
          return {
            ...b,
            flowThread: {
              ...b.flowThread,
              nodeConversations: {
                ...b.flowThread.nodeConversations,
                [action.runId]: { ...nc, contentStream: [...nc.contentStream, seg] },
              },
            },
          };
        }),
      };
    }

    case "flowNodeToolDone": {
      return {
        ...state,
        blocks: state.blocks.map((b) => {
          if (b.id !== action.blockId || b.kind !== "conversation" || !b.flowThread) return b;
          const nc = b.flowThread.nodeConversations[action.runId];
          if (!nc) return b;
          const stream = nc.contentStream.map((s) => {
            if (s.kind === "tool_call" && s.toolCallId === action.toolCallId) {
              return {
                ...s,
                status: action.status,
                result: action.result,
                ...(action.durationMs != null ? { durationMs: action.durationMs } : {}),
              };
            }
            return s;
          });
          return {
            ...b,
            flowThread: {
              ...b.flowThread,
              nodeConversations: {
                ...b.flowThread.nodeConversations,
                [action.runId]: { ...nc, contentStream: stream },
              },
            },
          };
        }),
      };
    }

    case "flowNodeTrace": {
      return {
        ...state,
        blocks: state.blocks.map((b) => {
          if (b.id !== action.blockId || b.kind !== "conversation" || !b.flowThread) return b;
          const nc = b.flowThread.nodeConversations[action.runId];
          if (!nc) return b;
          return {
            ...b,
            flowThread: {
              ...b.flowThread,
              nodeConversations: {
                ...b.flowThread.nodeConversations,
                [action.runId]: { ...nc, traceEntries: [...nc.traceEntries, action.entry] },
              },
            },
          };
        }),
      };
    }

    case "updateThreadBlockStatus": {
      return {
        ...state,
        blocks: state.blocks.map((b) => {
          if (b.id !== action.id) return b;
          if (b.kind !== "agent_thread" && b.kind !== "flow_thread") return b;
          return {
            ...b,
            status: action.status,
            endedAt: action.endedAt ?? b.endedAt,
          };
        }),
      };
    }

    case "setFlowThreadBlockRunId": {
      return {
        ...state,
        blocks: state.blocks.map((b) => {
          if (b.id !== action.id || b.kind !== "flow_thread") return b;
          return { ...b, flowRunId: action.flowRunId };
        }),
      };
    }

    case "insertAgentThreadBlock": {
      // Deduplicate by taskId — only insert if not already present.
      if (state.blocks.some((b) => b.kind === "agent_thread" && b.taskId === action.block.taskId)) {
        return state;
      }
      // Ensure new fields are initialised even if caller didn't set them.
      const block: AgentThreadBlock = {
        ...action.block,
        subRunId: action.block.subRunId ?? null,
        criticResults: action.block.criticResults ?? [],
      };
      return { ...state, blocks: [...state.blocks, block] };
    }

    case "insertFlowThreadBlock": {
      // Deduplicate by taskId — only insert if not already present.
      if (state.blocks.some((b) => b.kind === "flow_thread" && b.taskId === action.block.taskId)) {
        return state;
      }
      return { ...state, blocks: [...state.blocks, action.block] };
    }

    case "recordBlackboardInjection": {
      return {
        ...state,
        blocks: state.blocks.map((b) => {
          if (b.kind !== "conversation" || b.id !== action.blockId || !b.flowThread) return b;
          const existing = b.flowThread.humanInjectedKeys;
          if (existing.includes(action.key)) return b;
          return {
            ...b,
            flowThread: {
              ...b.flowThread,
              humanInjectedKeys: [...existing, action.key],
            },
          };
        }),
      };
    }

    case "setAgentSubRunId": {
      return {
        ...state,
        blocks: state.blocks.map((b) => {
          if (b.kind !== "agent_thread" || b.agentName !== action.agentName || b.subRunId !== null) return b;
          return { ...b, subRunId: action.subRunId };
        }),
      };
    }

    case "appendAgentCriticResult": {
      return {
        ...state,
        blocks: state.blocks.map((b) => {
          if (b.kind !== "agent_thread" || b.subRunId !== action.subRunId) return b;
          return { ...b, criticResults: [...b.criticResults, action.result] };
        }),
      };
    }

    default:
      return state;
  }
}

export const { Provider, useStore } = createPanelStore<State, Action>(reducer, initial);

// ── localStorage helpers ───────────────────────────────────────────────

const chatsListKey = "chats";
const chatStorageKeyV4 = (id: string) => `chat_history_v4:${id}`;
const chatStorageKeyV3 = (id: string) => `chat_history_v3:${id}`;
const chatStorageKeyV2 = (id: string) => `chat_history_v2:${id}`;
const chatStorageKeyV1 = (id: string) => `chat_history:${id}`;

interface ChatListRow {
  id: string;
  name: string;
}

export interface PersistedChatData {
  blocks: Block[];
  terminalTid: string | null;
  model: string;
  agentId?: string;
}

// ── Content stream persistence helpers ────────────────────────────────

/**
 * Strips tool_call results from a contentStream before persisting.
 * Results live in traceEntries; they are re-hydrated on load.
 */
export function stripContentStreamResults(contentStream: ContentSegment[]): ContentSegment[] {
  return contentStream.map((s) => {
    if (s.kind === "tool_call") {
      // eslint-disable-next-line @typescript-eslint/no-unused-vars
      const { result: _r, ...rest } = s;
      void _r;
      return { ...rest, result: null };
    }
    return s;
  });
}

/**
 * Re-hydrates tool_call results in a contentStream from traceEntries.
 * Matches by toolCallId in a single O(n) pass.
 */
export function rehydrateContentStream(contentStream: ContentSegment[], traceEntries: TraceEntry[]): ContentSegment[] {
  const resultMap = new Map<string, unknown>();
  for (const entry of traceEntries) {
    if (entry.kind === "tool_done") {
      resultMap.set(entry.toolCallId, entry.result);
    }
  }
  return contentStream.map((s) => {
    if (s.kind === "tool_call" && s.result == null) {
      const result = resultMap.get(s.toolCallId);
      if (result !== undefined) {
        return { ...s, result };
      }
    }
    return s;
  });
}

/**
 * Reconstructs a contentStream from v3 traceEntries + assistantContent.
 * Used for v3→v4 migration. Approximation: text before tools within each turn.
 */
function reconstructContentStream(blk: ConversationBlock): ContentSegment[] {
  try {
    const toolDoneMap = new Map<string, unknown>();
    for (const e of blk.traceEntries) {
      if (e.kind === "tool_done") {
        toolDoneMap.set(e.toolCallId, e.result);
      }
    }

    const hasUsefulEntries = blk.traceEntries.some((e) => e.kind === "assistant_turn" || e.kind === "tool_start");

    if (hasUsefulEntries) {
      const stream: ContentSegment[] = [];
      for (const entry of blk.traceEntries) {
        if (entry.kind === "assistant_turn" && entry.text) {
          stream.push({ kind: "text", content: entry.text });
        } else if (entry.kind === "tool_start") {
          const result = toolDoneMap.get(entry.toolCallId);
          stream.push({
            kind: "tool_call",
            toolCallId: entry.toolCallId,
            tool: entry.tool,
            args: entry.args,
            status: "done",
            result: result ?? null,
          });
        }
      }
      if (stream.length > 0) return stream;
    }

    // Fallback: single text segment from assistantContent
    return blk.assistantContent ? [{ kind: "text", content: blk.assistantContent }] : [];
  } catch {
    return blk.assistantContent ? [{ kind: "text", content: blk.assistantContent }] : [];
  }
}

/** Migrates v3 PersistedChatData to v4 by reconstructing contentStream. */
function migrateV3ToV4(v3Data: PersistedChatData): PersistedChatData {
  const blocks = v3Data.blocks.map((b): Block => {
    if (b.kind !== "conversation") return b;
    const blk = b as ConversationBlock;
    // Already has contentStream — skip reconstruction
    if (blk.contentStream !== undefined && blk.contentStream !== null) return b;
    return { ...blk, contentStream: reconstructContentStream(blk) };
  });
  return { ...v3Data, blocks };
}

export function loadChatsList(): ChatListRow[] {
  try {
    return JSON.parse(localStorage.getItem(chatsListKey) || "[]");
  } catch {
    return [];
  }
}

export function loadChatData(id: string): {
  data: PersistedChatData;
  migrationNotice: string | undefined;
} {
  // Try v4 first
  try {
    const raw = localStorage.getItem(chatStorageKeyV4(id));
    if (raw) {
      const parsed = JSON.parse(raw) as PersistedChatData;
      // Re-hydrate tool_call results from traceEntries
      const rehydrated: PersistedChatData = {
        ...parsed,
        blocks: parsed.blocks.map((b) => {
          if (b.kind !== "conversation") return b;
          const conv = b as ConversationBlock;
          if (!conv.contentStream) return b;
          return {
            ...conv,
            contentStream: rehydrateContentStream(conv.contentStream, conv.traceEntries),
          };
        }),
      };
      return { data: rehydrated, migrationNotice: undefined };
    }
  } catch {
    /* fall through to v3 */
  }

  // Try v3 — migrate to v4
  try {
    const raw = localStorage.getItem(chatStorageKeyV3(id));
    if (raw) {
      const parsed = JSON.parse(raw) as PersistedChatData;
      const migrated = migrateV3ToV4(parsed);
      // Persist as v4, remove v3 key
      try {
        localStorage.setItem(chatStorageKeyV4(id), JSON.stringify(migrated));
        localStorage.removeItem(chatStorageKeyV3(id));
      } catch {
        /* ignore quota */
      }
      // Re-hydrate after migration (results are already present from v3 data)
      const rehydrated: PersistedChatData = {
        ...migrated,
        blocks: migrated.blocks.map((b) => {
          if (b.kind !== "conversation") return b;
          const conv = b as ConversationBlock;
          if (!conv.contentStream) return b;
          return {
            ...conv,
            contentStream: rehydrateContentStream(conv.contentStream, conv.traceEntries),
          };
        }),
      };
      return { data: rehydrated, migrationNotice: undefined };
    }
  } catch {
    /* fall through to v2 */
  }

  // Try v2 — strip traceContent, inject traceEntries: [], migrate to v4
  try {
    const raw = localStorage.getItem(chatStorageKeyV2(id));
    if (raw) {
      const parsed = JSON.parse(raw) as PersistedChatData;
      const v3migrated: PersistedChatData = {
        ...parsed,
        blocks: parsed.blocks.map((b) => {
          if (b.kind !== "conversation") return b;
          const { traceContent: _tc, ...rest } = b as ConversationBlock & { traceContent?: unknown };
          void _tc;
          return {
            ...rest,
            traceEntries: [],
            thinkingText: rest.thinkingText ?? "",
            thinkingSealed: rest.thinkingSealed ?? false,
            thinkingStartedAt: rest.thinkingStartedAt ?? null,
            thinkingElapsedMs: rest.thinkingElapsedMs ?? 0,
          } as ConversationBlock;
        }),
      };
      const migrated = migrateV3ToV4(v3migrated);
      localStorage.setItem(chatStorageKeyV4(id), JSON.stringify(migrated));
      localStorage.removeItem(chatStorageKeyV2(id));
      const rehydrated: PersistedChatData = {
        ...migrated,
        blocks: migrated.blocks.map((b) => {
          if (b.kind !== "conversation") return b;
          const conv = b as ConversationBlock;
          if (!conv.contentStream) return b;
          return {
            ...conv,
            contentStream: rehydrateContentStream(conv.contentStream, conv.traceEntries),
          };
        }),
      };
      return { data: rehydrated, migrationNotice: undefined };
    }
  } catch {
    /* fall through to v1 */
  }

  // Try v1 (old flat messages array)
  try {
    const raw = localStorage.getItem(chatStorageKeyV1(id));
    if (raw) {
      const oldMessages = JSON.parse(raw) as Array<{
        role: string;
        content: string;
        agentName?: string;
      }>;
      const blocks: ConversationBlock[] = [];
      let pendingUser: string | null = null;
      for (const m of oldMessages) {
        if (m.role === "user") {
          pendingUser = m.content;
        } else if (m.role === "assistant" && pendingUser !== null) {
          const assistantContent = m.content;
          blocks.push({
            kind: "conversation",
            id: crypto.randomUUID(),
            userContent: pendingUser,
            attachments: [],
            contentStream: assistantContent ? [{ kind: "text", content: assistantContent }] : [],
            assistantContent,
            agentName: m.agentName,
            traceEntries: [],
            fileChanges: [],
            status: "ok",
            comments: [],
            createdAt: Date.now(),
            thinkingText: "",
            thinkingSealed: false,
            thinkingStartedAt: null,
            thinkingElapsedMs: 0,
          });
          pendingUser = null;
        }
      }
      return {
        data: { blocks, terminalTid: null, model: "" },
        migrationNotice: "Your chat history was migrated to the new block format.",
      };
    }
  } catch {
    /* ignore */
  }

  return {
    data: { blocks: [], terminalTid: null, model: "" },
    migrationNotice: undefined,
  };
}

export function persistChatData(id: string, data: PersistedChatData): void {
  try {
    const safe: PersistedChatData = {
      ...data,
      blocks: data.blocks.map((b) => {
        if (b.kind === "shell") {
          // eslint-disable-next-line @typescript-eslint/no-unused-vars
          const { rawBuf: _rb, ...rest } = b;
          return { ...rest, rawBuf: "" };
        }
        if (b.kind === "conversation") {
          const conv = b as ConversationBlock;
          return {
            ...conv,
            contentStream: conv.contentStream ? stripContentStreamResults(conv.contentStream) : [],
          };
        }
        return b;
      }),
    };
    localStorage.setItem(chatStorageKeyV4(id), JSON.stringify(safe));
  } catch {
    /* ignore quota */
  }
}

export function ensureChat(): { id: string; name: string } {
  // Each browser tab (CEF BrowserView) owns its own sessionStorage, so
  // this acts as a per-tab slot. On first load (no session key) we create
  // a brand-new chat so that "New Chat" from the title bar always opens an
  // empty conversation instead of re-opening the previous one.
  const SESSION_KEY = "cronymax_chat_tab_id";
  const existingTabId = sessionStorage.getItem(SESSION_KEY);

  if (existingTabId) {
    // Tab already has a chat bound — restore it (create if somehow deleted)
    const list = loadChatsList();
    const found = list.find((c) => c.id === existingTabId);
    if (found) return found;
  }

  // Brand-new tab (or chat was deleted) — create a fresh chat entry
  const list = loadChatsList();
  const num = list.length + 1;
  const id = `c${Date.now().toString(36)}`;
  const row = { id, name: `Chat ${num}` };
  try {
    localStorage.setItem(chatsListKey, JSON.stringify([...list, row]));
    sessionStorage.setItem(SESSION_KEY, id);
  } catch {
    /* ignore */
  }
  return row;
}

export function chatNameFor(id: string): string {
  const list = loadChatsList();
  return list.find((c) => c.id === id)?.name || "Chat";
}

// ── flow helpers (kept for backwards-compat with other panels) ─────────
export type ChatMode = "agent" | "flow";

interface SavedFlowSpec {
  nodes: Array<{
    id: string | number;
    type: string;
    config?: Record<string, unknown>;
    x?: number;
    y?: number;
  }>;
  edges?: Array<{ from_id: string | number; to_id: string | number }>;
}

export function loadFlowsList(): { flows: string[]; selected: string } {
  let flowsObj: Record<string, unknown> = {};
  try {
    flowsObj = JSON.parse(localStorage.getItem("flows") || "{}") || {};
  } catch {
    /* ignore */
  }
  const names = Object.keys(flowsObj).sort();
  const stored = localStorage.getItem("chat_active_flow") || "";
  const selected = stored && names.includes(stored) ? stored : names.includes("Chat") ? "Chat" : (names[0] ?? "");
  return { flows: names, selected };
}

export function persistSelectedFlow(name: string): void {
  try {
    localStorage.setItem("chat_active_flow", name);
  } catch {
    /* ignore */
  }
}

export function loadSelectedAgent(agents: string[]): string {
  const stored = localStorage.getItem("chat_active_agent") || "";
  if (stored && agents.includes(stored)) return stored;
  if (agents.includes("Chat")) return "Chat";
  return agents[0] ?? "";
}

export function persistSelectedAgent(name: string): void {
  try {
    localStorage.setItem("chat_active_agent", name);
  } catch {
    /* ignore */
  }
}

export function loadChatMode(): ChatMode {
  const stored = localStorage.getItem("chat_mode");
  return stored === "flow" ? "flow" : "agent";
}

export function persistChatMode(mode: ChatMode): void {
  try {
    localStorage.setItem("chat_mode", mode);
  } catch {
    /* ignore */
  }
}

export function loadSavedGraph(selectedFlow: string): SavedFlowSpec | null {
  try {
    const flows: Record<string, SavedFlowSpec> = JSON.parse(localStorage.getItem("flows") || "{}");
    return flows[selectedFlow] ?? null;
  } catch {
    return null;
  }
}

export function loadSelectedModel(): string {
  return localStorage.getItem("chat_model") || "";
}

export function persistSelectedModel(model: string): void {
  try {
    localStorage.setItem("chat_model", model);
  } catch {
    /* ignore */
  }
}

/** Provider config stored alongside the selected model so run-time routing
 *  doesn't have to depend on the async-loaded modelGroups. */
export interface StoredModelProvider {
  id: string;
  kind: string;
  base_url: string;
  api_key: string;
  /** When the picked model belongs to an extension agent provider rather than
   * a configured LLM provider, these identify the agent to dispatch the run
   * to. base_url / api_key are unused for this case (extensions handle their
   * own auth). */
  agent_id?: string;
  contribution_kind?: string;
}

export function loadSelectedModelProvider(): StoredModelProvider | null {
  try {
    const raw = localStorage.getItem("chat_model_provider");
    if (!raw) return null;
    return JSON.parse(raw) as StoredModelProvider;
  } catch {
    return null;
  }
}

export function persistSelectedModelProvider(p: StoredModelProvider | null): void {
  try {
    if (p) localStorage.setItem("chat_model_provider", JSON.stringify(p));
    else localStorage.removeItem("chat_model_provider");
  } catch {
    /* ignore */
  }
}

/** Reasoning-effort override chosen in the chat toolbar. Empty = "use default". */
export type ReasoningEffort = "" | "minimal" | "low" | "medium" | "high" | "xhigh";

export function loadReasoningEffort(): ReasoningEffort {
  const v = localStorage.getItem("chat_reasoning_effort") || "";
  return v === "minimal" || v === "low" || v === "medium" || v === "high" || v === "xhigh" ? v : "";
}

export function persistReasoningEffort(v: ReasoningEffort): void {
  try {
    localStorage.setItem("chat_reasoning_effort", v);
  } catch {
    /* ignore */
  }
}

/** Anthropic adaptive-thinking effort override chosen in the chat toolbar.
 * Empty = "use server default" (typically "high"). */
export type AnthropicEffort = "" | "low" | "medium" | "high" | "max";

export function loadAnthropicEffort(): AnthropicEffort {
  const v = localStorage.getItem("chat_anthropic_effort") || "";
  return v === "low" || v === "medium" || v === "high" || v === "max" ? v : "";
}

export function persistAnthropicEffort(v: AnthropicEffort): void {
  try {
    localStorage.setItem("chat_anthropic_effort", v);
  } catch {
    /* ignore */
  }
}
