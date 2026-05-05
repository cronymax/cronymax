/**
 * useSelectionTooltip — tracks text selection within the block timeline.
 *
 * Returns the selected text, the closest ancestor block ID, and the screen
 * rect of the selection range (used to position the floating action tooltip).
 */
import { useEffect, useRef, useState } from "react";

export interface SelectionInfo {
  selectedText: string;
  blockId: string;
  /** DOMRect of the selection in viewport coordinates */
  anchorRect: DOMRect;
}

export function useSelectionTooltip(
  /** ref to the scrollable timeline container */
  containerRef: React.RefObject<HTMLElement | null>,
): SelectionInfo | null {
  const [info, setInfo] = useState<SelectionInfo | null>(null);
  // Debounce timer
  const timerRef = useRef<ReturnType<typeof setTimeout> | null>(null);

  useEffect(() => {
    const handle = () => {
      if (timerRef.current) clearTimeout(timerRef.current);
      timerRef.current = setTimeout(() => {
        const sel = window.getSelection();
        if (!sel || sel.rangeCount === 0 || sel.isCollapsed) {
          setInfo(null);
          return;
        }
        const text = sel.toString().trim();
        if (!text) {
          setInfo(null);
          return;
        }

        // Walk up the DOM from the anchor node to find [data-block-id]
        let node: Node | null = sel.anchorNode;
        let blockId: string | null = null;
        while (node) {
          if (node instanceof Element) {
            const id = node.getAttribute("data-block-id");
            if (id) {
              blockId = id;
              break;
            }
          }
          node = node.parentNode;
        }
        if (!blockId) {
          setInfo(null);
          return;
        }

        // Make sure selection is inside our container
        const container = containerRef.current;
        if (container) {
          const range = sel.getRangeAt(0);
          if (!container.contains(range.commonAncestorContainer)) {
            setInfo(null);
            return;
          }
        }

        const range = sel.getRangeAt(0);
        const rect = range.getBoundingClientRect();
        setInfo({ selectedText: text, blockId, anchorRect: rect });
      }, 120);
    };

    document.addEventListener("selectionchange", handle);
    document.addEventListener("mouseup", handle);
    return () => {
      document.removeEventListener("selectionchange", handle);
      document.removeEventListener("mouseup", handle);
      if (timerRef.current) clearTimeout(timerRef.current);
    };
  }, [containerRef]);

  return info;
}
