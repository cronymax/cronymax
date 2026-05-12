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

export interface ConversationBlock {
  kind: "conversation";
  id: string;
  /** The prompt sent by user (including @-mentions, comments, etc.) */
  userContent: string;
  /** Attachment snapshots included in this prompt */
  attachments: Attachment[];
  /** Streamed assistant response */
  assistantContent: string;
  agentName?: string;
  traceEntries: TraceEntry[];
  /** "running" while streaming, "ok" or "fail" after final run_status */
  status: "running" | "ok" | "fail";
  comments: Comment[];
  thread?: Thread;
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

export type Block = ConversationBlock | ShellBlock;

export type ActiveView =
  | { kind: "main" }
  | { kind: "thread"; blockId: string; threadId: string };

export interface AgentSummary {
  name: string;
  kind: string;
  llm: string;
}

export interface State {
  activeChatId: string | null;
  chatName: string;
  blocks: Block[];
  running: boolean;
  /** UUID of the block currently being streamed/run */
  runningBlockId: string | null;
  /** Terminal session id allocated for this chat tab */
  terminalTid: string | null;
  /** Pending prompt attachments (cleared on send) */
  attachments: Attachment[];
  /** Navigation state: main timeline or a specific thread */
  activeView: ActiveView;
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
  | { type: "sealThinkingBlock"; id: string; elapsedMs: number };

// ── Shell output processor ────────────────────────────────────────────
//
// Strips ANSI codes and accumulates clean text.
// Completion is detected in App.tsx via a nonce sentinel echoed after
// every wrapped command — no OSC 133 markers needed.

function applyShellOutput(
  block: ShellBlock,
  chunk: string,
  now: number,
): ShellBlock {
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
  terminalTid: null,
  attachments: [],
  activeView: { kind: "main" },
  model: "",
  agentId: "",
  agents: [],
  flows: [],
  selectedFlow: "",
  migrationNotice: null,
  awaitingApproval: null,
};

// ── Reducer ────────────────────────────────────────────────────────────

function reducer(state: State, action: Action): State {
  switch (action.type) {
    case "loadChat":
      return {
        ...state,
        activeChatId: action.id,
        chatName: action.name,
        blocks: action.blocks,
        terminalTid: action.terminalTid,
        model: action.model || state.model,
        agentId: action.agentId ?? state.agentId,
        migrationNotice: action.migrationNotice ?? null,
        activeView: { kind: "main" },
      };

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
        id: "att-" + comment.id,
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
      const nextAttachments = state.attachments.filter(
        (a) => a.commentId !== action.commentId,
      );
      const nextBlocks = state.blocks.map((blk) => ({
        ...blk,
        comments: blk.comments.map((c) =>
          c.id === action.commentId ? { ...c, pinnedToPrompt: false } : c,
        ),
      }));
      return { ...state, blocks: nextBlocks, attachments: nextAttachments };
    }

    case "clearPinnedComments": {
      const nextAttachments = state.attachments.filter(
        (a) => a.kind !== "comment",
      );
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

    default:
      return state;
  }
}

export const { Provider, useStore } = createPanelStore<State, Action>(
  reducer,
  initial,
);

// ── localStorage helpers ───────────────────────────────────────────────

const chatsListKey = "chats";
const chatStorageKeyV3 = (id: string) => `chat_history_v3:${id}`;
const chatStorageKeyV2 = (id: string) => `chat_history_v2:${id}`;
const chatStorageKeyV1 = (id: string) => `chat_history:${id}`;

interface ChatListRow {
  id: string;
  name: string;
}

interface PersistedChatData {
  blocks: Block[];
  terminalTid: string | null;
  model: string;
  agentId?: string;
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
  // Try v3 first
  try {
    const raw = localStorage.getItem(chatStorageKeyV3(id));
    if (raw) {
      const parsed = JSON.parse(raw) as PersistedChatData;
      return { data: parsed, migrationNotice: undefined };
    }
  } catch {
    /* fall through to v2 */
  }

  // Try v2 — strip traceContent, inject traceEntries: [], migrate to v3
  try {
    const raw = localStorage.getItem(chatStorageKeyV2(id));
    if (raw) {
      const parsed = JSON.parse(raw) as PersistedChatData;
      const migrated: PersistedChatData = {
        ...parsed,
        blocks: parsed.blocks.map((b) => {
          if (b.kind !== "conversation") return b;
          // eslint-disable-next-line @typescript-eslint/no-explicit-any
          const { traceContent: _tc, ...rest } = b as any;
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
      localStorage.setItem(chatStorageKeyV3(id), JSON.stringify(migrated));
      localStorage.removeItem(chatStorageKeyV2(id));
      return { data: migrated, migrationNotice: undefined };
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
          blocks.push({
            kind: "conversation",
            id: crypto.randomUUID(),
            userContent: pendingUser,
            attachments: [],
            assistantContent: m.content,
            agentName: m.agentName,
            traceEntries: [],
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
        migrationNotice:
          "Your chat history was migrated to the new block format.",
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
        return b;
      }),
    };
    localStorage.setItem(chatStorageKeyV3(id), JSON.stringify(safe));
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
  const id = "c" + Date.now().toString(36);
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
  const selected =
    stored && names.includes(stored)
      ? stored
      : names.includes("Chat")
        ? "Chat"
        : (names[0] ?? "");
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
    const flows: Record<string, SavedFlowSpec> = JSON.parse(
      localStorage.getItem("flows") || "{}",
    );
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
