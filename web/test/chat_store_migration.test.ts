/**
 * Unit tests for the chat store localStorage migration:
 *   - v4-only: data is loaded and rehydrated directly
 *   - v3→v4 migration: contentStream reconstructed, written to v4 key, v3 key deleted
 *   - v2→v4 migration: traceContent stripped, traceEntries injected, v2 key deleted
 *   - both absent: returns empty data
 *   - persistChatData writes to v4 key with stripped results
 */
import { beforeEach, describe, expect, it } from "vitest";
import type { ContentSegment, ConversationBlock, PersistedChatData } from "../src/panels/chat/store";
import { loadChatData, persistChatData } from "../src/panels/chat/store";

// Provide a minimal localStorage mock
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
Object.defineProperty(globalThis, "localStorage", {
  value: localStorageMock,
  writable: true,
});

// Provide crypto.randomUUID stub for v1 migration path
if (!globalThis.crypto) {
  // @ts-expect-error minimal stub
  globalThis.crypto = {
    randomUUID: () => "00000000-0000-0000-0000-000000000000",
  };
}

const V2_KEY = (id: string) => `chat_history_v2:${id}`;
const V3_KEY = (id: string) => `chat_history_v3:${id}`;
const V4_KEY = (id: string) => `chat_history_v4:${id}`;

const makeConvBlock = (overrides: Partial<ConversationBlock> = {}): ConversationBlock => ({
  kind: "conversation",
  id: "block-1",
  userContent: "hello",
  attachments: [],
  contentStream: [],
  assistantContent: "world",
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

describe("loadChatData – v4 only", () => {
  beforeEach(() => localStorageMock.clear());

  it("returns data from v4 key", () => {
    const data: PersistedChatData = {
      blocks: [makeConvBlock({ contentStream: [{ kind: "text", content: "world" }] })],
      terminalTid: null,
      model: "gpt-4o",
    };
    localStorageMock.setItem(V4_KEY("chat-1"), JSON.stringify(data));
    const { data: result, migrationNotice } = loadChatData("chat-1");
    expect(result.model).toBe("gpt-4o");
    expect(result.blocks).toHaveLength(1);
    expect(migrationNotice).toBeUndefined();
  });

  it("does not fall through to v3 when v4 is present", () => {
    const v4Data: PersistedChatData = { blocks: [], terminalTid: null, model: "v4-model" };
    const v3Data: PersistedChatData = { blocks: [], terminalTid: null, model: "v3-model" };
    localStorageMock.setItem(V4_KEY("chat-2"), JSON.stringify(v4Data));
    localStorageMock.setItem(V3_KEY("chat-2"), JSON.stringify(v3Data));
    const { data } = loadChatData("chat-2");
    expect(data.model).toBe("v4-model");
  });
});

describe("loadChatData – v3 → v4 migration", () => {
  beforeEach(() => localStorageMock.clear());

  it("reconstructs contentStream from traceEntries (with assistant_turn and tool_start/done)", () => {
    const v3Block = makeConvBlock({
      contentStream: undefined as unknown as ContentSegment[],
      assistantContent: "hello",
      traceEntries: [
        { kind: "assistant_turn", turnId: 1, text: "hello", finishReason: "stop", ts: 100 },
        { kind: "tool_start", toolCallId: "tc1", tool: "search", args: { q: "x" }, ts: 200 },
        { kind: "tool_done", toolCallId: "tc1", tool: "search", result: ["r1"], terminal: false, ts: 300 },
      ],
    });
    const v3Data: PersistedChatData = { blocks: [v3Block], terminalTid: null, model: "m" };
    localStorageMock.setItem(V3_KEY("chat-m1"), JSON.stringify(v3Data));

    const { data } = loadChatData("chat-m1");
    const block = data.blocks[0] as ConversationBlock;
    expect(block.contentStream).toBeDefined();
    const textSeg = block.contentStream.find((s) => s.kind === "text");
    expect(textSeg).toMatchObject({ kind: "text", content: "hello" });
    const toolSeg = block.contentStream.find((s) => s.kind === "tool_call");
    expect(toolSeg).toMatchObject({ kind: "tool_call", tool: "search", status: "done" });
  });

  it("falls back to a single text segment when no traceEntries", () => {
    const v3Block = makeConvBlock({
      contentStream: undefined as unknown as ContentSegment[],
      assistantContent: "simple response",
      traceEntries: [],
    });
    const v3Data: PersistedChatData = { blocks: [v3Block], terminalTid: null, model: "" };
    localStorageMock.setItem(V3_KEY("chat-m2"), JSON.stringify(v3Data));

    const { data } = loadChatData("chat-m2");
    const block = data.blocks[0] as ConversationBlock;
    expect(block.contentStream).toHaveLength(1);
    expect(block.contentStream[0]).toMatchObject({ kind: "text", content: "simple response" });
  });

  it("writes migrated data to v4 key and removes v3 key", () => {
    const v3Data: PersistedChatData = { blocks: [], terminalTid: null, model: "m3" };
    localStorageMock.setItem(V3_KEY("chat-m3"), JSON.stringify(v3Data));
    loadChatData("chat-m3");
    expect(localStorageMock.getItem(V4_KEY("chat-m3"))).not.toBeNull();
    expect(localStorageMock.getItem(V3_KEY("chat-m3"))).toBeNull();
  });
});

describe("loadChatData – v2 → v4 migration", () => {
  beforeEach(() => localStorageMock.clear());

  it("strips traceContent and injects traceEntries: []", () => {
    const v2Block = {
      kind: "conversation",
      id: "b1",
      userContent: "hi",
      attachments: [],
      assistantContent: "resp",
      traceContent: "\n▶ running…\n✓ done",
      status: "ok",
      comments: [],
      createdAt: 0,
    };
    const v2Data = { blocks: [v2Block], terminalTid: null, model: "" };
    localStorageMock.setItem(V2_KEY("chat-3"), JSON.stringify(v2Data));

    const { data, migrationNotice } = loadChatData("chat-3");
    expect(data.blocks).toHaveLength(1);
    const block = data.blocks[0] as ConversationBlock;
    expect((block as unknown as Record<string, unknown>).traceContent).toBeUndefined();
    expect(block.traceEntries).toEqual([]);
    expect(migrationNotice).toBeUndefined();
  });

  it("writes migrated data to v4 key", () => {
    const v2Data = { blocks: [], terminalTid: null, model: "m1" };
    localStorageMock.setItem(V2_KEY("chat-4"), JSON.stringify(v2Data));
    loadChatData("chat-4");
    const v4Raw = localStorageMock.getItem(V4_KEY("chat-4"));
    expect(v4Raw).not.toBeNull();
    if (v4Raw === null) return;
    const parsed = JSON.parse(v4Raw) as PersistedChatData;
    expect(parsed.model).toBe("m1");
  });

  it("deletes v2 key after migration", () => {
    const v2Data = { blocks: [], terminalTid: null, model: "" };
    localStorageMock.setItem(V2_KEY("chat-5"), JSON.stringify(v2Data));
    loadChatData("chat-5");
    expect(localStorageMock.getItem(V2_KEY("chat-5"))).toBeNull();
  });
});

describe("loadChatData – both absent", () => {
  beforeEach(() => localStorageMock.clear());

  it("returns empty blocks with no migration notice", () => {
    const { data, migrationNotice } = loadChatData("chat-6");
    expect(data.blocks).toHaveLength(0);
    expect(data.terminalTid).toBeNull();
    expect(migrationNotice).toBeUndefined();
  });
});

describe("persistChatData", () => {
  beforeEach(() => localStorageMock.clear());

  it("writes to v4 key", () => {
    const data: PersistedChatData = { blocks: [], terminalTid: null, model: "saved" };
    persistChatData("chat-7", data);
    const v4Raw = localStorageMock.getItem(V4_KEY("chat-7"));
    expect(v4Raw).not.toBeNull();
    if (v4Raw === null) return;
    const parsed = JSON.parse(v4Raw) as PersistedChatData;
    expect(parsed.model).toBe("saved");
  });

  it("does not write to v3 or v2 key", () => {
    const data: PersistedChatData = { blocks: [], terminalTid: null, model: "" };
    persistChatData("chat-8", data);
    expect(localStorageMock.getItem(V3_KEY("chat-8"))).toBeNull();
    expect(localStorageMock.getItem(V2_KEY("chat-8"))).toBeNull();
  });

  it("strips rawBuf from shell blocks", () => {
    const shellBlock = {
      kind: "shell" as const,
      id: "s1",
      command: "ls",
      output: "file.txt",
      rawBuf: "some partial data",
      status: "ok" as const,
      exitCode: 0,
      startedAt: 0,
      endedAt: 100,
      comments: [],
    };
    const data: PersistedChatData = { blocks: [shellBlock], terminalTid: null, model: "" };
    persistChatData("chat-9", data);
    const raw = localStorageMock.getItem(V4_KEY("chat-9"));
    if (raw === null) return;
    const parsed = JSON.parse(raw) as PersistedChatData;
    expect((parsed.blocks[0] as Record<string, unknown>).rawBuf).toBe("");
  });

  it("strips tool_call results from contentStream", () => {
    const convBlock = makeConvBlock({
      contentStream: [
        { kind: "text", content: "response" },
        {
          kind: "tool_call",
          toolCallId: "tc1",
          tool: "search",
          args: { q: "test" },
          status: "done",
          result: { data: "big result" },
        },
      ],
    });
    const data: PersistedChatData = { blocks: [convBlock], terminalTid: null, model: "" };
    persistChatData("chat-10", data);
    const raw = localStorageMock.getItem(V4_KEY("chat-10"));
    if (raw === null) return;
    const parsed = JSON.parse(raw) as PersistedChatData;
    const block = parsed.blocks[0] as ConversationBlock;
    const toolSeg = block.contentStream.find((s) => s.kind === "tool_call") as
      | Extract<ContentSegment, { kind: "tool_call" }>
      | undefined;
    expect(toolSeg?.result).toBeNull();
  });
});
