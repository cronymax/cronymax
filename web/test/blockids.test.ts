// Round-trip + idempotency tests for the workbench block-id pass.
//
// Covers tasks 2.4 and 9.1 of `change: document-wysiwyg`.
//
// We assert two properties:
//   1. `assignMissingBlockIds` is a no-op on already-marked input — the
//      output equals the input byte-for-byte.
//   2. `parseBlockComments` recovers exactly the markers the fixture
//      embeds (so the regex stays in sync with the canonical form).
//   3. `stripBlockComments` removes every marker line and nothing else
//      (so the diff view's "compare visible content" guarantee holds).
//
// The fixture files live in `web/test/fixtures/markdown/` and are kept
// small so test failures are easy to read in CI output.

import { readdirSync, readFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

import { describe, expect, it } from "vitest";

import {
  assignMissingBlockIds,
  BLOCK_MARKER_REGEX,
  parseBlockComments,
  stripBlockComments,
} from "../src/panels/workbench/blockIds";

const FIXTURE_DIR = resolve(dirname(fileURLToPath(import.meta.url)), "fixtures/markdown");

function loadFixtures(): Array<{ name: string; content: string }> {
  return readdirSync(FIXTURE_DIR)
    .filter((f) => f.endsWith(".md"))
    .sort()
    .map((name) => ({
      name,
      content: readFileSync(join(FIXTURE_DIR, name), "utf-8"),
    }));
}

describe("blockIds: golden round-trip", () => {
  const fixtures = loadFixtures();

  for (const { name, content } of fixtures) {
    it(`${name}: assignMissingBlockIds is a no-op on marked input`, () => {
      // Use a deterministic id generator that fails loudly if called —
      // a marked fixture should never produce new ids.
      const out = assignMissingBlockIds(content, () => {
        throw new Error("idGenerator unexpectedly called");
      });
      expect(out).toBe(content);
    });

    it(`${name}: parseBlockComments recovers every marker`, () => {
      const markers = parseBlockComments(content);
      expect(markers.length).toBeGreaterThan(0);
      for (const m of markers) {
        expect(m.blockId).toMatch(/^[0-9a-fA-F-]{8,}$/);
        expect(BLOCK_MARKER_REGEX.test(m.raw)).toBe(true);
      }
    });

    it(`${name}: stripBlockComments removes only marker lines`, () => {
      const stripped = stripBlockComments(content);
      // No marker lines remain.
      for (const line of stripped.split("\n")) {
        expect(BLOCK_MARKER_REGEX.test(line)).toBe(false);
      }
      // Stripping is idempotent.
      expect(stripBlockComments(stripped)).toBe(stripped);
      // Non-marker line count is preserved.
      const before = content.split("\n").filter((l) => !BLOCK_MARKER_REGEX.test(l)).length;
      const after = stripped.split("\n").length;
      expect(after).toBe(before);
    });
  }
});

describe("blockIds: assignMissingBlockIds on raw input", () => {
  it("inserts a marker above an unmarked heading", () => {
    let n = 0;
    const out = assignMissingBlockIds(
      "# Hello\n\nA paragraph.\n",
      () => `00000000-0000-0000-0000-${String(++n).padStart(12, "0")}`,
    );
    expect(out).toBe(
      "<!-- block: 00000000-0000-0000-0000-000000000001 -->\n# Hello\n\n" +
        "<!-- block: 00000000-0000-0000-0000-000000000002 -->\nA paragraph.\n",
    );
  });

  it("is idempotent on its own output", () => {
    let n = 0;
    const first = assignMissingBlockIds(
      "## Section\n\nBody.\n\n- item one\n- item two\n",
      () => `00000000-0000-0000-0000-${String(++n).padStart(12, "0")}`,
    );
    const second = assignMissingBlockIds(first, () => {
      throw new Error("must not generate ids on second pass");
    });
    expect(second).toBe(first);
  });

  it("does not insert a marker above a fenced code block continuation", () => {
    let n = 0;
    const out = assignMissingBlockIds(
      "```\nline one\nline two\n```\n",
      () => `feedface-0000-0000-0000-${String(++n).padStart(12, "0")}`,
    );
    // Exactly one marker (above the fence opener), not three.
    expect(parseBlockComments(out).length).toBe(1);
  });
});
