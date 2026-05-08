// Copyright (c) 2026.
//
// Cronymax icon vocabulary — string-union mirror of the C++ IconId enum
// in app/browser/icon_registry.h. Keep this list in sync with
// assets/icons/README.md (the canonical mapping table) when adding or
// removing icons.

import type { FunctionComponent, SVGProps } from "react";

import ArrowLeft from "@vscode/codicons/src/icons/arrow-left.svg?react";
import ArrowRight from "@vscode/codicons/src/icons/arrow-right.svg?react";
import Refresh from "@vscode/codicons/src/icons/refresh.svg?react";
import Close from "@vscode/codicons/src/icons/close.svg?react";
import Add from "@vscode/codicons/src/icons/add.svg?react";
import SettingsGear from "@vscode/codicons/src/icons/settings-gear.svg?react";
import Terminal from "@vscode/codicons/src/icons/terminal.svg?react";
import CommentDiscussion from "@vscode/codicons/src/icons/comment-discussion.svg?react";
import TypeHierarchy from "@vscode/codicons/src/icons/type-hierarchy.svg?react";
import Globe from "@vscode/codicons/src/icons/globe.svg?react";
import DebugStop from "@vscode/codicons/src/icons/debug-stop.svg?react";
import Sparkle from "@vscode/codicons/src/icons/sparkle.svg?react";
import Tools from "@vscode/codicons/src/icons/tools.svg?react";
import Save from "@vscode/codicons/src/icons/save.svg?react";
import Trash from "@vscode/codicons/src/icons/trash.svg?react";
import LinkExternal from "@vscode/codicons/src/icons/link-external.svg?react";
import ChevronLeft from "@vscode/codicons/src/icons/chevron-left.svg?react";
import ChevronRight from "@vscode/codicons/src/icons/chevron-right.svg?react";
import ChevronUp from "@vscode/codicons/src/icons/chevron-up.svg?react";
import ChevronDown from "@vscode/codicons/src/icons/chevron-down.svg?react";

export type SvgComponent = FunctionComponent<SVGProps<SVGSVGElement>>;

/**
 * Every Codicon name used anywhere in the React panels. Adding a new icon
 * SHALL require:
 *
 *   1. Adding the name here.
 *   2. Adding a corresponding `?react` import above and an entry in `codiconComponents`.
 *   3. If the icon is also needed in the native chrome, adding the
 *      `IconId` value in `app/browser/icon_registry.h` and a matching
 *      entry to `kSpecs[]` in `app/browser/icon_registry.cc`.
 */
export type IconName =
  | "arrow-left"
  | "arrow-right"
  | "refresh"
  | "close"
  | "add"
  | "settings-gear"
  | "terminal"
  | "comment-discussion"
  | "type-hierarchy"
  | "globe"
  | "debug-stop"
  | "sparkle"
  | "tools"
  | "save"
  | "trash"
  | "link-external"
  | "chevron-left"
  | "chevron-right"
  | "chevron-up"
  | "chevron-down";

/** SVG React components for each icon, transformed at build time by vite-plugin-svgr. */
export const codiconComponents: Record<IconName, SvgComponent> = {
  "arrow-left": ArrowLeft,
  "arrow-right": ArrowRight,
  refresh: Refresh,
  close: Close,
  add: Add,
  "settings-gear": SettingsGear,
  terminal: Terminal,
  "comment-discussion": CommentDiscussion,
  "type-hierarchy": TypeHierarchy,
  globe: Globe,
  "debug-stop": DebugStop,
  sparkle: Sparkle,
  tools: Tools,
  save: Save,
  trash: Trash,
  "link-external": LinkExternal,
  "chevron-left": ChevronLeft,
  "chevron-right": ChevronRight,
  "chevron-up": ChevronUp,
  "chevron-down": ChevronDown,
};

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
