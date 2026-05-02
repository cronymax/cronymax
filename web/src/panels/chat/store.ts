/**
 * Chat panel store — message history, run state, flow selection.
 */
import { createPanelStore } from "@/hooks/usePanelStore";

export type Role = "user" | "assistant" | "system" | "trace";

export interface Message {
  id: number;
  role: Role;
  content: string;
}

export interface State {
  activeChatId: string | null;
  chatName: string;
  messages: Message[];
  running: boolean;
  flows: string[];
  selectedFlow: string;
  msgSeq: number;
}

export type Action =
  | {
      type: "loadChat";
      id: string;
      name: string;
      history: Array<{ role: Role; content: string }>;
    }
  | { type: "addMessage"; role: Role; content: string }
  | { type: "updateMessage"; id: number; content: string }
  | { type: "appendToMessage"; id: number; chunk: string }
  | { type: "setRunning"; running: boolean }
  | { type: "clearHistory" }
  | { type: "setFlows"; flows: string[]; selected: string }
  | { type: "setSelectedFlow"; name: string };

const initial: State = {
  activeChatId: null,
  chatName: "Chat",
  messages: [],
  running: false,
  flows: [],
  selectedFlow: "",
  msgSeq: 1,
};

function reducer(state: State, action: Action): State {
  switch (action.type) {
    case "loadChat": {
      const messages: Message[] = action.history.map((m, i) => ({
        id: i + 1,
        role: m.role,
        content: m.content,
      }));
      return {
        ...state,
        activeChatId: action.id,
        chatName: action.name,
        messages,
        msgSeq: messages.length + 1,
      };
    }
    case "addMessage": {
      const msg: Message = {
        id: state.msgSeq,
        role: action.role,
        content: action.content,
      };
      return {
        ...state,
        messages: [...state.messages, msg],
        msgSeq: state.msgSeq + 1,
      };
    }
    case "updateMessage": {
      const idx = state.messages.findIndex((m) => m.id === action.id);
      if (idx < 0) return state;
      const next = state.messages.slice();
      next[idx] = { ...next[idx]!, content: action.content };
      return { ...state, messages: next };
    }
    case "appendToMessage": {
      const idx = state.messages.findIndex((m) => m.id === action.id);
      if (idx < 0) return state;
      const next = state.messages.slice();
      next[idx] = { ...next[idx]!, content: next[idx]!.content + action.chunk };
      return { ...state, messages: next };
    }
    case "setRunning":
      return { ...state, running: action.running };
    case "clearHistory":
      return { ...state, messages: [], msgSeq: 1 };
    case "setFlows":
      return { ...state, flows: action.flows, selectedFlow: action.selected };
    case "setSelectedFlow":
      return { ...state, selectedFlow: action.name };
    default:
      return state;
  }
}

export const { Provider, useStore } = createPanelStore<State, Action>(
  reducer,
  initial,
);

// ── localStorage helpers (kept here so App.tsx stays focused on rendering) ─
const chatsListKey = "chats";
const chatStorageKey = (id: string) => `chat_history:${id}`;

interface ChatListRow {
  id: string;
  name: string;
}

export function loadChatsList(): ChatListRow[] {
  try {
    return JSON.parse(localStorage.getItem(chatsListKey) || "[]");
  } catch {
    return [];
  }
}

export function loadHistory(
  id: string,
): Array<{ role: Role; content: string }> {
  try {
    return JSON.parse(localStorage.getItem(chatStorageKey(id)) || "[]");
  } catch {
    return [];
  }
}

export function persistHistory(
  id: string,
  history: Array<{ role: Role; content: string }>,
): void {
  try {
    localStorage.setItem(chatStorageKey(id), JSON.stringify(history));
  } catch {
    /* ignore quota */
  }
}

export function ensureChat(): { id: string; name: string } {
  const list = loadChatsList();
  if (list.length > 0 && list[0]) return list[0];
  const id = "c" + Date.now().toString(36);
  const row = { id, name: "Chat 1" };
  try {
    localStorage.setItem(chatsListKey, JSON.stringify([row]));
  } catch {
    /* ignore */
  }
  return row;
}

export function chatNameFor(id: string): string {
  const list = loadChatsList();
  return list.find((c) => c.id === id)?.name || "Chat";
}

// ── flow helpers ──────────────────────────────────────────────────────────
export function loadFlowsList(): {
  flows: string[];
  selected: string;
} {
  let flowsObj: Record<string, unknown> = {};
  try {
    flowsObj = JSON.parse(localStorage.getItem("flows") || "{}") || {};
  } catch {
    /* ignore */
  }
  const names = Object.keys(flowsObj).sort();
  const stored = localStorage.getItem("chat_active_flow") || "";
  const selected = stored && names.includes(stored) ? stored : "";
  return { flows: names, selected };
}

export function persistSelectedFlow(name: string): void {
  try {
    localStorage.setItem("chat_active_flow", name);
  } catch {
    /* ignore */
  }
}

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

export function loadSavedGraph(selectedFlow: string): SavedFlowSpec | null {
  try {
    const flows: Record<string, SavedFlowSpec> =
      JSON.parse(localStorage.getItem("flows") || "{}") || {};
    const fallback = localStorage.getItem("active_flow") || "";
    const name = selectedFlow || fallback;
    if (
      name &&
      flows[name] &&
      Array.isArray(flows[name]!.nodes) &&
      flows[name]!.nodes.length > 0
    ) {
      return flows[name]!;
    }
    const raw = localStorage.getItem("agent_graph");
    if (!raw) return null;
    const obj = JSON.parse(raw) as SavedFlowSpec | null;
    if (!obj || !Array.isArray(obj.nodes) || obj.nodes.length === 0) {
      return null;
    }
    return obj;
  } catch {
    return null;
  }
}
