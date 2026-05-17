/**
 * PanelWindowHeader — wraps the topmost row of a panel page so that,
 * when the page is hosted in a standalone PanelWindow, it
 *   1. leaves room for the OS window controls that overlay the corners
 *      (traffic lights at top-left on macOS, min/max/close at top-right
 *      on Windows), and
 *   2. acts as a draggable strip so the user can move the window even
 *      though there is no visible system titlebar
 *      (`NSWindowStyleMaskFullSizeContentView` on macOS hides it).
 *
 * In tab context — the same panel HTML loaded into a regular tab inside
 * the main window — none of this is needed: there is no overlay, no
 * detached window to drag. The component therefore renders as a
 * transparent pass-through, applying no extra padding or drag class.
 *
 * Detection — `installPanelMode` sets `data-panel-window` on `<html>`
 * when the page URL ends in `#panel`, which only PanelWindow appends.
 * Platform is sniffed via `navigator.platform`; the wrap is currently
 * only active on macOS because Windows panel windows still use the
 * standard system chrome (the native `StylePanelWindow` is a no-op
 * outside the macOS implementation).
 */
import { type ElementType, forwardRef, type HTMLAttributes } from "react";
import { cn } from "@/lib/utils";

const isMac = typeof navigator !== "undefined" && /Mac/i.test(navigator.platform);

const isPanelWindow = (): boolean => {
  if (typeof document === "undefined") return false;
  return document.documentElement.hasAttribute("data-panel-window");
};

interface PanelWindowHeaderProps extends HTMLAttributes<HTMLElement> {
  /** Render as a `<header>` (default) or any other element for semantics. */
  as?: ElementType;
}

export const PanelWindowHeader = forwardRef<HTMLElement, PanelWindowHeaderProps>(
  ({ as: Tag = "header", className, children, ...rest }, ref) => {
    const active = isPanelWindow();
    // Mac: traffic lights occupy ~80 px top-left → `pl-20`.
    // Windows panel windows currently inherit the standard system
    // titlebar (no overlay), so they need no padding. When that changes,
    // add `active && !isMac && "app-drag pr-32"`.
    const overlayClasses = active && isMac ? "app-drag pl-20" : undefined;
    const Component = Tag as ElementType;
    return (
      <Component ref={ref} className={cn(className, overlayClasses)} {...rest}>
        {children}
      </Component>
    );
  },
);
PanelWindowHeader.displayName = "PanelWindowHeader";
