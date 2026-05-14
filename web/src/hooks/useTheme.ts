import { useEffect, useState } from "react";
import { shells } from "@/shells/bridge";
import type { ThemeMode, ThemeResolved } from "@/types";
import { useBridgeEvent } from "./useBridgeEvent";

/**
 * `useTheme` — single source of truth for the renderer-side theme state.
 *
 * Behaviour (refine-cronymax-theme-layout):
 *  - On mount, calls `theme.get` to fetch the persisted mode and the
 *    resolved appearance (system follow has already been computed by
 *    the host).
 *  - Subscribes to `theme.changed` so every panel updates in lock-step
 *    when the user picks Light/Dark/System or the OS appearance flips.
 *  - Mirrors the resolved appearance into `<html data-theme="…">` so
 *    the CSS overrides in `web/src/styles/theme.css` swap palettes.
 *    System mode intentionally clears the attribute so the
 *    prefers-color-scheme media query takes over.
 *  - `setMode` is a thin wrapper around `theme.set`; the host echoes
 *    a `theme.changed` event which drives the local state update.
 */
export interface UseThemeResult {
  mode: ThemeMode;
  resolved: ThemeResolved;
  setMode: (mode: ThemeMode) => void;
}

function applyAttribute(mode: ThemeMode, resolved: ThemeResolved) {
  const root = document.documentElement;
  if (mode === "system") {
    root.removeAttribute("data-theme");
  } else {
    root.setAttribute("data-theme", resolved);
  }
}

export function useTheme(): UseThemeResult {
  const [mode, setModeState] = useState<ThemeMode>("system");
  const [resolved, setResolved] = useState<ThemeResolved>("dark");

  useEffect(() => {
    let cancelled = false;
    shells.browser.theme
      .get()
      .then((res) => {
        if (cancelled) return;
        setModeState(res.mode);
        setResolved(res.resolved);
        applyAttribute(res.mode, res.resolved);
      })
      .catch(() => {
        // Best-effort; keep defaults if the host is unavailable.
      });
    return () => {
      cancelled = true;
    };
  }, []);

  useBridgeEvent("theme.changed", (payload) => {
    setModeState(payload.mode);
    setResolved(payload.resolved);
    applyAttribute(payload.mode, payload.resolved);
  });

  function setMode(next: ThemeMode) {
    shells.browser.theme.set({ mode: next }).catch(() => {
      // Ignore — the broadcast is the authoritative update.
    });
  }

  return { mode, resolved, setMode };
}
