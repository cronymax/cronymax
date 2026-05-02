/**
 * theme_sampler.ts — arc-style-tab-cards Phase 11.
 *
 * Observes a renderer page's effective chrome color and pushes it to the
 * native tab toolbar via `tab.set_chrome_theme`. Precedence (highest first):
 *   1. <meta name="theme-color"> content
 *   2. computed `background-color` of <body>
 * Falls back to clearing the override (empty string) if neither is present
 * or both resolve to fully transparent.
 *
 * Throttled to ≤4 fps via requestAnimationFrame; only emits on change.
 */
import { bridge } from "@/bridge";

const TAB_ID_QS = "tabId";

function readMetaThemeColor(): string | null {
  const m = document.head.querySelector<HTMLMetaElement>(
    'meta[name="theme-color"]',
  );
  const v = m?.content?.trim();
  return v && v.length > 0 ? v : null;
}

function readBodyBg(): string | null {
  if (!document.body) return null;
  const v = getComputedStyle(document.body).backgroundColor;
  if (!v || v === "rgba(0, 0, 0, 0)" || v === "transparent") return null;
  return v;
}

function effectiveColor(): string {
  return readMetaThemeColor() ?? readBodyBg() ?? "";
}

export function startThemeSampler(): void {
  // Per-tab id: the C++ side sets `?tabId=tab-N` on the content-browser URL.
  const tabId = new URLSearchParams(location.search).get(TAB_ID_QS);
  if (!tabId) return;

  let last = "";
  let rafScheduled = false;

  const publish = () => {
    rafScheduled = false;
    const color = effectiveColor();
    if (color === last) return;
    last = color;
    void bridge.send("tab.set_chrome_theme", { tabId, color });
  };
  const schedule = () => {
    if (rafScheduled) return;
    rafScheduled = true;
    requestAnimationFrame(publish);
  };

  // Emit once on startup and again after first paint.
  schedule();
  window.addEventListener("load", schedule, { once: true });

  // Watch <meta name="theme-color"> mutations and body style changes.
  const mo = new MutationObserver(schedule);
  mo.observe(document.head, {
    childList: true,
    subtree: true,
    attributes: true,
    attributeFilter: ["content", "name"],
  });
  if (document.body) {
    mo.observe(document.body, {
      attributes: true,
      attributeFilter: ["style", "class"],
    });
  }
}
