import { useCallback, useEffect, useState } from "react";
import { browser } from "@/shells/bridge";
import { useBridgeEvent } from "@/hooks/useBridgeEvent";
import { useDragRegions } from "@/hooks/useDragRegions";
import { Icon, type IconName } from "@/components/Icon";
import type { TabKind, TabSummary } from "@/types";
import { useStore } from "./store";
import { ProfilePickerOverlay } from "@/components/ProfilePickerOverlay";

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

function iconNameForKind(kind: TabKind): IconName {
  switch (kind) {
    case "terminal":
      return "terminal";
    case "chat":
      return "comment-discussion";
    case "agent":
      return "settings-gear";
    case "graph":
      return "type-hierarchy";
    case "web":
    default:
      return "globe";
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
          ? "bg-cronymax-active text-cronymax-title"
          : "text-cronymax-caption hover:bg-cronymax-hover hover:text-cronymax-title")
      }
    >
      <span className="flex h-3.5 w-3.5 flex-none items-center justify-center">
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
          <Icon name={iconNameForKind(tab.kind)} size={14} aria-hidden="true" />
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
        aria-label="Close"
      >
        <Icon name="close" size={12} aria-hidden="true" />
      </button>
    </li>
  );
}

export function App() {
  const dragRef = useDragRegions("sidebar");
  const [state, dispatch] = useStore();
  const { tabs, activeTabId } = state;
  const [switching, setSwitching] = useState(false);

  // ── Initial load ───────────────────────────────────────────────────
  useEffect(() => {
    void (async () => {
      try {
        const snap = await browser.send("shell.tabs_list");
        dispatch({
          type: "setTabs",
          tabs: snap.tabs ?? [],
          activeId: snap.activeTabId ?? null,
        });
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
  useBridgeEvent("space.switch_loading", ({ loading }) =>
    setSwitching(loading),
  );

  // ── Actions ────────────────────────────────────────────────────────
  const activate = useCallback(async (tab: TabSummary) => {
    try {
      await browser.send("shell.tab_switch", { id: tab.id });
    } catch (e) {
      console.warn("shell.tab_switch failed", e);
    }
  }, []);

  const close = useCallback(async (tab: TabSummary) => {
    try {
      await browser.send("shell.tab_close", { id: tab.id });
    } catch (e) {
      console.warn("shell.tab_close failed", e);
    }
  }, []);

  return (
    <>
      <ProfilePickerOverlay />
      <aside
        ref={dragRef as React.RefObject<HTMLElement>}
        className="app-drag flex h-full flex-col bg-cronymax-body pt-7 text-cronymax-title"
      >
        {/* Items section */}
        <section className="no-drag flex-1 overflow-auto px-2 pb-4 pt-2">
          {switching && (
            <div className="mb-2 rounded bg-cronymax-float px-2 py-1 text-[11px] text-cronymax-caption">
              Restarting runtime…
            </div>
          )}
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
    </>
  );
}
