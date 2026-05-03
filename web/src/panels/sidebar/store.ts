import { createPanelStore } from "@/hooks/usePanelStore";
import type { Space, TabSummary } from "@/types";

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
 */
export interface State {
  tabs: TabSummary[];
  activeTabId: string | null;
  spaces: Space[];
  activeSpaceId: string | null;
  activeSpaceName: string;
  spacesOpen: boolean;
}

export type Action =
  | { type: "setTabs"; tabs: TabSummary[]; activeId: string | null }
  | { type: "setActiveTab"; id: string | null }
  | { type: "setSpaces"; spaces: Space[] }
  | { type: "setActiveSpace"; id: string; name: string }
  | { type: "toggleSpaces"; open?: boolean };

const initial: State = {
  tabs: [],
  activeTabId: null,
  spaces: [],
  activeSpaceId: null,
  activeSpaceName: "Default",
  spacesOpen: false,
};

function reducer(s: State, a: Action): State {
  switch (a.type) {
    case "setTabs":
      return { ...s, tabs: a.tabs, activeTabId: a.activeId };
    case "setActiveTab":
      return { ...s, activeTabId: a.id };
    case "setSpaces":
      return { ...s, spaces: a.spaces };
    case "setActiveSpace":
      return { ...s, activeSpaceId: a.id, activeSpaceName: a.name };
    case "toggleSpaces":
      return { ...s, spacesOpen: a.open ?? !s.spacesOpen };
  }
}

export const { Provider, useStore } = createPanelStore<State, Action>(
  reducer,
  initial,
);
