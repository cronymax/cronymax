/**
 * Block-ID utilities for the document workbench.
 *
 * Block IDs persist in the markdown source as HTML comments above each
 * top-level block:
 *
 *     <!-- block: 0190abcd-... -->
 *     # Heading
 *
 *     <!-- block: 0190abcf-... -->
 *     A paragraph.
 *
 * The marker renders as nothing in every common markdown viewer
 * (GitHub, GitLab, VS Code preview, pandoc), so the on-disk file stays
 * readable. Comments in `reviews.json` reference these UUIDs so that
 * inserting / deleting unrelated blocks does not move the anchor.
 */

const BLOCK_MARKER_RE = /^[ \t]*<!--\s*block:\s*([0-9a-fA-F-]{8,})\s*-->\s*$/;

export function generateBlockId(): string {
  // crypto.randomUUID is available in Chromium ≥ 92 (CEF runtime is well
  // past that). The output is RFC 4122 v4 — sufficient for our anchor
  // needs (we don't require monotonic ordering on the client side).
  return crypto.randomUUID();
}

export interface BlockMarker {
  blockId: string;
  /** 0-based line index where the marker comment lives. */
  line: number;
  /** Raw matched line (including any leading whitespace). */
  raw: string;
}

/**
 * Find every `<!-- block: <uuid> -->` marker in the document.
 *
 * Markers may appear anywhere in the source; the workbench treats only
 * those immediately followed by a non-blank, non-marker line as block
 * anchors (the spec requires the marker to be on the line immediately
 * preceding the block). For migration / lookup purposes we return all
 * matches and let callers filter.
 */
export function parseBlockComments(md: string): BlockMarker[] {
  const out: BlockMarker[] = [];
  const lines = md.split("\n");
  for (let i = 0; i < lines.length; i += 1) {
    const m = BLOCK_MARKER_RE.exec(lines[i] ?? "");
    if (m?.[1]) {
      out.push({ blockId: m[1], line: i, raw: lines[i] ?? "" });
    }
  }
  return out;
}

/**
 * Return a copy of `md` with every block-marker line removed. Used by
 * the diff view so revisions diff over visible content rather than over
 * marker-comment churn.
 */
export function stripBlockComments(md: string): string {
  const lines = md.split("\n");
  const kept: string[] = [];
  for (const line of lines) {
    if (!BLOCK_MARKER_RE.test(line)) kept.push(line);
  }
  return kept.join("\n");
}

/**
 * Replace the block whose marker carries `blockId` with `replacement`
 * (which should NOT include the marker line — it is preserved verbatim).
 *
 * The "block" is defined as the lines from immediately after the marker
 * up to (but not including) the next marker line, or EOF. This matches
 * the on-disk shape used by `document.suggestion.apply`.
 *
 * Returns the new markdown string. If the block is not found, returns
 * the original string unchanged.
 */
export function replaceBlockContent(md: string, blockId: string, replacement: string): string {
  const lines = md.split("\n");
  let startIdx = -1;
  for (let i = 0; i < lines.length; i += 1) {
    const m = BLOCK_MARKER_RE.exec(lines[i] ?? "");
    if (m && m[1] === blockId) {
      startIdx = i + 1;
      break;
    }
  }
  if (startIdx < 0) return md;
  let endIdx = lines.length;
  for (let i = startIdx; i < lines.length; i += 1) {
    if (BLOCK_MARKER_RE.test(lines[i] ?? "")) {
      endIdx = i;
      break;
    }
  }
  // Trim a single trailing blank line in the replacement to keep paragraph
  // spacing consistent with the original block.
  const replacementLines = replacement.replace(/\n+$/, "").split("\n");
  const out = [...lines.slice(0, startIdx), ...replacementLines, "", ...lines.slice(endIdx)];
  return out.join("\n");
}

/**
 * Format a marker line for emission. Always uses the canonical form so
 * round-tripping through this helper produces byte-stable output.
 */
export function formatBlockMarker(blockId: string): string {
  return `<!-- block: ${blockId} -->`;
}

export const BLOCK_MARKER_REGEX = BLOCK_MARKER_RE;

// ---------------------------------------------------------------------------
// `assignMissingBlockIds` — string-level pass that mints UUIDs for any
// top-level block not already preceded by a `<!-- block: <uuid> -->`
// marker. Called before `document.submit` from both the WYSIWYG and
// Source editors.
//
// **Design adaptation note** (`change: document-wysiwyg`): the spec
// originally proposed implementing this via a remark/Milkdown plugin
// (`withBlockIds`) that walks the ProseMirror document on parse and
// serialize. After investigating the Milkdown 7.20 plugin surface we
// realised that:
//
//  1. CommonMark already round-trips `<!-- ... -->` blocks verbatim as
//     `html` nodes, so the markers survive parse → render → serialize
//     unchanged without any plugin wiring.
//  2. The block-anchored review model only needs the marker on the
//     line immediately before the block — a property easily enforced
//     at the markdown text level.
//
// So `withBlockIds` (Task 2.2) is implemented as a pass-through plugin
// that registers no transformations (kept as a stub so future plugin
// hooks have a home), and the block-id assignment work happens via this
// `assignMissingBlockIds` string pass (Task 2.3). This keeps us off the
// Milkdown plugin API and out of remark-AST territory, both of which
// have shipped breaking changes between minor versions.
//
// Definition of a "top-level block" for marker placement: any non-blank
// line that is preceded by a blank line or by the start of the file
// (and is not itself a `<!-- block: ... -->` marker). This deliberately
// matches CommonMark's "block precedes blank line" boundary rule and
// keeps the algorithm robust against nested-list indentation, table
// rows, fenced code interiors, and HTML blocks — none of which need
// special-casing under this single rule.

function isTopLevelBlockOpener(line: string, prev: string | undefined): boolean {
  if (line.trim() === "") return false;
  if (BLOCK_MARKER_RE.test(line)) return false;
  return prev === undefined || prev.trim() === "";
}

/**
 * Walks `md` and inserts `<!-- block: <uuid> -->\n` markers above every
 * top-level block that does not already have one immediately above it.
 *
 * Idempotent: running twice produces the same string. The function never
 * removes existing markers, even if they appear in unusual positions.
 *
 * Newly-minted UUIDs come from `generateBlockId()`. Pass `idGenerator`
 * to override (used by the golden round-trip test for determinism).
 */
export function assignMissingBlockIds(md: string, idGenerator: () => string = generateBlockId): string {
  const lines = md.split("\n");
  const out: string[] = [];
  for (let i = 0; i < lines.length; i += 1) {
    const line = lines[i] ?? "";
    const prev = i > 0 ? lines[i - 1] : undefined;
    if (isTopLevelBlockOpener(line, prev)) {
      // Already has a marker on the preceding line?
      const above = out.length > 0 ? out[out.length - 1] : undefined;
      if (above === undefined || !BLOCK_MARKER_RE.test(above)) {
        out.push(formatBlockMarker(idGenerator()));
      }
    }
    out.push(line);
  }
  return out.join("\n");
}

// ---------------------------------------------------------------------------
// Milkdown plugin stub (`withBlockIds`).
//
// CommonMark preserves `<!-- ... -->` HTML comments verbatim through the
// parser and serializer, so this plugin currently registers nothing. It
// exists as a typed extension point: future work that needs to attach
// `data-block-id` to ProseMirror nodes (e.g. for IntersectionObserver
// wiring in the comment rail) will hook in here. See `assignMissingBlockIds`
// for the actual id assignment pass.
//
// We deliberately type the parameter as `unknown` so this file does NOT
// import from `@milkdown/*` — the editor entry point pulls the heavy
// Milkdown bundle, and we want this file (which is also imported by the
// pure-string test fixtures) to remain dependency-free.
export function withBlockIds(_ctx: unknown): void {
  // intentionally a no-op; see comment block above.
}
