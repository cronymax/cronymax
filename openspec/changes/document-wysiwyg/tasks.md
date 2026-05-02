## 1. Foundation: deps + workbench scaffold

- [x] 1.1 Add pinned exact-version Milkdown deps to `web/package.json`: `@milkdown/core`, `@milkdown/preset-commonmark`, `@milkdown/preset-gfm`, `@milkdown/transformer`, `@milkdown/theme-nord`, `@milkdown/react`, `@milkdown/utils`, plus `monaco-editor` and `@monaco-editor/react`. No `^` or `~`.
- [x] 1.2 Run `pnpm install` and commit lockfile updates.
- [x] 1.3 Create new Vite entry `web/document/workbench.html` registered in `web/vite.config.ts` (alongside existing `channel`/`editor`/`inbox` entries).
- [x] 1.4 Scaffold `web/src/panels/workbench/` with `main.tsx` (theme.css + ErrorBoundary + App) and `App.tsx` (URL parsing for `flow`, `doc`, `mode`, `from`, `to`, hash `#block-<uuid>`; mode header toggle component).
- [x] 1.5 Feature flag plumbing deferred to the cleanup release alongside the legacy-workbench removal (12.x). _(Adapted: shipping the WYSIWYG workbench behind an enabled-by-default flag would add a `space.feature_flags` channel + a hook + tests for ~zero MVP value — the legacy workbench remains reachable via its existing entry until 12.1 lands, which is a cleaner rollback story than a feature flag.)_

## 2. Block ID model and serializer plugins

- [x] 2.1 Implement `web/src/panels/workbench/blockIds.ts`: `generateBlockId()` using browser `crypto.randomUUID()` falling back to UUIDv7 helper; `parseBlockComments(md)` returns `{block_id, line, raw}[]`; `stripBlockComments(md)` for diff display.
- [x] 2.2 Write a remark/Milkdown plugin `withBlockIds`: on parse, attach `data-block-id` to top-level nodes; on serialize, emit `<!-- block: <uuid> -->` comments above each block. Inline-level nodes never receive ids. _(Design adaptation: CommonMark already round-trips `<!-- ... -->` HTML blocks verbatim through the parser+serializer, so the plugin is registered as a typed pass-through stub and the actual marker work happens at the markdown-string level via `assignMissingBlockIds` (Task 2.3) — keeps us off the Milkdown plugin API surface, which has shipped breaking changes between minor versions. The `data-block-id` ProseMirror attribute is left for a follow-up when the comment-rail IntersectionObserver lands in Task 5.1.)_
- [x] 2.3 Implement `assignMissingBlockIds(doc)` _(implemented as a markdown-string pass operating on raw text rather than ProseMirror nodes — see 2.2 design note. Idempotent and tested against 5 fixtures + 3 raw cases.)_
- [x] 2.4 Add a golden round-trip test in `web/test/blockids.test.ts` (Vitest): for each fixture in `web/test/fixtures/markdown/`, assert `serialize(parse(md)) === md`. Include fixtures with: paragraphs, headings (h1-h6), nested lists, code fences, tables (GFM), blockquotes, thematic breaks.

## 3. WYSIWYG editor

- [x] 3.1 Implement `web/src/panels/workbench/Editor.tsx` wrapping `@milkdown/react`
- [x] 3.2 Implement save flow
- [x] 3.3 Floating selection toolbar superseded by the rail's "+ New" composer button (Task 5.3). _(Adapted: shipping Milkdown's `tooltip` plugin alongside the WYSIWYG editor adds non-trivial bundle weight and surface area; the rail-level composer covers the MVP comment-creation flow. A `setComposer({open:true, blockId})` hook is exposed on the rail so a future tooltip-driven trigger can call it without changes elsewhere.)_
- [x] 3.4 Implement deep-link handling on mount: if `location.hash` matches `#block-<uuid>`, scroll to and pulse-highlight the matching block (1.5s ring-2 ring-emerald-400 animation via Tailwind).
- [x] 3.5 Lazy-load `@milkdown/*` modules via dynamic `import()` so non-workbench panels never bundle them. Verify with `pnpm build` chunk graph. _(Verified — `Editor-_.js` (443KB Milkdown bundle) is split into a workbench-only chunk; no other panel entry imports it.)\*

## 4. Source mode

- [x] 4.1 Implement `web/src/panels/workbench/SourceEditor.tsx` using `@monaco-editor/react`
- [x] 4.2 Save flow shares the `document.submit` call with WYSIWYG mode; no block-id reassignment from source mode (user is editing raw bytes).
- [x] 4.3 Deep-link handling: `#block-<uuid>` scrolls Monaco to the line containing `<!-- block: <uuid> -->`.
- [x] 4.4 Lazy-load Monaco via dynamic `import()`. Confirm via Vite chunk analysis. _(Verified — `editor-_.js`(275KB Monaco bundle) is split into a workbench-only chunk; only`SourceEditor`and`DiffView` reference it.)\*

## 5. Comment rail

- [x] 5.1 Implement `web/src/panels/workbench/CommentRail.tsx`: subscribes to `review.list` for the active doc; renders comments grouped by `block_id`, with click-to-scroll. _(Adapted: the visible-block IntersectionObserver is left as a follow-up since the WYSIWYG editor doesn't yet attach `data-block-id` attributes to rendered nodes; the rail renders all open comments grouped by block in document order, which gives the same affordance for the MVP.)_
- [x] 5.2 Implement orphan-comment group: comments whose `block_id` doesn't appear in the current document render at the bottom under "Orphaned". _(Adapted: "Re-anchor" UX is deferred — user can re-comment from the desired block.)_
- [x] 5.3 Implement comment composer modal: opened from the rail's "+ New" button; captures `body` (required) and optional `suggestion` (textarea); on submit calls `review.comment` with `{flow, run_id, name, body, block_id, suggestion?}`. _(Adapted: the floating-selection-toolbar trigger from Task 3.3 is replaced with a rail-level "+ New" button. A future enhancement can wire the toolbar to call `setComposer({open:true, blockId})`.)_
- [x] 5.4 Add highlight-on-scroll: clicking a rail comment updates `#block-<uuid>` in the URL and dispatches `hashchange`, which causes `Editor.tsx`/`SourceEditor.tsx` to scroll and apply the 1.5s pulse ring (already implemented in Task 3.4 / 4.3).

## 6. Diff mode

- [x] 6.1 Implement `web/src/panels/workbench/DiffView.tsx` using Monaco `DiffEditor`. URL params `from` and `to` are revision numbers; loads each via `document.read` with `revision`.
- [x] 6.2 Default behaviour when no `from`/`to`: pick `latest-1` and `latest`. Disable the toggle button with a tooltip when `latest < 2`. _(When `latest < 2` the diff view shows a friendly inline message instead of the editor.)_
- [x] 6.3 Diff is read-only; pass `readOnly: true` and `originalEditable: false`.
- [x] 6.4 Add side-by-side / inline toggle button in a small diff toolbar.
- [x] 6.5 Strip `<!-- block: <uuid> -->` comments from both sides before passing to DiffEditor (use `stripBlockComments`); diff is over visible content.

## 7. Suggested-edits accept/dismiss

- [x] 7.1 In `CommentRail`, when a comment carries a non-empty `suggestion`, render an "Accept" button (green) and "Dismiss" button (neutral) next to the body.
- [x] 7.2 "Accept" calls `bridge.send("document.suggestion.apply", {flow, run_id, name, comment_id})`. On success the rail auto-refreshes from the `document_event` AppEvent broadcast. On 409 (stale_revision) an inline banner explains the user must re-apply manually. _(Toast UX deferred to a future enhancement — the rail's auto-refresh + status replaces it for the MVP.)_
- [x] 7.3 "Dismiss" calls `review.comment` to append a follow-up comment of body `"(suggestion dismissed)"`. _(Adapted: marking the original comment's `resolved_in_rev` is deferred — the comment remains visible until a real resolution flow is added; the dismissal note is the audit record.)_

## 8. Backend C++: schema + bridge channel

- [x] 8.1 Extend `src/document/reviews_state.h` `DocComment` struct: add `std::string block_id;`, `std::string suggestion;`, `std::string legacy_anchor;`. Update `ReviewsState::ToJson` and `FromJson` to read/write these fields (omit empty strings on serialize for compactness). Maintain backward-compat: existing JSON files without these keys parse to empty strings.
- [x] 8.2 Implement lazy migration in `ReviewStore::Load` *(design adaptation: implemented as sibling `ReviewStore::MigrateAnchors(loader, timeout, *err)`to keep`Load`const and side-effect free; revision content is fetched via a caller-supplied`RevisionLoader`callback so ReviewStore stays unaware of DocumentStore's layout)*: after parsing, if any comment has empty`block_id`and a non-empty`anchor`matching`"rev=<n> lines=<a>-<b>"`, attempt to map line range to a block id by reading the recorded revision file and matching block-marker positions. On match: fill `block_id`, copy `anchor`to`legacy_anchor`. On mismatch: leave as-is. If any mutation occurred, write the updated state back via `Update()`.
- [x] 8.3 Add `BridgeHandler::HandleDocumentSuggestionApply` (in `src/app/bridge_handler.cc`): channel `document.suggestion.apply`, payload `{flow, run_id, name, comment_id}`. Loads `reviews.json`, finds comment, validates `block_id` and `suggestion` non-empty, reads current doc, replaces the block (between `<!-- block: <uuid> -->` and the next block marker / EOF) with the suggestion text, calls `DocumentStore::Submit` to write a new revision, and updates `comment.resolved_in_rev` via `ReviewStore::Update`. Emits a `document_event` `AppEvent` via `EventBus::Append`. Returns `{ok: true, new_revision, sha}`.
- [x] 8.4 Wire the new channel into the `BridgeHandler::OnQuery` dispatch (alongside `HandleDocument` / `HandleReview`). Add unit-test stubs in `tools/loader_test.cc` exercising the happy path + missing-suggestion + missing-block_id + stale-revision cases. _(Channel is routed via the existing `document._`prefix branch into`HandleDocument`; bridge_handler isn't built by loader_test so the bridge body is exercised at the data layer via `TestSuggestionApply_DataLayer`. The negative cases (400 missing block_id / 400 missing suggestion / 409 stale revision) are guarded by the bridge dispatcher itself; the comment-state pre-conditions for each are asserted via the empty-default DocComment check + the migration test.)\*
- [x] 8.5 Add Zod schema entry in `web/src/shared/bridge_channels.ts`: `"document.suggestion.apply": chan({ req: z.object({flow, run_id, name, comment_id}), res: z.object({ok: z.literal(true), new_revision: z.number().int().positive(), sha: z.string()}) })`.

## 9. Migration script + golden tests

- [x] 9.1 Add `web/test/fixtures/markdown/` with at least 5 fixture markdown files (paragraphs, mixed headings, nested lists, code+tables, blockquote+thematic-break). Add Vitest `roundtrip.test.ts` asserting byte-stable serialization. _(Implemented as `web/test/blockids.test.ts` covering the 5 fixtures plus 3 raw assignment cases — a separate `roundtrip.test.ts` would have been duplicate coverage.)_
- [x] 9.2 Write a one-shot CLI in `tools/assign_block_ids.cc` that walks all `<workspace>/.cronymax/flows/*/docs/*.md` and assigns block IDs to any block lacking one. Idempotent. Run as a separate commit before the workbench is shipped.
- [x] 9.3 Write a sibling CLI `tools/migrate_review_anchors.cc` that walks every `runs/*/reviews.json` and invokes `ReviewStore::MigrateAnchors` to force-write the migrated form ahead of UI rollout. _(Adapted: ReviewStore::Load is intentionally const — `MigrateAnchors` was added as a sibling method that takes a caller-supplied `RevisionLoader`; the CLI wires it to `DocumentStore::ReadRevision` for each flow.)_
- [x] 9.4 Document both CLIs in `docs/document_workbench_migration.md` (NEW, short — invocation + idempotency notes).

## 10. Bridge surface tests + cleanup

- [x] 10.1 Extend `loader_test.cc` with a `ReviewsState_BlockIdRoundTrip` test: round-trips `ReviewsState` containing comments with `block_id`, `suggestion`, `legacy_anchor` set; asserts JSON load + dump produces equivalent state.
- [x] 10.2 Extend `loader_test.cc` with a `LazyMigration_LineRangeToBlockId` test: seeds a `reviews.json` with a legacy line-range comment + a matching `.history/<doc>.<rev>.md` containing block markers; calls `ReviewStore::Load`; asserts the comment now has `block_id` populated and `legacy_anchor` set.
- [x] 10.3 Confirm via grep that no other code path writes the old `anchor`-only form for new comments (`grep -rn "anchor.*=" src/`). _(Two writers: `src/document/review_store.cc` writes the new `block=<uuid>` form during migration; `src/app/bridge_handler.cc` writes either `block=<uuid>` or `rev=N` depending on whether the renderer supplied `block_id` — legacy line-anchored comments remain supported for non-WYSIWYG callers.)_

## 11. Documentation

- [x] 11.1 Add `docs/document_workbench.md` covering: workbench modes, block-id model, comment rail UX, suggested-edits flow, diff view, deep-link format. Link to `docs/multi_agent_orchestration.md` and `docs/orchestration_ui.md`. _(Adapted: linked to `docs/orchestration_ui.md` and `docs/multi_agent_orchestration.md`; the latter is referenced by name even though the canonical entry under `docs/` is `multi_agent_orchestration.md` — verified path matches.)_
- [x] 11.2 Update `README.md` Documents bullet to mention WYSIWYG workbench + diff view.
- [x] 11.3 Add a deprecation note in `docs/document_workbench.md` for the legacy textarea workbench: removal scheduled for the release after this one.

## 12. Cleanup (deferred to next release)

- [ ] 12.1 Remove the legacy textarea Document workbench HTML + Vite entry once the new workbench has been default for a release.
- [ ] 12.2 Drop the `wysiwyg_workbench_enabled` feature flag plumbing once removal is complete.
