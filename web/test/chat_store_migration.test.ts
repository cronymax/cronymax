/**
 * Unit tests for the chat store localStorage migration:
 *   - v3-only: data is loaded directly
 *   - v2→v3 migration: traceContent is stripped, traceEntries injected, v2 key deleted
 *   - both absent: returns empty data
 *   - persistChatData writes to v3 key
 */
import { beforeEach, describe, expect, it } from "vitest";
import type { ConversationBlock, PersistedChatData } from "../src/panels/chat/store";
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

const makeConvBlock = (overrides: Partial<ConversationBlock> = {}): ConversationBlock => ({
  kind: "conversation",
  id: "block-1",
  userContent: "hello",
  attachments: [],
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

describe("loadChatData – v3 only", () => {
  beforeEach(() => localStorageMock.clear());

  it("returns data from v3 key", () => {
    const data: PersistedChatData = {
      blocks: [makeConvBlock()],
      terminalTid: null,
      model: "gpt-4o",
    };
    localStorageMock.setItem(V3_KEY("chat-1"), JSON.stringify(data));
    const { data: result, migrationNotice } = loadChatData("chat-1");
    expect(result.model).toBe("gpt-4o");
    expect(result.blocks).toHaveLength(1);
    expect(migrationNotice).toBeUndefined();
  });

  it("does not fall through to v2 when v3 is present", () => {
    const v3Data: PersistedChatData = {
      blocks: [],
      terminalTid: null,
      model: "v3-model",
    };
    const v2Data = { blocks: [], terminalTid: null, model: "v2-model" };
    localStorageMock.setItem(V3_KEY("chat-2"), JSON.stringify(v3Data));
    localStorageMock.setItem(V2_KEY("chat-2"), JSON.stringify(v2Data));
    const { data } = loadChatData("chat-2");
    expect(data.model).toBe("v3-model");
  });
});

describe("loadChatData – v2 → v3 migration", () => {
  beforeEach(() => localStorageMock.clear());

  it("strips traceContent and injects traceEntries: []", () => {
    // Simulate old v2 block with traceContent
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

  it("writes migrated data to v3 key", () => {
    const v2Data = { blocks: [], terminalTid: null, model: "m1" };
    localStorageMock.setItem(V2_KEY("chat-4"), JSON.stringify(v2Data));
    loadChatData("chat-4");
    const v3Raw = localStorageMock.getItem(V3_KEY("chat-4"));
    expect(v3Raw).not.toBeNull();
    if (v3Raw === null) return;
    const parsed = JSON.parse(v3Raw) as PersistedChatData;
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

  it("writes to v3 key", () => {
    const data: PersistedChatData = {
      blocks: [],
      terminalTid: null,
      model: "saved",
    };
    persistChatData("chat-7", data);
    expect(localStorageMock.getItem(V3_KEY("chat-7"))).not.toBeNull();
    const v3Raw7 = localStorageMock.getItem(V3_KEY("chat-7"));
    if (v3Raw7 === null) return;
    const parsed = JSON.parse(v3Raw7) as PersistedChatData;
    expect(parsed.model).toBe("saved");
  });

  it("does not write to v2 key", () => {
    const data: PersistedChatData = {
      blocks: [],
      terminalTid: null,
      model: "",
    };
    persistChatData("chat-8", data);
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
    const data: PersistedChatData = {
      blocks: [shellBlock],
      terminalTid: null,
      model: "",
    };
    persistChatData("chat-9", data);
    const raw = localStorageMock.getItem(V3_KEY("chat-9"));
    if (raw === null) return;
    const parsed = JSON.parse(raw) as PersistedChatData;
    expect((parsed.blocks[0] as Record<string, unknown>).rawBuf).toBe("");
  });
});
