import { useCallback, useEffect, useRef, useState } from "react";
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
  /** view_keys of every extension view whose iframe is currently alive (open
   *  main tabs + the dock's loaded view). The native side diffs against this
   *  to fire dispose; absent on older hosts. */
  open?: string[];
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
  const [maskUrl, setMaskUrl] = useState<string | null>(null);
  const [imgFailed, setImgFailed] = useState(false);

  // VS Code recolors monochrome icons with `-webkit-mask` + a theme color, but
  // its icons are same-origin. Ours are cross-origin (cronymax-webview://), and
  // a CSS mask / fetch can't read that scheme from the file:// rail. So we load
  // the icon as a CORS-clean <img> (the scheme returns Access-Control-Allow-
  // Origin: *), rasterize it onto a canvas, and export a same-origin data: URL
  // — which CAN be used as a mask. Filling that mask with `currentColor` gives
  // the exact built-in treatment (text-muted-foreground inactive /
  // text-foreground active) and follows theme flips. The icon's own colors are
  // irrelevant: only its alpha (shape) is used as the mask.
  useEffect(() => {
    setMaskUrl(null);
    setImgFailed(false);
    if (iconUrl === null) return;
    let alive = true;
    const img = new Image();
    img.crossOrigin = "anonymous";
    img.onload = () => {
      if (!alive) return;
      try {
        const px = 40; // 2x of the 20px slot for crisp edges
        const canvas = document.createElement("canvas");
        canvas.width = px;
        canvas.height = px;
        const ctx = canvas.getContext("2d");
        if (!ctx) {
          setImgFailed(true);
          return;
        }
        ctx.drawImage(img, 0, 0, px, px);
        setMaskUrl(canvas.toDataURL());
      } catch {
        setImgFailed(true); // tainted canvas → fall back
      }
    };
    img.onerror = () => {
      if (alive) setImgFailed(true);
    };
    img.src = iconUrl;
    return () => {
      alive = false;
    };
  }, [iconUrl]);

  const showImgFallback = iconUrl !== null && !imgFailed;
  return (
    <RailButton
      label={view.title}
      active={active}
      onClick={() => onOpen(view)}
      onContextMenu={(e) => onContextMenu(view, e)}
    >
      {maskUrl ? (
        // mask (same-origin data: URL) filled with currentColor → exact match
        // with the built-in icons, incl. active/inactive shades + theme flips.
        <span
          aria-hidden="true"
          className="h-5 w-5"
          style={{
            backgroundColor: "currentColor",
            WebkitMaskImage: `url("${maskUrl}")`,
            maskImage: `url("${maskUrl}")`,
            WebkitMaskRepeat: "no-repeat",
            maskRepeat: "no-repeat",
            WebkitMaskPosition: "center",
            maskPosition: "center",
            WebkitMaskSize: "contain",
            maskSize: "contain",
          }}
        />
      ) : showImgFallback ? (
        // Fallback (canvas tainted / load error): render directly, forced to a
        // theme-colored silhouette so the icon stays visible.
        <img
          src={iconUrl}
          width={20}
          height={20}
          alt=""
          className="h-5 w-5 object-contain brightness-0 dark:invert"
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

  // Last visibility we told each extension view's host about (viewId →
  // visible). Lets us fire `onDidChangeVisibility` only on real transitions
  // and keeps the redundant `visible: true` that follows a resolve from
  // double-firing (the bootstrap drops a no-change `_setVisible` anyway).
  const viewVisibleRef = useRef<Map<string, boolean>>(new Map());
  // The set of view_keys whose iframe was alive at the previous event, so we
  // can detect a teardown (a key that left the set) and fire dispose.
  const openKeysRef = useRef<Set<string>>(new Set());
  // Same set, as state, so the rail can show "Close view" only for views that
  // are actually open right now.
  const [openKeys, setOpenKeys] = useState<Set<string>>(new Set());

  useBridgeEvent("shell.active_view_changed" as never, (p: ActiveViews) => {
    const next: ActiveViews = { main: p?.main ?? "", dock: p?.dock ?? "" };
    setActive(next);

    const prev = viewVisibleRef.current;

    // Dispose: a view_key that was alive last time but is gone now had its
    // iframe torn down (main tab closed, or dock navigated to another view) —
    // fire `onDidDispose`. A collapse / switch-away keeps the key alive, so it
    // stays a hide, handled below. `open` is absent on older hosts → treat as
    // "unknown", skip the diff so we never dispose spuriously.
    const disposed = new Set<string>();
    if (p?.open) {
      const openNow = new Set(p.open);
      for (const key of openKeysRef.current) {
        if (openNow.has(key)) continue;
        disposed.add(key);
        const v = views.find((vv) => viewKey(vv) === key);
        if (!v) continue;
        prev.delete(v.viewId);
        void runtimeSend("extension.view.dispose", { view_id: v.viewId }).catch((e) =>
          console.warn("extension.view.dispose failed", e),
        );
      }
      openKeysRef.current = openNow;
      setOpenKeys(openNow);
    }

    // Visibility: a view is visible iff it's the active tab on its surface
    // (main area or right dock). Switching tabs keeps the surface alive but
    // hidden, so we flip `WebviewView.visible` rather than dispose — mirrors
    // VS Code collapsing a view section.
    const seen = new Set<string>();
    for (const v of views) {
      seen.add(v.viewId);
      const key = viewKey(v);
      if (disposed.has(key)) continue; // just torn down — no trailing hide
      const nowVisible = v.target === "right" ? next.dock === key : next.main === key;
      if ((prev.get(v.viewId) ?? false) === nowVisible) continue;
      prev.set(v.viewId, nowVisible);
      void runtimeSend("extension.view.visibility", { view_id: v.viewId, visible: nowVisible }).catch((e) =>
        console.warn("extension.view.visibility failed", e),
      );
    }
    // Forget views no longer contributed (extension deactivated) so a later
    // reinstall starts fresh.
    for (const id of [...prev.keys()]) {
      if (!seen.has(id)) prev.delete(id);
    }
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
      // The view's iframe is now mounted (it self-registers its frame by
      // viewId in the renderer). Ask the owning extension to resolve the
      // view so its WebviewViewProvider can post into it. No-op on the Rust
      // side for views without a registered provider (declarative-only).
      // The viewId is the contributed id (`buildViewUrl` puts it in `?id=`).
      void runtimeSend("extension.view.resolve", { view_id: view.viewId }).catch((e) =>
        console.warn("extension.view.resolve failed", e),
      );
    } catch (e) {
      console.warn("open_extension_view failed", e);
    }
  }, []);

  const closeView = useCallback(async (view: ExtensionView) => {
    setMenu(null);
    try {
      // Native tears down the view's iframe (closes its main tab / drops it
      // from the dock) without deactivating the extension. That removes it
      // from the next `active_view_changed` `open` set, so the handler above
      // fires `extension.view.dispose` → the provider's onDidDispose.
      await shells.browser.shell.close_extension_view({ view_key: viewKey(view) });
    } catch (e) {
      console.warn("close_extension_view failed", e);
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
            {openKeys.has(viewKey(menu.view)) && (
              <button
                type="button"
                className="block w-full px-3 py-1.5 text-left text-foreground hover:bg-accent"
                onClick={() => void closeView(menu.view)}
              >
                Close view
              </button>
            )}
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
