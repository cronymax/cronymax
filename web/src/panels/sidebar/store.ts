import { createPanelStore } from "@/hooks/usePanelStore";
import type { TabSummary } from "@/types";

/**
 * Sidebar state.
 *
 * `refine-cronymax-theme-layout` (per the design's Decision 6) collapses the
 * legacy three-list layout (tabs/terminals/chats with separate active ids
 * and a `panel: Panel` enum) into a single unified `tabs: TabSummary[]`
 * with one `activeTabId: string | null`. The native shell broadcasts a
 * `shell.tabs_list` snapshot that already knows about every kind, so the
 * sidebar just mirrors that into a flat list and asks the shell to
 * activate/close by id; clicks no longer drive a local "panel" mode at
 * all.
 *
 * Space management has moved to the native title-bar workspace selector
 * (CefMenuButton in MainWindow::BuildTitleBar). The sidebar no longer owns
 * the space dropdown.
 */
export interface State {
  tabs: TabSummary[];
  activeTabId: string | null;
}

export type Action =
  | { type: "setTabs"; tabs: TabSummary[]; activeId: string | null }
  | { type: "setActiveTab"; id: string | null };

const initial: State = {
  tabs: [],
  activeTabId: null,
};

function reducer(s: State, a: Action): State {
  switch (a.type) {
    case "setTabs":
      return { ...s, tabs: a.tabs, activeTabId: a.activeId };
    case "setActiveTab":
      return { ...s, activeTabId: a.id };
  }
}

export const { Provider, useStore } = createPanelStore<State, Action>(reducer, initial);
