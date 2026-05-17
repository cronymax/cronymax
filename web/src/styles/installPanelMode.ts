/**
 * installPanelMode — synchronous side-effect installed at the top of every
 * panel main.tsx that may be hosted in a standalone PanelWindow (settings,
 * flows, activity).
 *
 * The native `PanelWindow` (app/browser/views/panel_window.cc) appends a
 * `#panel` hash to the page URL when it loads the panel into its own
 * top-level CefWindow. The window has `NSWindowStyleMaskFullSizeContentView`
 * so the page content extends to the very top of the window; the OS traffic
 * lights overlay the upper-left ~80 px.
 *
 * Setting `data-panel-window` on `<html>` lets CSS (see theme.css) add the
 * necessary top-left clearance to panel headers ONLY when running inside
 * such a window — when the same panel HTML is loaded into a regular tab,
 * the hash is absent and the header keeps its flush-left layout.
 *
 * Idempotent.
 */
export function installPanelMode(): void {
  if (typeof window === "undefined") return;
  if (window.location.hash !== "#panel") return;
  document.documentElement.setAttribute("data-panel-window", "");
}
