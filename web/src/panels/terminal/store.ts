/**
 * Terminal panel store — thin state for xterm.js-based classic terminal.
 *
 * The old Wrap-style block renderer (Block, PaneState, applyOutput, stripAnsi)
 * has been moved to the chat panel store for $-mode shell blocks.
 * This store keeps only what the terminal panel itself needs.
 */
import { createPanelStore } from "@/hooks/usePanelStore";

export interface PaneState {
  /** True once terminal.start has been sent for this tid. */
  started: boolean;
}

export interface State {
  panes: Record<string, PaneState>;
  activeTid: string | null;
}

export type Action =
  | { type: "ensurePane"; tid: string }
  | { type: "setActive"; tid: string }
  | { type: "removePane"; tid: string }
  | { type: "markStarted"; tid: string };

function blankPane(): PaneState {
  return { started: false };
}

const initial: State = {
  panes: {},
  activeTid: null,
};

function reducer(state: State, action: Action): State {
  switch (action.type) {
    case "ensurePane": {
      if (state.panes[action.tid]) return state;
      return {
        ...state,
        panes: { ...state.panes, [action.tid]: blankPane() },
        activeTid: state.activeTid ?? action.tid,
      };
    }
    case "setActive": {
      const panes = state.panes[action.tid]
        ? state.panes
        : { ...state.panes, [action.tid]: blankPane() };
      return { ...state, panes, activeTid: action.tid };
    }
    case "removePane": {
      if (!state.panes[action.tid]) return state;
      const { [action.tid]: _, ...rest } = state.panes;
      const nextActive =
        state.activeTid === action.tid
          ? (Object.keys(rest)[0] ?? null)
          : state.activeTid;
      return { ...state, panes: rest, activeTid: nextActive };
    }
    case "markStarted": {
      const p = state.panes[action.tid];
      if (!p || p.started) return state;
      return {
        ...state,
        panes: { ...state.panes, [action.tid]: { ...p, started: true } },
      };
    }
    default:
      return state;
  }
}

export const { Provider, useStore } = createPanelStore<State, Action>(
  reducer,
  initial,
);
