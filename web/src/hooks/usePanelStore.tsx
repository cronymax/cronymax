import {
  type Context,
  createContext,
  type Dispatch,
  type ReactNode,
  type Reducer,
  useContext,
  useReducer,
} from "react";

/**
 * Build a small typed `useReducer` + Context store. Returns:
 *   - Provider — wraps the panel's tree
 *   - useStore — returns [state, dispatch]
 *   - useState  — returns just state
 *   - useDispatch — returns just dispatch
 *
 * Pattern:
 *
 *   const { Provider, useStore } = createPanelStore(reducer, initial);
 *
 * Cross-panel sync is NOT a goal — each panel is its own React tree and
 * its own CEF BrowserView. Use bridge events for inter-panel coordination.
 */
export function createPanelStore<S, A>(reducer: Reducer<S, A>, initial: S) {
  const Ctx = createContext<[S, Dispatch<A>] | null>(null);
  Ctx.displayName = "PanelStore";

  function Provider({ children }: { children: ReactNode }) {
    const value = useReducer(reducer, initial);
    return <Ctx.Provider value={value}>{children}</Ctx.Provider>;
  }

  function useStore(): [S, Dispatch<A>] {
    const v = useContext(Ctx);
    if (!v) throw new Error("useStore must be used inside Provider");
    return v;
  }
  function useState(): S {
    return useStore()[0];
  }
  function useDispatch(): Dispatch<A> {
    return useStore()[1];
  }

  return { Context: Ctx as Context<[S, Dispatch<A>] | null>, Provider, useStore, useState, useDispatch };
}
