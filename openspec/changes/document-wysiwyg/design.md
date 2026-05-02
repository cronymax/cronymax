## Context

The Document workbench currently exposes a raw markdown textarea + rendered preview, with comments anchored by `rev=<n> lines=<a>-<b>` (free-form `anchor` string in `DocComment`). Editors find raw markdown disruptive for prose-heavy artifacts (PRDs, tech specs); reviewers find line-range anchoring brittle (any insertion above a comment shifts it). Persisted state lives in `<workspace>/.cronymax/flows/<flow>/runs/<run-id>/reviews.json` (see `src/document/reviews_state.h`) and source files at `<flow>/docs/<name>.md` with immutable snapshots in `.history/`.

This change keeps the on-disk format git-trackable but layers a Notion-style WYSIWYG editor on top, anchors comments to stable block IDs persisted inside the markdown itself, and adds a Monaco-powered diff view between revisions.

Constraints:

- C++ side stays no-exceptions, hand-written `JsonValue` (no nlohmann); changes to `DocComment` must remain forward-compatible (legacy `anchor` field preserved).
- Renderer is React 19 + TS strict + Tailwind v4 + multi-entry Vite. Bridge framing is `{channel, payload}` with payload as JSON-string.
- Build constraint: `CRONYMAX_BUILD_APP=OFF` in current cmake cache; verify C++ via clangd diagnostics, not full link.
- No real-time collaboration (single author per session, existing per-doc write lock from `DocumentStore` remains the conflict mechanism).
- Bundle: workbench route lazy-loads Milkdown + Monaco; static panels (channel/inbox) MUST NOT pull these in.

Stakeholders: Document authors (agents + humans), reviewers (agents + humans), Flow owners.

## Goals / Non-Goals

**Goals:**

- WYSIWYG markdown editing with deterministic, idempotent serialization (round-trip stable on the markdown subset Milkdown emits).
- Stable block-ID anchoring: a comment placed on block `b-abc` survives any edit that does not delete that block, including insertions above/below.
- 3-mode workbench: WYSIWYG / Source / Diff. Toggle is local UI state; no backend changes per mode.
- Comment rail viewport-aware: shows comments anchored to currently visible blocks, with click-to-scroll and highlight.
- Suggested-edits flow: reviewer comment can carry `suggestion: <markdown>`; author can accept (one-click apply, replacing anchored block's content) or dismiss.
- Migration: legacy line-range comments converted in-place on first load with `legacy_anchor` retained for audit; idempotent on second load.
- One new bridge channel `document.suggestion.apply`; all other backend surfaces unchanged.

**Non-Goals:**

- Real-time multi-author editing (no Y.js, no CRDT).
- Visual regression testing (manual QA acceptable for v1).
- WYSIWYG export to non-markdown formats (PDF/HTML/DOCX).
- Editing or annotating the diff view (diff is read-only).
- Block IDs for inline-level elements (only top-level blocks: paragraphs, headings, lists, code blocks, tables, blockquotes, thematic breaks).

## Decisions

### D1: Editor library = Milkdown (over Lexical / TipTap / Slate)

**Rationale:** Milkdown is markdown-native — it parses to a ProseMirror schema designed around CommonMark + GFM, and round-trips to markdown via `@milkdown/transformer`. Lexical and Slate have no first-class markdown serializer (we'd hand-write a ProseMirror-style transformer). TipTap is also ProseMirror-based but its markdown plugin is community-maintained and lossy on tables/footnotes. Milkdown's `@milkdown/preset-commonmark` + `@milkdown/preset-gfm` cover the subset we need.

**Alternatives considered:**

- _Lexical_: smaller bundle, faster, but markdown is a community plugin and we'd own the serializer.
- _TipTap_: largest community, but markdown is not the source of truth — would need round-trip golden tests on every release.
- _CodeMirror 6 with markdown highlighting_: keeps source-as-truth, but is not WYSIWYG.

### D2: Block IDs persisted as `<!-- block: <uuid> -->` HTML comments above the block

**Rationale:** Survives plain markdown viewers (renders as nothing), git-diffs cleanly (one line per block when stable), and is parseable by Milkdown via a custom node attribute (`data-block-id`) wired through a remark plugin. Alternative `data-block-id` attribute on the rendered HTML breaks markdown round-trip (Milkdown would emit raw HTML blocks).

UUIDs use the existing `cronymax::uuid_v7` helper for monotonic IDs; first-save assigns IDs to all top-level blocks.

**Alternatives considered:**

- _Heading-slug anchors_: not all blocks are under headings; slug collisions on duplicate headings.
- _Front-matter map (`block_ids: {b1: "p1", b2: "p2"}`)_: fragile when blocks reorder — order-based mapping defeats the purpose.
- _Append `{#b-abc}` Pandoc-style attributes_: not supported by CommonMark; Milkdown won't preserve them on round-trip.

### D3: `reviews.json` schema additions are additive, not breaking

- New optional fields on `DocComment`: `block_id` (string, empty = legacy line-anchored), `suggestion` (string, empty = no suggestion), `legacy_anchor` (string, copy of pre-migration `anchor`).
- `DocComment::anchor` field stays as the human-readable form (e.g. `"block=b-abc"` or `"rev=2 lines=10-12"`) for backward compatibility.
- Reader code that ignores unknown JSON keys (existing `JsonValue` does) continues to work.

### D4: Migration runs lazily in `ReviewStore::Load`

On first load of a `reviews.json`, if any `DocComment.block_id` is empty AND the source markdown has block IDs assigned, attempt to map line ranges → block IDs by reading the recorded revision (`revisions[?].sha` matches; we re-read `<flow>/docs/.history/<name>.<rev>.md`). On match, fill `block_id` and copy old `anchor` to `legacy_anchor`. On mismatch (revision file missing, line out of bounds), leave `block_id` empty and the comment renders in a "Legacy comments" group below the rail.

This is idempotent: second load skips comments that already have `block_id` set.

### D5: Diff view = Monaco DiffEditor over markdown source (not WYSIWYG diff)

**Rationale:** Diffing rendered HTML is unreliable; diffing markdown source is the ground truth. Monaco's `DiffEditor` is industry-standard, supports inline + side-by-side, and is already a dependency we accept (its bundle cost is one-time when the workbench loads). The existing Source-mode editor and the diff editor share the Monaco instance.

### D6: `document.suggestion.apply` is the only new bridge channel

**Surface:**

```
req: { flow: string, run_id: string, name: string, comment_id: string }
res: { ok: true, new_revision: int, sha: string }
```

**Backend behaviour:** Reads `reviews.json`, finds the comment by id, validates it has a non-empty `suggestion` and `block_id`, reads the current doc revision, replaces the block's content (between its `<!-- block: <id> -->` marker and the next block marker / EOF), submits the new content via `DocumentStore::Submit` (gets new revision + sha), and marks the comment's `resolved_in_rev` to the new revision in `reviews.json`. Emits an `AppEvent` of `kind: document_event` for the new revision via the existing `EventBus`.

### D7: Workbench mode is URL-driven for deep-linkability

- `web/document/workbench.html?flow=<id>&doc=<name>&mode=wysiwyg` (default)
- `mode=source` for raw markdown editing
- `mode=diff&from=<rev>&to=<rev>` for revision diff
- Block deep link: `#block-<uuid>` scrolls + highlights, regardless of mode (in source mode, scrolls to the line containing the marker comment).

## Risks / Trade-offs

- **[Risk]** Milkdown serializer drift on minor version bump → markdown round-trip changes, polluting git diffs.
  **Mitigation:** Pin `@milkdown/*` exact versions in `package.json` (no `^`); add a golden-file round-trip test (Group 9.1) covering the markdown subset we care about; CI fails on any change to the golden output without an accompanying test update.

- **[Risk]** Block-ID assignment on first save adds N HTML comments, polluting the very first git commit of an existing doc.
  **Mitigation:** Run a one-shot migration script (Group 9.2) that assigns block IDs to all existing docs in a single commit, separate from any content changes.

- **[Risk]** Monaco bundle is ~3 MB minified; loading it in the renderer first time is slow.
  **Mitigation:** Lazy-load Monaco only when the workbench route is opened (dynamic import); other panels (channel/editor/inbox) never see it. Verify via Vite's `--mode=analyze` output.

- **[Risk]** Suggested-edit `apply` could clobber an in-flight agent write to the same doc (race between human accepting suggestion and agent submitting next revision).
  **Mitigation:** `document.suggestion.apply` goes through `DocumentStore::Submit` which already takes the per-doc flock; on contention, the call returns 409 and the renderer shows a "Doc is being updated, please reload" banner.

- **[Risk]** Block ID for a heading may be visually jarring in a plain markdown viewer (the comment appears just above the heading).
  **Mitigation:** Comments render as zero-height in every common markdown renderer (GitHub, GitLab, VS Code preview, pandoc); manually verified before merge.

- **[Trade-off]** We don't migrate truly orphan legacy comments (no matching revision file). They render in a separate "Legacy" rail group; users can re-anchor manually.

## Migration Plan

1. **Pre-deploy:** ship the change behind a per-Space feature flag `wysiwyg_workbench_enabled` (default OFF). Existing source/preview workbench remains the default for one release.
2. **First open with flag ON:** workbench loads in WYSIWYG mode by default; on save, all existing top-level blocks get IDs assigned (one-time per doc).
3. **`reviews.json` migration:** on first `ReviewStore::Load` after upgrade, comments without `block_id` are mapped to block IDs by looking up the revision file (idempotent on second load).
4. **Rollback:** set `wysiwyg_workbench_enabled = false` per Space; UI reverts to source/preview. The block ID comments in markdown remain (harmless) and `reviews.json` retains the new fields (ignored by old code).
5. **Removal of legacy workbench:** in the release after this one, after the flag has been ON by default for a full release cycle, delete the textarea workbench code path.

## Open Questions

- **OQ-1:** Should the suggestion-apply path emit a new `review_event` (kind = `suggestion_applied`) in addition to the `document_event`? Default: yes, for channel-view auditability.
- **OQ-2:** When a block is split (one paragraph becomes two), do existing comments on it stay on the first half or both halves? Default: stay on the first half (the half that retains the ID); user can manually move.
- **OQ-3:** Should the diff view support 3-way diff (current vs. ancestor vs. suggestion)? Default: no for v1; revisit if reviewers ask.
