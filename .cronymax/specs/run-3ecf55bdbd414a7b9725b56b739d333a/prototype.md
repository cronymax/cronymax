---
title: Markdown Preview in Document Review Card
doc_type: prototype
---

# Prototype: Markdown Preview in Document Review Card

## Problem

The `DocumentCard` component in the **Channel panel** (`web/src/panels/channel/components/DocumentCard.tsx`) is the primary surface humans use to review agent-produced documents. Today it shows only:

- doc path / doc type / revision / producer (metadata row)
- Approve / Request Changes buttons
- A short review history

It does **not** render the document content, so reviewers must open the Workbench panel separately just to read what they are approving. This creates friction and slows down the review loop.

---

## Proposed Change

Render the document's markdown content as a **collapsible read-only preview** directly inside `DocumentCard`, using the existing `WysiwygMarkdown` (Streamdown) component.

### UI Behaviour

1. **Collapsed by default** — the card keeps its compact footprint; a single "Show preview ▾" toggle expands it.
2. **Lazy fetch** — content is loaded on first expand (not on mount), so non-expanded cards cost zero extra IPC calls.
3. **Max-height scrollable** — the preview area is capped at `24rem` with `overflow-y: auto` so very long documents don't take over the feed.
4. **Loading / error states** — a subtle spinner while fetching; an inline error message if the fetch fails.
5. **"Open in Workbench" link** — a small secondary button remains available for full editing.

---

## Affected Files

| File | Change |
|---|---|
| `web/src/panels/channel/components/DocumentCard.tsx` | Add `PreviewPane` sub-component; wire `shells.document.read` call on expand |
| `web/src/components/WysiwygMarkdown/index.tsx` | No change (already read-only via `Streamdown` when `readOnly` prop is set) |

---

## Implementation Sketch

```tsx
// DocumentCard.tsx (additions only)

import { useState } from "react";
import { WysiwygMarkdown } from "@/components/WysiwygMarkdown";
import { shells } from "@/shells/bridge";

// … inside DocumentCard() …

const [previewOpen, setPreviewOpen] = useState(false);
const [previewContent, setPreviewContent] = useState<string | null>(null);
const [previewError, setPreviewError] = useState<string | null>(null);
const [previewLoading, setPreviewLoading] = useState(false);

const togglePreview = async () => {
  if (!previewOpen && previewContent === null && !previewLoading) {
    setPreviewLoading(true);
    setPreviewError(null);
    try {
      const res = await shells.document.read({ flow: flowId, name: thread.doc_path ?? thread.doc_id });
      setPreviewContent(res.content ?? "");
    } catch (err) {
      setPreviewError(err instanceof Error ? err.message : String(err));
    } finally {
      setPreviewLoading(false);
    }
  }
  setPreviewOpen((v) => !v);
};

// … in JSX, after the metadata row, before the action buttons …

<button
  type="button"
  className="mt-2 text-xs text-muted-foreground hover:text-foreground"
  onClick={() => void togglePreview()}
>
  {previewOpen ? "Hide preview ▴" : "Show preview ▾"}
</button>

{previewOpen && (
  <div className="mt-2 max-h-96 overflow-y-auto rounded border border-border bg-card p-3 text-sm">
    {previewLoading && <span className="text-xs opacity-60">Loading…</span>}
    {previewError && <span className="text-xs text-red-400">{previewError}</span>}
    {!previewLoading && !previewError && previewContent !== null && (
      <WysiwygMarkdown value={previewContent} readOnly />
    )}
  </div>
)}
```

---

## Data Flow

```
DocumentCard mounts
  └─ user clicks "Show preview"
       └─ shells.document.read({ flow, name })   [IPC call]
            └─ res.content (markdown string)
                 └─ WysiwygMarkdown readOnly      [Streamdown renderer]
                      └─ HTML rendered inline
```

---

## Scope / Out of Scope

| In scope | Out of scope |
|---|---|
| Read-only preview of current document revision | Inline editing inside the card |
| Collapsible toggle | Auto-expand all cards |
| Lazy load on first expand | Pre-fetching / caching across cards |
| Error / loading states | Diff view vs previous revision |

---

## Open Questions

1. Should the preview auto-expand when a `review_event` with `verdict = "request_changes"` arrives, so the reviewer immediately sees context?
2. Max-height: `24rem` feels right for the chat-feed density — is there a preference for more/less?
3. Should "Open in Workbench" remain as a separate button, or replace it with a header link inside the expanded preview?
