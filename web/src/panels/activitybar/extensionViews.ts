// Operation-view registry for the activity-bar rail.
//
// Loads `cronymax.ui.sidebar.view` contributions from the runtime once per
// session (refetched on `runtime.reconnected`) and exposes them as rail
// icons. Clicking one opens the extension's webview view in the main content
// area (target "main") or the right-side dock (target "right").
//
// Live activate/deactivate updates are refetched on reconnect; finer-grained
// live updates are P10 hardening (mirrors extensionRenderers.ts).

import { useEffect, useRef, useState } from "react";
import { browser } from "@/shells/bridge";
import { type ContributionDescriptor, ContributionKind, contributionRegistry } from "@/shells/runtime";
import type { ViewTarget } from "@/types";

export interface ExtensionView {
  /** Owning extension id (`publisher.name`). */
  extId: string;
  /** View id, locally unique within the extension. */
  viewId: string;
  /** Display title (tooltip). */
  title: string;
  /** Icon path relative to the extension dir (served via cronymax-webview). */
  icon?: string;
  /** Entry HTML path declared in the manifest. */
  entry: string;
  /** Where the view opens: replace main area, or the right dock. */
  target: ViewTarget;
}

/** Metadata payload the Rust side stamps onto sidebar-view descriptors.
 *  Mirrors `SidebarViewContribution` from the manifest IDL. */
interface ViewMetadata {
  id?: string;
  title?: string;
  icon?: string;
  entry?: string;
  target?: ViewTarget;
}

function descriptorToView(desc: ContributionDescriptor): ExtensionView | null {
  if (desc.owner.type !== "extension") return null;
  const md = (desc.metadata as ViewMetadata | undefined) ?? {};
  const entry = typeof md.entry === "string" ? md.entry : "";
  if (!entry) return null;
  const target: ViewTarget = md.target === "right" ? "right" : "main";
  const icon = desc.icon ?? (typeof md.icon === "string" ? md.icon : undefined);
  return {
    extId: desc.owner.extId,
    viewId: desc.id,
    title: desc.label,
    icon,
    entry,
    target,
  };
}

/** Stable dedup key for an open view tab — must match the native side. */
export function viewKey(view: ExtensionView): string {
  return `${view.extId}::${view.viewId}`;
}

/** Build the `cronymax-webview://<ext>/<entry>?surface=panel&id=<viewId>` URL
 *  the platform loads into the view tab / dock. Mirrors the Rust
 *  `WebviewRegistry::url_for_surface` "panel" surface. */
export function buildViewUrl(view: ExtensionView): string {
  const cleanEntry = view.entry.replace(/^\.\//, "").replace(/^\/+/, "");
  const encId = encodeURIComponent(view.viewId);
  return `cronymax-webview://${view.extId}/${cleanEntry}?surface=panel&id=${encId}`;
}

/** Build the icon image URL, or null when the view declared no icon. */
export function buildIconUrl(view: ExtensionView): string | null {
  if (!view.icon) return null;
  const cleanIcon = view.icon.replace(/^\.\//, "").replace(/^\/+/, "");
  return `cronymax-webview://${view.extId}/${cleanIcon}`;
}

/**
 * Hook exposing the current operation-view registry. Empty while the initial
 * fetch is in flight or the runtime is unavailable; refetches on
 * `runtime.reconnected`.
 */
export function useExtensionViewRegistry(): ExtensionView[] {
  const [views, setViews] = useState<ExtensionView[]>([]);
  const initialFetched = useRef(false);

  useEffect(() => {
    let cancelled = false;
    const refetch = async () => {
      try {
        const { contributions } = await contributionRegistry.list();
        if (cancelled) return;
        const next = contributions
          .filter((d) => d.kind === ContributionKind.SidebarView)
          .map(descriptorToView)
          .filter((v): v is ExtensionView => v !== null);
        setViews(next);
      } catch {
        // First load can race runtime startup; reconnect listener retries.
      }
    };

    if (!initialFetched.current) {
      initialFetched.current = true;
      void refetch();
    }
    const off = browser.on("runtime.reconnected", () => {
      void refetch();
    });

    return () => {
      cancelled = true;
      off();
    };
  }, []);

  return views;
}
