## ADDED Requirements

### Requirement: Milkdown-based WYSIWYG editor

The system SHALL provide a Milkdown-based WYSIWYG markdown editor as the default editing surface in the Document workbench. The editor SHALL be loaded only when the workbench route is opened (lazy-loaded) so that other panels (channel, editor, inbox) do not bear the bundle cost.

#### Scenario: Workbench opens in WYSIWYG mode by default

- **WHEN** a user navigates to `web/document/workbench.html?flow=<id>&doc=<name>`
- **THEN** the WYSIWYG editor renders the document content; the URL effectively reads `&mode=wysiwyg`; no Monaco bundle is loaded yet

#### Scenario: WYSIWYG bundle is not loaded for non-workbench panels

- **WHEN** any of `channel.html`, `editor.html`, or `inbox.html` is opened in a fresh renderer process
- **THEN** the network log shows no Milkdown or Monaco chunks fetched

---

### Requirement: Workbench mode toggle

The workbench SHALL expose three modes — `wysiwyg`, `source`, `diff` — switchable via a header toggle. The selected mode SHALL be reflected in the URL query string so the surface is deep-linkable.

#### Scenario: Source mode shows raw markdown

- **WHEN** the user clicks the "Source" toggle
- **THEN** the URL updates to include `&mode=source`; the WYSIWYG editor unmounts; a Monaco editor renders the raw markdown including any `<!-- block: <uuid> -->` markers

#### Scenario: Diff mode requires from/to revisions

- **WHEN** the user clicks the "Diff" toggle
- **THEN** the URL updates to include `&mode=diff&from=<latest-1>&to=<latest>` by default; the workbench loads the Monaco DiffEditor and shows the markdown diff

#### Scenario: Mode survives reload

- **WHEN** the user reloads the page while in `mode=source`
- **THEN** the source editor reopens (no flash of WYSIWYG)

---

### Requirement: Deterministic markdown serialization

The Milkdown serializer SHALL produce byte-stable output for the same parsed document. The system SHALL include a golden-file round-trip test asserting that for every fixture in `web/test/fixtures/markdown/`, `parse(md) → serialize → equals md`.

#### Scenario: Round-trip is idempotent

- **WHEN** a fixture markdown file is parsed and re-serialized
- **THEN** the output equals the input byte-for-byte

#### Scenario: Pinned dependency versions

- **WHEN** an engineer inspects `web/package.json`
- **THEN** every `@milkdown/*` dependency is pinned to an exact version (no `^` or `~`)

---

### Requirement: Source mode editor

When the workbench is in `source` mode, the system SHALL present a Monaco-based markdown editor (read-write) over the document's raw markdown bytes. Saving from source mode SHALL go through the same `document.submit` bridge channel as WYSIWYG saves.

#### Scenario: Source edit persists

- **WHEN** the user edits markdown in source mode and saves
- **THEN** the new content is written to `<flow>/docs/<name>.md` and a new revision snapshot is added to `.history/`

#### Scenario: Block markers are visible and editable

- **WHEN** the user views the markdown in source mode
- **THEN** `<!-- block: <uuid> -->` comments are visible above each top-level block and may be edited or deleted (deleting one orphans any comments anchored to that block)

---

### Requirement: Lazy-loaded heavy editor bundles

Milkdown and Monaco bundles SHALL be loaded via dynamic `import()` and SHALL not appear in the initial page chunk for any panel.

#### Scenario: Vite chunk analysis

- **WHEN** the engineer runs `pnpm build` and inspects the chunk graph
- **THEN** no entry chunk for `channel`, `editor`, or `inbox` references the Milkdown or Monaco modules; the workbench entry's initial chunk also excludes them (they are split into their own dynamic chunks)
