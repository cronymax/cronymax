import { useCallback, useState } from "react";
import { Icon, type IconName } from "@/components/Icon";
import { shells } from "@/shells/bridge";
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
 * declared target).
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

function RailButton({ label, onClick, children }: { label: string; onClick: () => void; children: React.ReactNode }) {
  return (
    <button
      type="button"
      title={label}
      aria-label={label}
      onClick={onClick}
      className="no-drag flex h-10 w-10 flex-none items-center justify-center rounded-md text-muted-foreground transition-colors hover:bg-accent hover:text-foreground"
    >
      {children}
    </button>
  );
}

/** One extension-contributed view icon. Falls back to a generic glyph when
 *  the extension declared no icon or the image fails to load. */
function ExtensionViewButton({ view, onOpen }: { view: ExtensionView; onOpen: (v: ExtensionView) => void }) {
  const iconUrl = buildIconUrl(view);
  const [imgFailed, setImgFailed] = useState(false);
  const showImg = iconUrl !== null && !imgFailed;
  return (
    <RailButton label={view.title} onClick={() => onOpen(view)}>
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

  return (
    <nav className="app-drag flex h-full w-full flex-col items-center gap-1 bg-cronymax-body pt-7 pb-2 text-foreground">
      {BUILTIN_VIEWS.map((v) => (
        <RailButton key={v.id} label={v.label} onClick={() => void openSingleton(v.id)}>
          <Icon name={v.icon} size={20} aria-hidden="true" />
        </RailButton>
      ))}
      {views.length > 0 && <div className="my-1 h-px w-6 flex-none bg-border" />}
      {views.map((v) => (
        <ExtensionViewButton key={viewKey(v)} view={v} onOpen={() => void openView(v)} />
      ))}
    </nav>
  );
}
