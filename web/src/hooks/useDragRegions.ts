import { useRef } from "react";

/**
 * arc-style-tab-cards Phase 12 stub.
 *
 * Native drag strips are now installed by C++ via mac_view_style.mm; the
 * old JS-side rect-pump (CSS -webkit-app-region polyfill) has been
 * removed along with the `shell.set_drag_regions` channel. This hook is
 * kept as a no-op shim so existing call sites compile until the sidebar
 * rewrite (Phase 10) drops the import entirely.
 */
export function useDragRegions(_panel: "sidebar" | "topbar") {
  return useRef<HTMLElement | null>(null);
}
