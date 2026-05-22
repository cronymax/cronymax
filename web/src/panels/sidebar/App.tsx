import { useCallback, useEffect, useRef, useState } from "react";
import { Icon, type IconName } from "@/components/Icon";
import { useBridgeEvent } from "@/hooks/useBridgeEvent";
import { useDragRegions } from "@/hooks/useDragRegions";
import { shells } from "@/shells/bridge";
import { session as runtimeSession } from "@/shells/runtime";
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
  const iconUrl = tab.kind === "web" ? (tab.favicon ?? faviconFor(tab.url)) : null;
  const [renaming, setRenaming] = useState(false);
  const [renameValue, setRenameValue] = useState("");
  const renameRef = useRef<HTMLInputElement>(null);

  // For chat tabs: prefer sessionTitle if set, fall back to displayName.
  const chatTab = tab.kind === "chat" ? tab : null;
  const label = chatTab?.sessionTitle ?? tab.displayName;
  const excerpt = chatTab?.firstMessageExcerpt ?? null;

  function startRename() {
    if (tab.kind !== "chat") return;
    setRenameValue(label);
    setRenaming(true);
    setTimeout(() => renameRef.current?.select(), 0);
  }

  async function commitRename() {
    if (!renaming) return;
    setRenaming(false);
    const trimmed = renameValue.trim();
    if (!trimmed || trimmed === label) return;
    try {
      await runtimeSession.rename(tab.id, trimmed);
    } catch (e) {
      console.warn("session.rename failed", e);
    }
  }

  return (
    <li
      onClick={onActivate}
      onDoubleClick={(e) => {
        e.stopPropagation();
        startRename();
      }}
      className={
        "no-drag group flex min-h-7 cursor-pointer items-center gap-2 rounded-md px-2 py-1 text-xs " +
        (active ? "bg-primary/20 text-foreground" : "text-muted-foreground hover:bg-accent hover:text-foreground")
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
      <div className="flex min-w-0 flex-1 flex-col">
        {renaming ? (
          <input
            ref={renameRef}
            className="w-full rounded border-none bg-background px-0 py-0 text-xs outline-none ring-1 ring-primary"
            value={renameValue}
            onChange={(e) => setRenameValue(e.target.value)}
            onBlur={() => void commitRename()}
            onKeyDown={(e) => {
              if (e.key === "Enter") void commitRename();
              if (e.key === "Escape") setRenaming(false);
              e.stopPropagation();
            }}
            onClick={(e) => e.stopPropagation()}
          />
        ) : (
          <>
            <span className={"truncate leading-snug " + (chatTab?.sessionTitle ? "font-medium text-foreground" : "")}>
              {label}
            </span>
            {excerpt && !chatTab?.sessionTitle && (
              <span className="truncate text-[10px] italic text-muted-foreground/70">{excerpt}</span>
            )}
          </>
        )}
      </div>
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
        className="flex h-4 w-4 flex-none items-center justify-center rounded text-muted-foreground opacity-60 hover:bg-accent hover:text-foreground hover:opacity-100"
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
        const snap = await shells.browser.shell.tabs_list();
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
  useBridgeEvent("shell.tab_activated", (p) => dispatch({ type: "setActiveTab", id: p.tabId }));
  useBridgeEvent("space.switch_loading", ({ loading }) => setSwitching(loading));

  // supervisor-session-ux: update sessionTitle when a session is renamed.
  // Manual renames (manually_named: true) always win.
  // Auto-names (manually_named: false) only apply when no manually-set title exists.
  useBridgeEvent("session.renamed" as never, (p: { session_id: string; name: string; manually_named: boolean }) => {
    dispatch({
      type: "setTabs",
      tabs: tabs.map((t) => {
        if (t.kind === "chat" && t.id === p.session_id) {
          // For auto-naming: don't overwrite an existing title (could be a prior manual rename
          // that arrived before the tab metadata was refreshed).
          const nextTitle = p.manually_named ? p.name : (t.sessionTitle ?? p.name);
          return { ...t, sessionTitle: nextTitle };
        }
        return t;
      }),
      activeId: activeTabId,
    });
  });

  // ── Actions ────────────────────────────────────────────────────────
  const activate = useCallback(async (tab: TabSummary) => {
    try {
      await shells.browser.shell.tab_switch({ id: tab.id });
    } catch (e) {
      console.warn("shell.tab_switch failed", e);
    }
  }, []);

  const close = useCallback(async (tab: TabSummary) => {
    try {
      await shells.browser.shell.tab_close({ id: tab.id });
    } catch (e) {
      console.warn("shell.tab_close failed", e);
    }
  }, []);

  return (
    // Background driven by HTML rather than the native chrome: the native
    // `SidebarView::ApplyTheme` path was not propagating the runtime theme
    // change, so the sidebar appeared frozen in its initial mode even though
    // text colors (which follow `--foreground` via `data-theme` on <html>)
    // were updating correctly. `bg-cronymax-body` reads `--color-cronymax-body`,
    // which is defined per-theme in theme.css and matches the native chrome's
    // `chrome.bg_body` so the seam between the sidebar and the title bar / body
    // panel stays seamless either way.
    <aside
      ref={dragRef as React.RefObject<HTMLElement>}
      className="app-drag flex h-full flex-col bg-cronymax-body pt-7 text-foreground"
    >
      {/* Items section */}
      <section className="no-drag flex-1 overflow-auto px-2 pb-4 pt-2">
        {switching && (
          <div className="mb-2 rounded bg-card px-2 py-1 text-xs text-muted-foreground">Restarting runtime…</div>
        )}
        <div className="no-drag px-2 pb-1 text-xs font-semibold uppercase tracking-wider text-muted-foreground">
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
