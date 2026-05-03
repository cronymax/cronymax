import { bridge } from "@/bridge";
import type { ThemeMode, ThemeResolved } from "@/types";

/**
 * `installThemeMirror` — non-React side-effect installed at the top of
 * every panel's main.tsx. Fetches the current theme from the host and
 * subscribes to `theme.changed` so the `<html data-theme="…">`
 * attribute always reflects the resolved appearance.
 *
 * Light/Dark explicit modes set the attribute. System mode clears it
 * so the `prefers-color-scheme` media query in `theme.css` decides.
 *
 * Idempotent: safe to call multiple times in a single document.
 */
let installed = false;

function applyAttribute(mode: ThemeMode, resolved: ThemeResolved) {
  const root = document.documentElement;
  if (mode === "system") {
    root.removeAttribute("data-theme");
  } else {
    root.setAttribute("data-theme", resolved);
  }
}

export function installThemeMirror(): void {
  if (installed) return;
  installed = true;

  bridge
    .send("theme.get")
    .then((res) => applyAttribute(res.mode, res.resolved))
    .catch(() => {
      // No-op: keep CSS defaults if the host is unavailable (storybook).
    });

  bridge.on("theme.changed", (payload) => {
    applyAttribute(payload.mode, payload.resolved);
  });
}
