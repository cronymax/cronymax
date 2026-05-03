import { useCallback, useEffect } from "react";
import { bridge } from "@/bridge";
import { useBridgeEvent } from "@/hooks/useBridgeEvent";
import { useDragRegions } from "@/hooks/useDragRegions";
import type { TabKind, TabSummary } from "@/types";
import { useStore } from "./store";

/**
 * Sidebar — unified tab list.
 *
 * Subscribes to `shell.tabs_list` (snapshot) and `shell.tab_activated`
 * (focus change). Clicking a row dispatches `shell.tab_switch`; the close
 * button dispatches `shell.tab_close`. There is no local notion of
 * "active panel" — the native side is the source of truth and the only
 * thing that swaps the visible content card.
 */

function faviconFor(url?: string): string | null {
  if (!url) return null;
  try {
    const host = new URL(url).hostname;
    if (host) return `https://www.google.com/s2/favicons?domain=${host}&sz=16`;
  } catch {
    // ignore
  }
  return null;
}

function glyphFor(kind: TabKind): string {
  switch (kind) {
    case "terminal":
      return "⌨";
    case "chat":
      return "💬";
    case "agent":
      return "⚙";
    case "graph":
      return "▦";
    case "web":
    default:
      return "🌐";
  }
}

function Row({
  tab,
  active,
  onActivate,
  onClose,
}: {
  tab: TabSummary;
  active: boolean;
  onActivate: () => void;
  onClose: () => void;
}) {
  const iconUrl =
    tab.kind === "web" ? (tab.favicon ?? faviconFor(tab.url)) : null;
  return (
    <li
      onClick={onActivate}
      className={
        "no-drag group flex h-7 cursor-pointer items-center gap-2 rounded-md px-2 text-xs " +
        (active
          ? "bg-cronymax-float text-cronymax-title"
          : "text-cronymax-caption hover:bg-cronymax-float hover:text-cronymax-title")
      }
    >
      <span className="flex h-3.5 w-3.5 flex-none items-center justify-center text-[11px]">
        {iconUrl ? (
          <img
            src={iconUrl}
            width={14}
            height={14}
            className="rounded-sm"
            onError={(e) => {
              (e.target as HTMLImageElement).style.display = "none";
            }}
          />
        ) : (
          glyphFor(tab.kind)
        )}
      </span>
      <span className="flex-1 truncate">{tab.displayName}</span>
      <button
        type="button"
        title="Close"
        onMouseDown={(e) => {
          // Prevent the row's onClick from firing on the same gesture.
          e.stopPropagation();
        }}
        onClick={(e) => {
          e.stopPropagation();
          e.preventDefault();
          onClose();
        }}
        className="flex h-4 w-4 flex-none items-center justify-center rounded text-cronymax-caption opacity-60 hover:bg-cronymax-border hover:text-white hover:opacity-100"
      >
        ×
      </button>
    </li>
  );
}

export function App() {
  const dragRef = useDragRegions("sidebar");
  const [state, dispatch] = useStore();
  const {
    tabs,
    activeTabId,
    spaces,
    activeSpaceId,
    activeSpaceName,
    spacesOpen,
  } = state;

  // ── Initial load ───────────────────────────────────────────────────
  useEffect(() => {
    void (async () => {
      try {
        const snap = await bridge.send("shell.tabs_list");
        dispatch({
          type: "setTabs",
          tabs: snap.tabs ?? [],
          activeId: snap.activeTabId ?? null,
        });
      } catch {
        // ignore
      }
      try {
        const sp = await bridge.send("space.list");
        dispatch({ type: "setSpaces", spaces: sp });
        if (sp.length > 0) {
          dispatch({
            type: "setActiveSpace",
            id: sp[0]!.id,
            name: sp[0]!.name,
          });
        }
      } catch {
        // ignore
      }
    })();
  }, [dispatch]);

  // ── Push events ────────────────────────────────────────────────────
  useBridgeEvent("shell.tabs_list", (snap) =>
    dispatch({
      type: "setTabs",
      tabs: snap.tabs ?? [],
      activeId: snap.activeTabId ?? null,
    }),
  );
  useBridgeEvent("shell.tab_activated", (p) =>
    dispatch({ type: "setActiveTab", id: p.tabId }),
  );
  useBridgeEvent("shell.space_changed", (p) =>
    dispatch({ type: "setActiveSpace", id: p.id, name: p.name }),
  );

  // Close spaces dropdown on outside click.
  useEffect(() => {
    if (!spacesOpen) return;
    const close = () => dispatch({ type: "toggleSpaces", open: false });
    document.addEventListener("click", close);
    return () => document.removeEventListener("click", close);
  }, [spacesOpen, dispatch]);

  // ── Actions ────────────────────────────────────────────────────────
  const activate = useCallback(async (tab: TabSummary) => {
    try {
      await bridge.send("shell.tab_switch", { id: tab.id });
    } catch (e) {
      console.warn("shell.tab_switch failed", e);
    }
  }, []);

  const close = useCallback(async (tab: TabSummary) => {
    try {
      await bridge.send("shell.tab_close", { id: tab.id });
    } catch (e) {
      console.warn("shell.tab_close failed", e);
    }
  }, []);

  // ── Spaces ─────────────────────────────────────────────────────────
  const refreshSpaces = useCallback(async () => {
    try {
      const sp = await bridge.send("space.list");
      dispatch({ type: "setSpaces", spaces: sp });
    } catch {
      // ignore
    }
  }, [dispatch]);

  const switchSpace = useCallback(
    async (id: string, name: string) => {
      try {
        await bridge.send("space.switch", { space_id: id });
        dispatch({ type: "setActiveSpace", id, name });
        dispatch({ type: "toggleSpaces", open: false });
      } catch (e) {
        console.warn("space.switch failed", e);
      }
    },
    [dispatch],
  );

  const createSpace = useCallback(async () => {
    const name = prompt("Space name:");
    if (!name) return;
    const path = prompt("Workspace path (leave blank for current):", "");
    try {
      await bridge.send("space.create", {
        name,
        root_path: path || ".",
      });
      void refreshSpaces();
    } catch (e) {
      console.warn("space.create failed", e);
    }
  }, [refreshSpaces]);

  return (
    <aside
      ref={dragRef as React.RefObject<HTMLElement>}
      className="app-drag flex h-full flex-col bg-cronymax-body pt-7 text-cronymax-title"
    >
      {/* Space header */}
      <div className="no-drag relative flex items-center gap-2 px-3 py-2.5">
        <span className="h-2.5 w-2.5 flex-none rounded-full bg-cronymax-primary" />
        <span className="flex-1 truncate text-sm font-medium">
          {activeSpaceName}
        </span>
        <button
          type="button"
          title="Switch Space"
          onClick={(e) => {
            e.stopPropagation();
            const next = !spacesOpen;
            dispatch({ type: "toggleSpaces", open: next });
            if (next) void refreshSpaces();
          }}
          className="flex h-5 w-5 items-center justify-center rounded text-cronymax-caption hover:bg-cronymax-float hover:text-white"
        >
          ▾
        </button>
        {spacesOpen && (
          <div
            onClick={(e) => e.stopPropagation()}
            className="absolute left-3 right-3 top-full z-10 mt-1 rounded-lg border border-cronymax-border bg-cronymax-base p-1 shadow-cronymax-elev-2"
          >
            <ul>
              {spaces.map((sp) => (
                <li
                  key={sp.id}
                  onClick={() => void switchSpace(sp.id, sp.name)}
                  className={
                    "cursor-pointer rounded px-2 py-1.5 text-xs hover:bg-cronymax-float " +
                    (sp.id === activeSpaceId
                      ? "text-cronymax-title"
                      : "text-cronymax-caption")
                  }
                >
                  {sp.name}
                </li>
              ))}
            </ul>
            <button
              type="button"
              onClick={() => {
                dispatch({ type: "toggleSpaces", open: false });
                void createSpace();
              }}
              className="mt-1 w-full rounded px-2 py-1.5 text-left text-xs text-cronymax-secondary hover:bg-cronymax-float"
            >
              + New Space
            </button>
          </div>
        )}
      </div>

      {/* Items section */}
      <section className="no-drag flex-1 overflow-auto px-2 pb-4">
        <div className="no-drag px-2 pb-1 text-[10px] font-semibold uppercase tracking-wider text-cronymax-caption">
          Tabs
        </div>
        <ul className="no-drag space-y-0.5">
          {tabs.map((t) => (
            <Row
              key={t.id}
              tab={t}
              active={t.id === activeTabId}
              onActivate={() => void activate(t)}
              onClose={() => void close(t)}
            />
          ))}
        </ul>
      </section>
    </aside>
  );
}
