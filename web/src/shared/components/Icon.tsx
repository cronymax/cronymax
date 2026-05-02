import type { SVGProps } from "react";
import type { IconName } from "@/shared/icons";
import { codiconComponents } from "@/shared/icons";

/**
 * Renders a single Codicon glyph as a React SVG component, transformed at
 * build time by vite-plugin-svgr (SVGO). The icon inherits `currentColor`
 * so it responds to CSS colour tokens.
 *
 * Usage:
 *   ```tsx
 *   <Icon name="refresh" aria-label="Reload" />
 *   <Icon name="terminal" width={20} height={20} className="opacity-80" />
 *   ```
 *
 * Accessibility: pass `aria-label` for icon-only affordances; for
 * decorative icons next to a visible text label, pass `aria-hidden="true"`
 * (or omit the label and the consumer's own label will speak).
 */
export function Icon({
  name,
  size = 16,
  ...rest
}: { name: IconName; size?: number } & SVGProps<SVGSVGElement>) {
  const SvgIcon = codiconComponents[name];
  return (
    <SvgIcon
      width={size}
      height={size}
      fill="currentColor"
      role="img"
      {...rest}
    />
  );
}
