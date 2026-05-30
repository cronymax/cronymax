import { useCallback, useState } from "react";
import { Icon, type IconName } from "@/components/Icon";
import { useBridgeEvent } from "@/hooks/useBridgeEvent";
import { runtimeSend, shells } from "@/shells/bridge";
import type { SingletonViewKind } from "@/types";
import { buildIconUrl, buildViewUrl, type ExtensionView, useExtensionViewRegistry, viewKey } from "./extensionViews";

/**
 * Activity bar — the leftmost vertical icon rail.
 *
 * Built-in operation views (Activities, Flows) sit at the top as icon
 * buttons; below a divider, one icon per extension that contributes a
 * `cronymax.ui.sidebar.view`. Built-ins open their native singleton tab via
 * `shell.tab_open_singleton`; extension views open via
 * `shell.open_extension_view` (main area or right dock per the view's
 * declared target). Right-click an extension icon to disable the extension.
 *
 * The native side pushes `shell.active_view_changed {main, dock}` whenever
 * the active main view or the dock changes; the rail highlights the matching
 * icon(s). It refetches its view list on `extensions/contributions`.
 */

interface BuiltinView {
  id: SingletonViewKind;
  label: string;
  icon: IconName;
}

const BUILTIN_VIEWS: BuiltinView[] = [
  { id: "activity", label: "Activities", icon: "pulse" },
  { id: "flows", label: "Flows", icon: "layers" },
];

interface ActiveViews {
  main: string;
  dock: string;
}

interface ContextMenu {
  view: ExtensionView;
  x: number;
  y: number;
}

function RailButton({
  label,
  active,
  onClick,
  onContextMenu,
  children,
}: {
  label: string;
  active: boolean;
  onClick: () => void;
  onContextMenu?: (e: React.MouseEvent) => void;
  children: React.ReactNode;
}) {
  return (
    <div className="relative flex w-full flex-none justify-center" onContextMenu={onContextMenu}>
      {active && <span className="absolute left-0 top-1.5 bottom-1.5 w-0.5 rounded-r bg-primary" aria-hidden="true" />}
      <button
        type="button"
        title={label}
        aria-label={label}
        aria-current={active ? "true" : undefined}
        onClick={onClick}
        className={`no-drag flex h-10 w-10 items-center justify-center rounded-md transition-colors ${
          active
            ? "bg-cronymax-selected text-foreground"
            : "text-muted-foreground hover:bg-accent hover:text-foreground"
        }`}
      >
        {children}
      </button>
    </div>
  );
}

/** One extension-contributed view icon. Falls back to a generic glyph when
 *  the extension declared no icon or the image fails to load. */
function ExtensionViewButton({
  view,
  active,
  onOpen,
  onContextMenu,
}: {
  view: ExtensionView;
  active: boolean;
  onOpen: (v: ExtensionView) => void;
  onContextMenu: (v: ExtensionView, e: React.MouseEvent) => void;
}) {
  const iconUrl = buildIconUrl(view);
  const [imgFailed, setImgFailed] = useState(false);
  const showImg = iconUrl !== null && !imgFailed;
  return (
    <RailButton
      label={view.title}
      active={active}
      onClick={() => onOpen(view)}
      onContextMenu={(e) => onContextMenu(view, e)}
    >
      {showImg ? (
        <img
          src={iconUrl}
          width={20}
          height={20}
          alt=""
          className="h-5 w-5 object-contain"
          onError={() => setImgFailed(true)}
        />
      ) : (
        <Icon name="tools" size={20} aria-hidden="true" />
      )}
    </RailButton>
  );
}

export function App() {
  const views = useExtensionViewRegistry();
  const [active, setActive] = useState<ActiveViews>({ main: "", dock: "" });
  const [menu, setMenu] = useState<ContextMenu | null>(null);

  useBridgeEvent("shell.active_view_changed" as never, (p: ActiveViews) => {
    setActive({ main: p?.main ?? "", dock: p?.dock ?? "" });
  });

  const openSingleton = useCallback(async (kind: SingletonViewKind) => {
    try {
      await shells.browser.shell.tab_open_singleton({ kind });
    } catch (e) {
      console.warn("tab_open_singleton failed", e);
    }
  }, []);

  const openView = useCallback(async (view: ExtensionView) => {
    try {
      await shells.browser.shell.open_extension_view({
        url: buildViewUrl(view),
        view_key: viewKey(view),
        title: view.title,
        target: view.target,
      });
    } catch (e) {
      console.warn("open_extension_view failed", e);
    }
  }, []);

  const disableExtension = useCallback(async (extId: string) => {
    setMenu(null);
    try {
      // Close the extension's open view tab/dock + switch to chat (native),
      // then deactivate the extension in the runtime (drops its rail icon
      // via the extensions/contributions signal).
      await shells.browser.shell.close_extension_views({ ext_id: extId });
      await runtimeSend("extension.deactivate", { ext_id: extId });
    } catch (e) {
      console.warn("disable extension failed", e);
    }
  }, []);

  return (
    // Rail sits a shade darker than the sidebar/body in both themes so the
    // two columns read as distinct surfaces. Derived from the body token so
    // it tracks the theme (light → darker gray, dark → near-black); the
    // native ActivityBarView column uses the matching ×0.82 darken.
    <nav
      className="app-drag relative flex h-full w-full flex-col items-center gap-1 pt-7 pb-2 text-foreground"
      style={{ background: "color-mix(in srgb, var(--color-cronymax-body), #000 18%)" }}
    >
      {BUILTIN_VIEWS.map((v) => (
        <RailButton key={v.id} label={v.label} active={active.main === v.id} onClick={() => void openSingleton(v.id)}>
          <Icon name={v.icon} size={20} aria-hidden="true" />
        </RailButton>
      ))}
      {views.length > 0 && <div className="my-1 h-px w-6 flex-none bg-border" />}
      {views.map((v) => {
        const key = viewKey(v);
        return (
          <ExtensionViewButton
            key={key}
            view={v}
            active={active.main === key || active.dock === key}
            onOpen={() => void openView(v)}
            onContextMenu={(view, e) => {
              e.preventDefault();
              setMenu({ view, x: e.clientX, y: e.clientY });
            }}
          />
        );
      })}

      {menu && (
        <>
          {/* Click-away backdrop. */}
          <button
            type="button"
            aria-label="Dismiss menu"
            className="no-drag fixed inset-0 z-40 cursor-default"
            onClick={() => setMenu(null)}
            onContextMenu={(e) => {
              e.preventDefault();
              setMenu(null);
            }}
          />
          <div
            className="no-drag fixed z-50 min-w-36 rounded-md border border-border bg-cronymax-float py-1 text-xs shadow-lg"
            style={{ left: menu.x, top: menu.y }}
          >
            <button
              type="button"
              className="block w-full px-3 py-1.5 text-left text-foreground hover:bg-accent"
              onClick={() => void disableExtension(menu.view.extId)}
            >
              Disable extension
            </button>
            <div className="px-3 pt-0.5 pb-1 text-[10px] text-muted-foreground">{menu.view.extId}</div>
          </div>
        </>
      )}
    </nav>
  );
}
