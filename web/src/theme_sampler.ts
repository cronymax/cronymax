/**
 * theme_sampler.ts — arc-style-tab-cards Phase 11.
 *
 * Observes a renderer page's effective content accent and pushes it to the
 * native tab toolbar via `tab.set_chrome_theme`. Precedence (highest first):
 *   1. <meta name="theme-color"> content
 *   2. computed `background-color` of <body>
 * Falls back to the app's neutral `--color-cronymax-base` surface if neither is
 * present or the sampled color is rejected. Sampled colors are normalized to
 * #RRGGBB and bounded so they cannot erase contrast or overpower the shell.
 *
 * Throttled to ≤4 fps via requestAnimationFrame; only emits on change.
 */
import { browser } from "@/shells/bridge";

const TAB_ID_QS = "tabId";

type Rgba = { r: number; g: number; b: number; a: number };

let colorProbe: HTMLSpanElement | null = null;

function getColorProbe(): HTMLSpanElement {
  if (colorProbe) return colorProbe;
  colorProbe = document.createElement("span");
  colorProbe.style.position = "absolute";
  colorProbe.style.width = "0";
  colorProbe.style.height = "0";
  colorProbe.style.pointerEvents = "none";
  colorProbe.style.opacity = "0";
  colorProbe.style.color = "transparent";
  document.documentElement.appendChild(colorProbe);
  return colorProbe;
}

function parseComputedColor(input: string): Rgba | null {
  const probe = getColorProbe();
  probe.style.color = "";
  probe.style.color = input;
  if (!probe.style.color) return null;
  const computed = getComputedStyle(probe).color;
  const match = computed.match(/^rgba?\((\d+),\s*(\d+),\s*(\d+)(?:,\s*([\d.]+))?\)$/i);
  if (!match) return null;
  return {
    r: Number(match[1]),
    g: Number(match[2]),
    b: Number(match[3]),
    a: match[4] == null ? 1 : Number(match[4]),
  };
}

function toHex(color: Rgba): string {
  const toPart = (value: number) =>
    Math.max(0, Math.min(255, Math.round(value)))
      .toString(16)
      .padStart(2, "0");
  return `#${toPart(color.r)}${toPart(color.g)}${toPart(color.b)}`;
}

function srgbToLinear(value: number): number {
  const channel = value / 255;
  return channel <= 0.04045 ? channel / 12.92 : ((channel + 0.055) / 1.055) ** 2.4;
}

function luminance(color: Rgba): number {
  return 0.2126 * srgbToLinear(color.r) + 0.7152 * srgbToLinear(color.g) + 0.0722 * srgbToLinear(color.b);
}

function contrastRatio(a: Rgba, b: Rgba): number {
  const l1 = luminance(a);
  const l2 = luminance(b);
  const [hi, lo] = l1 >= l2 ? [l1, l2] : [l2, l1];
  return (hi + 0.05) / (lo + 0.05);
}

function saturation(color: Rgba): number {
  const max = Math.max(color.r, color.g, color.b) / 255;
  const min = Math.min(color.r, color.g, color.b) / 255;
  if (max === min) return 0;
  const lightness = (max + min) / 2;
  const delta = max - min;
  return lightness > 0.5 ? delta / (2 - max - min) : delta / (max + min);
}

function mix(a: Rgba, b: Rgba, weight: number): Rgba {
  const inv = 1 - weight;
  return {
    r: a.r * inv + b.r * weight,
    g: a.g * inv + b.g * weight,
    b: a.b * inv + b.b * weight,
    a: 1,
  };
}

function neutralSurface(): Rgba | null {
  const css = getComputedStyle(document.documentElement).getPropertyValue("--color-cronymax-base").trim();
  return css ? parseComputedColor(css) : null;
}

function readableText(): Rgba | null {
  const css = getComputedStyle(document.documentElement).getPropertyValue("--color-cronymax-title").trim();
  return css ? parseComputedColor(css) : null;
}

function normalizeSample(input: string | null): string | null {
  if (!input) return null;
  const sample = parseComputedColor(input);
  if (!sample || sample.a <= 0) return null;

  const neutral = neutralSurface();
  const text = readableText();
  let adjusted = { ...sample, a: 1 };

  if (neutral) {
    const lum = luminance(adjusted);
    const sat = saturation(adjusted);
    if (lum < 0.08 || lum > 0.92 || sat > 0.9) {
      adjusted = mix(adjusted, neutral, 0.45);
    }
  }

  if (neutral && text) {
    const contrastToText = contrastRatio(adjusted, text);
    const contrastToShell = contrastRatio(adjusted, neutral);
    if (contrastToText < 3.5 || contrastToShell < 1.12) {
      adjusted = mix(adjusted, neutral, 0.6);
    }
    if (contrastRatio(adjusted, text) < 3) {
      adjusted = neutral;
    }
  }

  return toHex(adjusted);
}

function readMetaThemeColor(): string | null {
  const m = document.head.querySelector<HTMLMetaElement>('meta[name="theme-color"]');
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
  return (
    normalizeSample(readMetaThemeColor()) ??
    normalizeSample(readBodyBg()) ??
    normalizeSample(getComputedStyle(document.documentElement).getPropertyValue("--color-cronymax-base").trim()) ??
    ""
  );
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
    void browser.send("tab.set_chrome_theme", { tabId, color });
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
