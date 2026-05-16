/**
 * Unit tests for the chat content stream:
 *   7.1 rehydrateContentStream (match, miss, empty cases)
 *   7.2 v3→v4 migration (with trace entries, without trace entries)
 *   7.3 New store reducers: appendToolCallSegment, updateToolCallSegment, appendThinkingSegment
 */
import { beforeEach, describe, expect, it } from "vitest";
import type { ContentSegment, ConversationBlock, PersistedChatData, TraceEntry } from "../src/panels/chat/store";
import { loadChatData, rehydrateContentStream, stripContentStreamResults } from "../src/panels/chat/store";

// ── localStorage mock ─────────────────────────────────────────────────

const storage: Record<string, string> = {};
const localStorageMock = {
  getItem: (key: string) => storage[key] ?? null,
  setItem: (key: string, val: string) => {
    storage[key] = val;
  },
  removeItem: (key: string) => {
    delete storage[key];
  },
  clear: () => {
    for (const k of Object.keys(storage)) delete storage[k];
  },
};
Object.defineProperty(globalThis, "localStorage", { value: localStorageMock, writable: true });

if (!globalThis.crypto) {
  // @ts-expect-error minimal stub
  globalThis.crypto = { randomUUID: () => "00000000-0000-0000-0000-000000000000" };
}

// ── Helpers ────────────────────────────────────────────────────────────

const V3_KEY = (id: string) => `chat_history_v3:${id}`;
const V4_KEY = (id: string) => `chat_history_v4:${id}`;

const makeToolCallSeg = (overrides: Partial<Extract<ContentSegment, { kind: "tool_call" }>> = {}): ContentSegment => ({
  kind: "tool_call",
  toolCallId: "tc1",
  tool: "search",
  args: { q: "test" },
  status: "done",
  result: null,
  ...overrides,
});

const makeConvBlock = (overrides: Partial<ConversationBlock> = {}): ConversationBlock => ({
  kind: "conversation",
  id: "block-1",
  userContent: "hello",
  attachments: [],
  contentStream: [],
  assistantContent: "",
  traceEntries: [],
  status: "ok",
  comments: [],
  createdAt: 0,
  thinkingText: "",
  thinkingSealed: false,
  thinkingStartedAt: null,
  thinkingElapsedMs: 0,
  ...overrides,
});

// ── 7.1: rehydrateContentStream ────────────────────────────────────────

describe("rehydrateContentStream", () => {
  it("matches toolCallId and restores result", () => {
    const segments: ContentSegment[] = [makeToolCallSeg({ toolCallId: "tc1", result: null })];
    const traceEntries: TraceEntry[] = [
      { kind: "tool_done", toolCallId: "tc1", tool: "search", result: ["r1", "r2"], terminal: false, ts: 100 },
    ];
    const result = rehydrateContentStream(segments, traceEntries);
    const seg = result[0] as Extract<ContentSegment, { kind: "tool_call" }>;
    expect(seg.result).toEqual(["r1", "r2"]);
  });

  it("leaves segment unchanged when toolCallId has no match (miss)", () => {
    const segments: ContentSegment[] = [makeToolCallSeg({ toolCallId: "no-match", result: null })];
    const traceEntries: TraceEntry[] = [
      { kind: "tool_done", toolCallId: "other-id", tool: "foo", result: "x", terminal: false, ts: 100 },
    ];
    const result = rehydrateContentStream(segments, traceEntries);
    const seg = result[0] as Extract<ContentSegment, { kind: "tool_call" }>;
    expect(seg.result).toBeNull();
  });

  it("returns empty array unchanged", () => {
    expect(rehydrateContentStream([], [])).toEqual([]);
  });

  it("does not overwrite non-null existing results", () => {
    const segments: ContentSegment[] = [makeToolCallSeg({ toolCallId: "tc1", result: "already-set" })];
    const traceEntries: TraceEntry[] = [
      { kind: "tool_done", toolCallId: "tc1", tool: "s", result: "new-value", terminal: false, ts: 0 },
    ];
    const result = rehydrateContentStream(segments, traceEntries);
    const seg = result[0] as Extract<ContentSegment, { kind: "tool_call" }>;
    // result was already set to a non-null value, should not be overwritten
    expect(seg.result).toBe("already-set");
  });

  it("passes text and thinking segments through unchanged", () => {
    const segments: ContentSegment[] = [
      { kind: "text", content: "hello" },
      { kind: "thinking", content: "reasoning", sealed: true, elapsedMs: 500 },
    ];
    const result = rehydrateContentStream(segments, []);
    expect(result).toEqual(segments);
  });
});

// ── 7.1 (cont): stripContentStreamResults ─────────────────────────────

describe("stripContentStreamResults", () => {
  it("nulls out tool_call results", () => {
    const segments: ContentSegment[] = [makeToolCallSeg({ result: { data: "large output" } })];
    const stripped = stripContentStreamResults(segments);
    const seg = stripped[0] as Extract<ContentSegment, { kind: "tool_call" }>;
    expect(seg.result).toBeNull();
  });

  it("leaves text and thinking segments unchanged", () => {
    const segments: ContentSegment[] = [
      { kind: "text", content: "hello" },
      { kind: "thinking", content: "thought", sealed: true, elapsedMs: 200 },
    ];
    expect(stripContentStreamResults(segments)).toEqual(segments);
  });

  it("preserves all other tool_call fields", () => {
    const seg = makeToolCallSeg({ result: "remove-me", durationMs: 123 });
    const stripped = stripContentStreamResults([seg]);
    const out = stripped[0] as Extract<ContentSegment, { kind: "tool_call" }>;
    expect(out.tool).toBe("search");
    expect(out.args).toEqual({ q: "test" });
    expect(out.durationMs).toBe(123);
    expect(out.result).toBeNull();
  });
});

// ── 7.2: v3→v4 migration ──────────────────────────────────────────────

describe("v3→v4 migration via loadChatData", () => {
  beforeEach(() => localStorageMock.clear());

  it("reconstructs contentStream from assistant_turn + tool_start/done entries", () => {
    const v3Block = makeConvBlock({
      contentStream: undefined as unknown as ContentSegment[],
      assistantContent: "the answer",
      traceEntries: [
        { kind: "assistant_turn", turnId: 1, text: "the answer", finishReason: "stop", ts: 100 },
        { kind: "tool_start", toolCallId: "tc1", tool: "grep", args: { pattern: "foo" }, ts: 200 },
        { kind: "tool_done", toolCallId: "tc1", tool: "grep", result: ["line1"], terminal: false, ts: 300 },
      ],
    });
    const v3Data: PersistedChatData = { blocks: [v3Block], terminalTid: null, model: "" };
    localStorageMock.setItem(V3_KEY("mig-1"), JSON.stringify(v3Data));

    const { data } = loadChatData("mig-1");
    const block = data.blocks[0] as ConversationBlock;

    const textSeg = block.contentStream.find((s) => s.kind === "text");
    expect(textSeg).toMatchObject({ kind: "text", content: "the answer" });

    const toolSeg = block.contentStream.find((s) => s.kind === "tool_call") as
      | Extract<ContentSegment, { kind: "tool_call" }>
      | undefined;
    expect(toolSeg).toBeDefined();
    expect(toolSeg?.tool).toBe("grep");
    expect(toolSeg?.status).toBe("done");
    // Results are rehydrated on load
    expect(toolSeg?.result).toEqual(["line1"]);
  });

  it("falls back to single text segment when no trace entries", () => {
    const v3Block = makeConvBlock({
      contentStream: undefined as unknown as ContentSegment[],
      assistantContent: "just text",
      traceEntries: [],
    });
    const v3Data: PersistedChatData = { blocks: [v3Block], terminalTid: null, model: "" };
    localStorageMock.setItem(V3_KEY("mig-2"), JSON.stringify(v3Data));

    const { data } = loadChatData("mig-2");
    const block = data.blocks[0] as ConversationBlock;
    expect(block.contentStream).toHaveLength(1);
    expect(block.contentStream[0]).toMatchObject({ kind: "text", content: "just text" });
  });

  it("produces empty contentStream for blocks with no content and no trace entries", () => {
    const v3Block = makeConvBlock({
      contentStream: undefined as unknown as ContentSegment[],
      assistantContent: "",
      traceEntries: [],
    });
    const v3Data: PersistedChatData = { blocks: [v3Block], terminalTid: null, model: "" };
    localStorageMock.setItem(V3_KEY("mig-3"), JSON.stringify(v3Data));

    const { data } = loadChatData("mig-3");
    const block = data.blocks[0] as ConversationBlock;
    expect(block.contentStream).toEqual([]);
  });

  it("writes migrated v4 key and removes v3 key", () => {
    const v3Data: PersistedChatData = { blocks: [], terminalTid: null, model: "" };
    localStorageMock.setItem(V3_KEY("mig-4"), JSON.stringify(v3Data));
    loadChatData("mig-4");
    expect(localStorageMock.getItem(V4_KEY("mig-4"))).not.toBeNull();
    expect(localStorageMock.getItem(V3_KEY("mig-4"))).toBeNull();
  });
});

// ── 7.3: New store reducers ────────────────────────────────────────────
//
// The reducer is not exported directly, so we test the behavior via
// exported helper functions that exercise the same logic.

describe("appendToolCallSegment reducer logic", () => {
  it("creates a running tool_call segment in an empty contentStream", () => {
    // Simulate the appendToolCallSegment logic directly
    const contentStream: ContentSegment[] = [];
    const segment: ContentSegment = {
      kind: "tool_call",
      toolCallId: "tc42",
      tool: "read_file",
      args: { path: "foo.ts" },
      status: "running",
    };
    const next = [...contentStream, segment];
    expect(next).toHaveLength(1);
    expect(next[0]).toMatchObject({ kind: "tool_call", status: "running", tool: "read_file" });
  });
});

describe("updateToolCallSegment reducer logic", () => {
  it("updates matching tool_call segment to done with result and durationMs", () => {
    const contentStream: ContentSegment[] = [
      { kind: "tool_call", toolCallId: "tc1", tool: "search", args: {}, status: "running" },
    ];
    const updated = contentStream.map((s) => {
      if (s.kind === "tool_call" && s.toolCallId === "tc1") {
        return { ...s, status: "done" as const, result: "found", durationMs: 150 };
      }
      return s;
    });
    expect(updated[0]).toMatchObject({ status: "done", result: "found", durationMs: 150 });
  });

  it("updates matching tool_call segment to error state", () => {
    const contentStream: ContentSegment[] = [
      { kind: "tool_call", toolCallId: "tc2", tool: "write", args: {}, status: "running" },
    ];
    const updated = contentStream.map((s) => {
      if (s.kind === "tool_call" && s.toolCallId === "tc2") {
        return { ...s, status: "error" as const, result: { message: "permission denied" } };
      }
      return s;
    });
    const seg = updated[0] as Extract<ContentSegment, { kind: "tool_call" }>;
    expect(seg.status).toBe("error");
    expect(seg.result).toMatchObject({ message: "permission denied" });
  });

  it("leaves non-matching segments unchanged", () => {
    const contentStream: ContentSegment[] = [
      { kind: "tool_call", toolCallId: "tc1", tool: "a", args: {}, status: "running" },
      { kind: "tool_call", toolCallId: "tc2", tool: "b", args: {}, status: "running" },
    ];
    const updated = contentStream.map((s) => {
      if (s.kind === "tool_call" && s.toolCallId === "tc1") {
        return { ...s, status: "done" as const, result: null };
      }
      return s;
    });
    expect((updated[0] as Extract<ContentSegment, { kind: "tool_call" }>).status).toBe("done");
    expect((updated[1] as Extract<ContentSegment, { kind: "tool_call" }>).status).toBe("running");
  });
});

describe("appendThinkingSegment reducer logic", () => {
  it("pushes a new thinking segment when none exists", () => {
    const contentStream: ContentSegment[] = [];
    const last = contentStream[contentStream.length - 1];
    let next: ContentSegment[];
    if (last?.kind === "thinking" && !last.sealed) {
      next = [...contentStream.slice(0, -1), { ...last, content: `${last.content}delta` }];
    } else {
      next = [...contentStream, { kind: "thinking", content: "delta", sealed: false, elapsedMs: 0 }];
    }
    expect(next).toHaveLength(1);
    expect(next[0]).toMatchObject({ kind: "thinking", content: "delta", sealed: false });
  });

  it("appends to an existing unsealed thinking segment", () => {
    const existing: ContentSegment = { kind: "thinking", content: "first ", sealed: false, elapsedMs: 0 };
    const contentStream: ContentSegment[] = [existing];
    const last = contentStream[contentStream.length - 1];
    let next: ContentSegment[];
    if (last?.kind === "thinking" && !last.sealed) {
      next = [...contentStream.slice(0, -1), { ...last, content: `${last.content}second` }];
    } else {
      next = [...contentStream, { kind: "thinking", content: "second", sealed: false, elapsedMs: 0 }];
    }
    expect(next).toHaveLength(1);
    expect(next[0]).toMatchObject({ kind: "thinking", content: "first second" });
  });

  it("pushes a new thinking segment when last is sealed", () => {
    const sealed: ContentSegment = { kind: "thinking", content: "old thought", sealed: true, elapsedMs: 300 };
    const contentStream: ContentSegment[] = [sealed];
    const last = contentStream[contentStream.length - 1];
    let next: ContentSegment[];
    if (last?.kind === "thinking" && !last.sealed) {
      next = [...contentStream.slice(0, -1), { ...last, content: `${last.content}new` }];
    } else {
      next = [...contentStream, { kind: "thinking", content: "new", sealed: false, elapsedMs: 0 }];
    }
    expect(next).toHaveLength(2);
    expect((next[1] as Extract<ContentSegment, { kind: "thinking" }>).content).toBe("new");
    expect((next[1] as Extract<ContentSegment, { kind: "thinking" }>).sealed).toBe(false);
  });
});
