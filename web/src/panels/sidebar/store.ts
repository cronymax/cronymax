import { createPanelStore } from "@/hooks/usePanelStore";
import type { BrowserTab, Space, TerminalRow } from "@/types";

export type Panel =
  | "browser"
  | "terminal"
  | "agent"
  | "graph"
  | "chat"
  | "config";

export interface ChatRow {
  id: string;
  name: string;
}

export interface State {
  tabs: BrowserTab[];
  activeTabId: number | null;
  terminals: TerminalRow[];
  activeTerminalId: string | null;
  chats: ChatRow[];
  activeChatId: string | null;
  spaces: Space[];
  activeSpaceId: string | null;
  activeSpaceName: string;
  panel: Panel;
  spacesOpen: boolean;
}

export type Action =
  | { type: "setTabs"; tabs: BrowserTab[]; activeId: number | null }
  | { type: "addTab"; tab: BrowserTab }
  | { type: "closeTab"; id: number }
  | { type: "updateTab"; id: number; patch: Partial<BrowserTab> }
  | { type: "setActiveTab"; id: number | null }
  | { type: "setTerminals"; items: TerminalRow[]; active: string | null }
  | { type: "addTerminal"; item: TerminalRow }
  | { type: "removeTerminal"; id: string }
  | { type: "setActiveTerminal"; id: string | null }
  | { type: "setChats"; chats: ChatRow[] }
  | { type: "addChat"; chat: ChatRow }
  | { type: "removeChat"; id: string }
  | { type: "setActiveChat"; id: string | null }
  | { type: "setSpaces"; spaces: Space[] }
  | { type: "setActiveSpace"; id: string; name: string }
  | { type: "setPanel"; panel: Panel }
  | { type: "toggleSpaces"; open?: boolean };

const initial: State = {
  tabs: [],
  activeTabId: null,
  terminals: [],
  activeTerminalId: null,
  chats: loadChats(),
  activeChatId: null,
  spaces: [],
  activeSpaceId: null,
  activeSpaceName: "Default",
  panel: "browser",
  spacesOpen: false,
};

function loadChats(): ChatRow[] {
  try {
    return JSON.parse(localStorage.getItem("chats") || "[]") as ChatRow[];
  } catch {
    return [];
  }
}

export function saveChats(chats: ChatRow[]) {
  try {
    localStorage.setItem("chats", JSON.stringify(chats));
  } catch {
    // ignore
  }
}

function reducer(s: State, a: Action): State {
  switch (a.type) {
    case "setTabs":
      return { ...s, tabs: a.tabs, activeTabId: a.activeId };
    case "addTab":
      if (s.tabs.some((t) => t.id === a.tab.id)) return s;
      return {
        ...s,
        tabs: [...s.tabs, a.tab],
        activeTabId: a.tab.id,
        panel: "browser",
      };
    case "closeTab": {
      const tabs = s.tabs.filter((t) => t.id !== a.id);
      const activeTabId =
        s.activeTabId === a.id ? (tabs[0]?.id ?? null) : s.activeTabId;
      return { ...s, tabs, activeTabId };
    }
    case "updateTab":
      return {
        ...s,
        tabs: s.tabs.map((t) => (t.id === a.id ? { ...t, ...a.patch } : t)),
      };
    case "setActiveTab":
      return { ...s, activeTabId: a.id, panel: "browser" };
    case "setTerminals":
      return { ...s, terminals: a.items, activeTerminalId: a.active };
    case "addTerminal":
      if (s.terminals.some((t) => t.id === a.item.id)) return s;
      return { ...s, terminals: [...s.terminals, a.item] };
    case "removeTerminal": {
      const terminals = s.terminals.filter((t) => t.id !== a.id);
      const activeTerminalId =
        s.activeTerminalId === a.id
          ? (terminals[0]?.id ?? null)
          : s.activeTerminalId;
      return { ...s, terminals, activeTerminalId };
    }
    case "setActiveTerminal":
      return { ...s, activeTerminalId: a.id };
    case "setChats":
      return { ...s, chats: a.chats };
    case "addChat":
      return { ...s, chats: [...s.chats, a.chat], activeChatId: a.chat.id };
    case "removeChat": {
      const chats = s.chats.filter((c) => c.id !== a.id);
      const activeChatId =
        s.activeChatId === a.id ? (chats[0]?.id ?? null) : s.activeChatId;
      return { ...s, chats, activeChatId };
    }
    case "setActiveChat":
      return { ...s, activeChatId: a.id };
    case "setSpaces":
      return { ...s, spaces: a.spaces };
    case "setActiveSpace":
      return { ...s, activeSpaceId: a.id, activeSpaceName: a.name };
    case "setPanel":
      return {
        ...s,
        panel: a.panel,
        activeTabId: a.panel === "browser" ? s.activeTabId : null,
      };
    case "toggleSpaces":
      return { ...s, spacesOpen: a.open ?? !s.spacesOpen };
  }
}

export const { Provider, useStore } = createPanelStore<State, Action>(
  reducer,
  initial,
);
