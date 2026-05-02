import { useEffect, useCallback } from "react";
import { bridge } from "@/bridge";
import { useBridgeEvent } from "@/hooks/useBridgeEvent";
import { useDragRegions } from "@/hooks/useDragRegions";
import { useStore, saveChats } from "./store";

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

interface RowItem {
  kind: "tab" | "terminal" | "chat";
  id: string;
  numericId?: number;
  label: string;
  iconUrl?: string | null;
  iconText?: string;
}

function Row({
  item,
  active,
  onActivate,
  onClose,
}: {
  item: RowItem;
  active: boolean;
  onActivate: () => void;
  onClose: () => void;
}) {
  return (
    <li
      onClick={onActivate}
      className={
        "group flex h-7 cursor-pointer items-center gap-2 rounded-md px-2 text-xs " +
        (active
          ? "bg-cronymax-surface-2 text-cronymax-fg"
          : "text-cronymax-fg-muted hover:bg-cronymax-surface-2 hover:text-cronymax-fg")
      }
    >
      <span className="flex h-3.5 w-3.5 flex-none items-center justify-center text-[11px]">
        {item.iconUrl ? (
          <img
            src={item.iconUrl}
            width={14}
            height={14}
            className="rounded-sm"
            onError={(e) => {
              (e.target as HTMLImageElement).style.display = "none";
            }}
          />
        ) : (
          item.iconText || "○"
        )}
      </span>
      <span className="flex-1 truncate">{item.label}</span>
      <button
        type="button"
        title="Close"
        onClick={(e) => {
          e.stopPropagation();
          onClose();
        }}
        className="hidden h-4 w-4 flex-none items-center justify-center rounded text-cronymax-fg-muted hover:bg-cronymax-border hover:text-white group-hover:flex"
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
    terminals,
    activeTerminalId,
    chats,
    activeChatId,
    spaces,
    activeSpaceId,
    activeSpaceName,
    panel,
    spacesOpen,
  } = state;

  // ── Initial load ───────────────────────────────────────────────────
  useEffect(() => {
    void (async () => {
      try {
        const tabsResp = await bridge.send("shell.tabs_list");
        dispatch({
          type: "setTabs",
          tabs: tabsResp.tabs ?? [],
          activeId: tabsResp.active_tab_id ?? null,
        });
      } catch {
        // ignore
      }
      try {
        const termResp = await bridge.send("terminal.list");
        dispatch({
          type: "setTerminals",
          items: termResp.items ?? [],
          active: termResp.active ?? termResp.items?.[0]?.id ?? null,
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
  }, []);

  // ── Push events ────────────────────────────────────────────────────
  useBridgeEvent("shell.tab_created", (p) =>
    dispatch({ type: "addTab", tab: p }),
  );
  useBridgeEvent("shell.tab_closed", (p) =>
    dispatch({ type: "closeTab", id: p.id }),
  );
  useBridgeEvent("shell.tab_title_changed", (p) =>
    dispatch({ type: "updateTab", id: p.id, patch: { title: p.title } }),
  );
  useBridgeEvent("shell.tab_url_changed", (p) =>
    dispatch({ type: "updateTab", id: p.id, patch: { url: p.url } }),
  );
  useBridgeEvent("shell.active_tab_changed", (p) =>
    dispatch({ type: "setActiveTab", id: p.id }),
  );
  useBridgeEvent("shell.space_changed", (p) =>
    dispatch({ type: "setActiveSpace", id: p.id, name: p.name }),
  );
  useBridgeEvent("terminal.created", (p) =>
    dispatch({ type: "addTerminal", item: p }),
  );
  useBridgeEvent("terminal.removed", (p) =>
    dispatch({ type: "removeTerminal", id: p.id }),
  );
  useBridgeEvent("terminal.switched", (p) =>
    dispatch({ type: "setActiveTerminal", id: p.id }),
  );

  // Close spaces dropdown on outside click.
  useEffect(() => {
    if (!spacesOpen) return;
    const close = () => dispatch({ type: "toggleSpaces", open: false });
    document.addEventListener("click", close);
    return () => document.removeEventListener("click", close);
  }, [spacesOpen, dispatch]);

  // ── Actions ────────────────────────────────────────────────────────
  const activate = useCallback(
    async (item: RowItem) => {
      try {
        if (item.kind === "tab" && item.numericId != null) {
          await bridge.send("shell.tab_switch", { id: item.numericId });
          dispatch({ type: "setActiveTab", id: item.numericId });
        } else if (item.kind === "terminal") {
          await bridge.send("terminal.switch", { id: item.id });
          dispatch({ type: "setActiveTerminal", id: item.id });
          dispatch({ type: "setPanel", panel: "terminal" });
        } else if (item.kind === "chat") {
          dispatch({ type: "setActiveChat", id: item.id });
          dispatch({ type: "setPanel", panel: "chat" });
        }
      } catch (e) {
        console.warn("activate failed", e);
      }
    },
    [dispatch],
  );

  const close = useCallback(
    async (item: RowItem) => {
      try {
        if (item.kind === "tab" && item.numericId != null) {
          await bridge.send("shell.tab_close", { id: item.numericId });
          dispatch({ type: "closeTab", id: item.numericId });
        } else if (item.kind === "terminal") {
          await bridge.send("terminal.close", { id: item.id });
        } else if (item.kind === "chat") {
          dispatch({ type: "removeChat", id: item.id });
          saveChats(chats.filter((c) => c.id !== item.id));
        }
      } catch (e) {
        console.warn("close failed", e);
      }
    },
    [dispatch, chats],
  );

  // native-title-bar: + Tab / + Terminal / + Chat live on the native title
  // bar now; the sidebar bottom action row was removed. Settings now opens
  // from the native title-bar gear button as well, so no panel-switching
  // helper is needed here anymore.

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

  // ── Build rows ─────────────────────────────────────────────────────
  const pinned: RowItem[] = tabs
    .filter((t) => t.is_pinned)
    .map((t) => ({
      kind: "tab",
      id: "tab-" + t.id,
      numericId: t.id,
      label: t.title || t.url || "New Tab",
      iconUrl: faviconFor(t.url),
      iconText: "🌐",
    }));

  const items: RowItem[] = [
    ...tabs
      .filter((t) => !t.is_pinned)
      .map<RowItem>((t) => ({
        kind: "tab",
        id: "tab-" + t.id,
        numericId: t.id,
        label: t.title || t.url || "New Tab",
        iconUrl: faviconFor(t.url),
        iconText: "🌐",
      })),
    ...terminals.map<RowItem>((t) => ({
      kind: "terminal",
      id: t.id,
      label: t.name,
      iconText: "⌨",
    })),
    ...chats.map<RowItem>((c) => ({
      kind: "chat",
      id: c.id,
      label: c.name,
      iconText: "💬",
    })),
  ];

  function isActive(it: RowItem): boolean {
    if (it.kind === "tab")
      return panel === "browser" && it.numericId === activeTabId;
    if (it.kind === "terminal")
      return panel === "terminal" && it.id === activeTerminalId;
    if (it.kind === "chat") return panel === "chat" && it.id === activeChatId;
    return false;
  }

  return (
    <aside
      ref={dragRef as React.RefObject<HTMLElement>}
      className="app-drag flex h-full flex-col text-cronymax-fg pt-7"
      style={{ backgroundColor: "#14141a" }}
    >
      {/* Space header */}
      <div className="no-drag relative flex items-center gap-2 px-3 py-2.5">
        <span className="h-2.5 w-2.5 flex-none rounded-full bg-cronymax-accent" />
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
          className="flex h-5 w-5 items-center justify-center rounded text-cronymax-fg-muted hover:bg-cronymax-surface-2 hover:text-white"
        >
          ▾
        </button>
        {spacesOpen && (
          <div
            onClick={(e) => e.stopPropagation()}
            className="absolute left-3 right-3 top-full z-10 mt-1 rounded-lg border border-cronymax-border bg-cronymax-surface p-1 shadow-elev-2"
          >
            <ul>
              {spaces.map((sp) => (
                <li
                  key={sp.id}
                  onClick={() => void switchSpace(sp.id, sp.name)}
                  className={
                    "cursor-pointer rounded px-2 py-1.5 text-xs hover:bg-cronymax-surface-2 " +
                    (sp.id === activeSpaceId
                      ? "text-cronymax-fg"
                      : "text-cronymax-fg-muted")
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
              className="mt-1 w-full rounded px-2 py-1.5 text-left text-xs text-cronymax-accent-soft hover:bg-cronymax-surface-2"
            >
              + New Space
            </button>
          </div>
        )}
      </div>

      {/* Pinned section */}
      {pinned.length > 0 && (
        <section className="no-drag px-2 pb-1">
          <div className="px-2 pb-1 text-[10px] font-semibold uppercase tracking-wider text-cronymax-fg-muted">
            Pinned
          </div>
          <ul className="space-y-0.5">
            {pinned.map((it) => (
              <Row
                key={it.id}
                item={it}
                active={isActive(it)}
                onActivate={() => void activate(it)}
                onClose={() => void close(it)}
              />
            ))}
          </ul>
        </section>
      )}

      {/* Items section */}
      <section className="flex-1 overflow-auto px-2 pb-1">
        <div className="no-drag px-2 pb-1 text-[10px] font-semibold uppercase tracking-wider text-cronymax-fg-muted">
          Items
        </div>
        <ul className="no-drag space-y-0.5">
          {items.map((it) => (
            <Row
              key={`${it.kind}-${it.id}`}
              item={it}
              active={isActive(it)}
              onActivate={() => void activate(it)}
              onClose={() => void close(it)}
            />
          ))}
        </ul>
      </section>

      {/* Bottom dock: Settings now lives on the native title bar
          (see MainWindow::BuildTitleBar). The sidebar's bottom action
          row was removed entirely with the Config entry. */}
    </aside>
  );
}
