## Why

The earlier changes ship Document collaboration with a raw markdown editor (source view + rendered preview). For frequent human reviewers and authors, raw markdown is friction; comment anchoring by line number breaks under edits; reviewing v2 vs v1 means scanning text. This change upgrades the Document workbench to a Notion-quality editing experience — WYSIWYG markdown editing, stable block-anchored comments that survive edits, and a Monaco-powered revision diff view — without losing markdown-source fidelity or git-trackability.

## What Changes

- **NEW** WYSIWYG markdown editor based on **Milkdown** (ProseMirror, markdown-native). Replaces the raw textarea in the Document workbench. Source toggle and Preview toggle remain available.
- **NEW** Deterministic markdown serialization: configured Milkdown schema produces stable output for the same document (no random whitespace, stable heading slugs, no incidental HTML). Validates that round-trip (`md → editor → md`) is idempotent in tests.
- **NEW** Block-ID anchoring: every top-level block gets a stable UUID assigned on first save. IDs persist in the markdown via a `data-block-id` attribute (HTML comment fallback for plain markdown viewers). Comments anchor to block IDs, surviving edits within other blocks.
- **NEW** Comment rail UI: side rail in the workbench shows comments anchored to the current viewport's blocks; clicking a comment scrolls to the anchored block and highlights it. New comments created via text-selection toolbar.
- **NEW** Revision diff view: a third workbench mode (`Diff v_n ↔ v_{n+1}`) using **Monaco DiffEditor** over the markdown source. Available regardless of editor mode (WYSIWYG/Source). Reviewers use this to see what changed between revisions.
- **NEW** Suggested-edits flow: a reviewer comment may include a `suggestion:` markdown block; the author can accept it (one-click apply to the doc) or dismiss. Modeled like GitHub's suggested changes.
- **NEW** Real-time collaboration is **NOT** in scope; the editor remains single-author per session. Conflict avoidance via existing per-doc write-lock from `document-collaboration`.
- **MODIFIED** `document-collaboration`: comments migrate from line-range anchors to block-ID anchors. Migration tool converts existing `reviews.json` files. The `reviews.json` schema gains a `block_id` field (line-range kept as fallback for legacy comments).
- **MODIFIED** `flow-channel-view`: channel-thread comments link to the workbench's block-anchored view (`#block-<uuid>` deep links).

## Capabilities

### New Capabilities

- `document-wysiwyg-editor`: Milkdown-based WYSIWYG editor, deterministic serialization, source/preview/diff modes.
- `block-anchored-comments`: block-ID assignment, persistence in markdown, comment-rail UI, anchor stability across edits, deep links.
- `revision-diff-view`: Monaco DiffEditor over revision pairs, side-by-side and inline modes, suggested-edits accept/dismiss.

### Modified Capabilities

- `document-collaboration`: comment anchoring switches to block-ID; `reviews.json` schema migration; suggested-edits semantics.
- `flow-channel-view`: thread replies deep-link to anchored blocks.

## Impact

- **Frontend deps added**: `@milkdown/core` and plugins, `monaco-editor`, `prosemirror-changeset` (for in-editor change indicators). Bundle size grows ~MB-scale; lazy-load the workbench route.
- **`web/document/`** workbench refactor: 3-mode toggle, comment rail, suggestion UI.
- **`src/document_store/`**: block-ID assignment on first save; migration script for existing docs.
- **Migration**: existing `reviews.json` files converted in-place on first load (line ranges preserved as `legacy_anchor`); idempotent and reversible from `.history/`.
- **Test surface**: golden round-trip tests for serialization stability across the markdown subset Milkdown emits; visual regression tests are NOT in scope (manual QA acceptable for v1 of this change).
- **No new C++ deps** — entirely a frontend / renderer change. Bridge surface unchanged except for one new `document.suggestion.apply` channel.
