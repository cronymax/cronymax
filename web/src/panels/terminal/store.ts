/**
 * Terminal panel store.
 *
 * One pane per terminal id (`tid`). Output is fed through a pure reducer
 * that:
 *   1. Buffers raw bytes per pane.
 *   2. Splits on OSC 133 sequences (`\x1b]133;<type>(;args)?\x07`).
 *   3. Routes inter-sequence text to either the active block (after ANSI
 *      strip) or the pre-block raw output region.
 *   4. Opens/closes blocks on type "C" / "D" markers.
 *
 * Side effects (`bridge.send('terminal.block_save', …)` and
 * `bridge.send('agent.task_from_command', …)`) live in `App.tsx` — the
 * reducer marks blocks as `pendingSave` and the App reacts by persisting
 * + dispatching `markSaved`.
 */
import { createPanelStore } from "@/hooks/usePanelStore";

// ── helpers ────────────────────────────────────────────────────────────

// Strip ANSI / control sequences for display & storage. Mirrors the
// legacy stripAnsi() behavior so saved blocks read cleanly.
export function stripAnsi(str: string): string {
  return (
    str
      // CSI: ESC [ ... letter
      .replace(/\x1b\[[\x30-\x3f]*[\x20-\x2f]*[\x40-\x7e]/g, "")
      // OSC: ESC ] ... BEL or ST
      .replace(/\x1b\][\s\S]*?(?:\x07|\x1b\\)/g, "")
      // DCS / PM / APC
      .replace(/\x1b[PX^_][\s\S]*?(?:\x07|\x1b\\)/g, "")
      // Charset designators
      .replace(/\x1b[()][\x20-\x7e]/g, "")
      // Single-char escapes
      .replace(/\x1b[=>78MEDH]/g, "")
      // Lone ESC
      .replace(/\x1b/g, "")
      // CR without LF → LF
      .replace(/\r(?!\n)/g, "\n")
      // BEL
      .replace(/\x07/g, "")
  );
}

// ── types ──────────────────────────────────────────────────────────────

export type BlockStatus = "running" | "ok" | "fail";

export interface Block {
  id: number;
  command: string;
  output: string;
  status: BlockStatus;
  startedAt: number;
  endedAt: number | null;
  exitCode: number | null;
  /** True once the reducer has emitted a transition that requires the
   *  App to persist via `terminal.block_save`. Cleared by `markSaved`. */
  pendingSave: boolean;
}

export interface PaneState {
  blocks: Block[];
  /** Raw output that arrived before any block was opened. */
  rawOutput: string;
  /** Buffer for partial OSC sequences across chunks. */
  rawBuf: string;
  /** Most recent user-submitted command awaiting a "C" prompt marker. */
  pendingCommand: string;
  /** True once `terminal.start` has been sent for this tid. */
  started: boolean;
  /** True once persisted blocks have been loaded for this tid. */
  restored: boolean;
  /** Sticky "session restored" notice text, or null. */
  restoredNotice: string | null;
}

export interface State {
  panes: Record<string, PaneState>;
  activeTid: string | null;
  /** Monotonic id generator for blocks (unique across panes). */
  blockSeq: number;
}

export type Action =
  | { type: "ensurePane"; tid: string }
  | { type: "setActive"; tid: string }
  | { type: "removePane"; tid: string }
  | { type: "markStarted"; tid: string }
  | { type: "markRestored"; tid: string; count: number }
  | { type: "submit"; tid: string; command: string; now: number }
  | { type: "output"; tid: string; chunk: string; now: number }
  | { type: "exit"; tid: string; code: number; now: number }
  | { type: "restartClear"; tid: string }
  | { type: "markSaved"; tid: string; blockId: number };

// ── factories ──────────────────────────────────────────────────────────

function blankPane(): PaneState {
  return {
    blocks: [],
    rawOutput: "",
    rawBuf: "",
    pendingCommand: "",
    started: false,
    restored: false,
    restoredNotice: null,
  };
}

const initial: State = {
  panes: {},
  activeTid: null,
  blockSeq: 1,
};

// ── OSC 133 parser ─────────────────────────────────────────────────────

interface ParseResult {
  pane: PaneState;
  nextSeq: number;
}

/**
 * Apply an output chunk to a pane. Pure: returns a new pane object plus
 * the next block-seq counter to consume.
 */
function applyOutput(
  pane: PaneState,
  chunk: string,
  now: number,
  blockSeq: number,
): ParseResult {
  let buf = pane.rawBuf + chunk;
  let blocks = pane.blocks;
  let rawOutput = pane.rawOutput;
  let pendingCommand = pane.pendingCommand;
  let seq = blockSeq;

  // Helper: append text segment to the active block (last running block) or
  // pre-block raw region.
  const flush = (text: string) => {
    if (!text) return;
    const clean = stripAnsi(text);
    if (!clean) return;
    const lastIdx = lastRunningBlockIndex(blocks);
    if (lastIdx >= 0) {
      const blk = blocks[lastIdx]!;
      const next = { ...blk, output: blk.output + clean };
      blocks = blocks.slice();
      blocks[lastIdx] = next;
    } else {
      rawOutput = rawOutput + clean;
    }
  };

  // Loop OSC 133 sequences. Format: \x1b]133;<type>(;arg)?\x07
  for (;;) {
    const oscStart = buf.indexOf("\x1b]133;");
    if (oscStart === -1) {
      flush(buf);
      buf = "";
      break;
    }
    if (oscStart > 0) {
      flush(buf.slice(0, oscStart));
      buf = buf.slice(oscStart);
    }
    const bel = buf.indexOf("\x07");
    if (bel === -1) break; // partial — keep in buf
    const inner = buf.slice(6, bel); // strip ESC]133;  and  BEL
    buf = buf.slice(bel + 1);
    const parts = inner.split(";");
    const type = parts[0];
    if (type === "C") {
      // Command output begins. If the user already opened a block via
      // submit(), keep it. Otherwise create one (shell-initiated commands).
      const lastRun = lastRunningBlockIndex(blocks);
      if (lastRun < 0) {
        const newBlock: Block = {
          id: seq,
          command: pendingCommand,
          output: "",
          status: "running",
          startedAt: now,
          endedAt: null,
          exitCode: null,
          pendingSave: false,
        };
        seq += 1;
        blocks = [...blocks, newBlock];
      }
      pendingCommand = "";
    } else if (type === "D") {
      const ec = parts[1] !== undefined ? parseInt(parts[1], 10) : 0;
      const code = Number.isNaN(ec) ? 0 : ec;
      const lastRun = lastRunningBlockIndex(blocks);
      if (lastRun >= 0) {
        const blk = blocks[lastRun]!;
        blocks = blocks.slice();
        blocks[lastRun] = {
          ...blk,
          status: code === 0 ? "ok" : "fail",
          exitCode: code,
          endedAt: now,
          pendingSave: true,
        };
      }
    }
    // unknown types are dropped
  }

  return {
    pane: {
      ...pane,
      blocks,
      rawOutput,
      rawBuf: buf,
      pendingCommand,
    },
    nextSeq: seq,
  };
}

function lastRunningBlockIndex(blocks: Block[]): number {
  for (let i = blocks.length - 1; i >= 0; i -= 1) {
    if (blocks[i]!.status === "running") return i;
  }
  return -1;
}

// ── reducer ────────────────────────────────────────────────────────────

function reducer(state: State, action: Action): State {
  switch (action.type) {
    case "ensurePane": {
      if (state.panes[action.tid]) return state;
      return {
        ...state,
        panes: { ...state.panes, [action.tid]: blankPane() },
        activeTid: state.activeTid ?? action.tid,
      };
    }
    case "setActive": {
      const next = state.panes[action.tid]
        ? state.panes
        : {
            ...state.panes,
            [action.tid]: blankPane(),
          };
      return { ...state, panes: next, activeTid: action.tid };
    }
    case "removePane": {
      if (!state.panes[action.tid]) return state;
      const { [action.tid]: _, ...rest } = state.panes;
      const nextActive =
        state.activeTid === action.tid
          ? (Object.keys(rest)[0] ?? null)
          : state.activeTid;
      return { ...state, panes: rest, activeTid: nextActive };
    }
    case "markStarted": {
      const p = state.panes[action.tid];
      if (!p || p.started) return state;
      return {
        ...state,
        panes: { ...state.panes, [action.tid]: { ...p, started: true } },
      };
    }
    case "markRestored": {
      const p = state.panes[action.tid];
      if (!p) return state;
      return {
        ...state,
        panes: {
          ...state.panes,
          [action.tid]: {
            ...p,
            restored: true,
            restoredNotice:
              action.count > 0
                ? `— restored ${action.count} block(s) from previous session —`
                : null,
          },
        },
      };
    }
    case "submit": {
      const p = state.panes[action.tid] ?? blankPane();
      // Close any straggler "running" block (no D marker observed) so the
      // new command starts a fresh block. Marks it pendingSave with last
      // known exit code (0 if none).
      let blocks = p.blocks;
      const lastRun = lastRunningBlockIndex(blocks);
      if (lastRun >= 0) {
        const blk = blocks[lastRun]!;
        blocks = blocks.slice();
        blocks[lastRun] = {
          ...blk,
          status: blk.exitCode === null || blk.exitCode === 0 ? "ok" : "fail",
          exitCode: blk.exitCode ?? 0,
          endedAt: action.now,
          pendingSave: true,
        };
      }
      const newBlock: Block = {
        id: state.blockSeq,
        command: action.command,
        output: "",
        status: "running",
        startedAt: action.now,
        endedAt: null,
        exitCode: null,
        pendingSave: false,
      };
      return {
        ...state,
        blockSeq: state.blockSeq + 1,
        panes: {
          ...state.panes,
          [action.tid]: {
            ...p,
            blocks: [...blocks, newBlock],
            pendingCommand: action.command,
          },
        },
      };
    }
    case "output": {
      const p = state.panes[action.tid] ?? blankPane();
      const { pane, nextSeq } = applyOutput(
        p,
        action.chunk,
        action.now,
        state.blockSeq,
      );
      return {
        ...state,
        blockSeq: nextSeq,
        panes: { ...state.panes, [action.tid]: pane },
      };
    }
    case "exit": {
      const p = state.panes[action.tid];
      if (!p) return state;
      let blocks = p.blocks;
      const lastRun = lastRunningBlockIndex(blocks);
      if (lastRun >= 0) {
        const blk = blocks[lastRun]!;
        blocks = blocks.slice();
        blocks[lastRun] = {
          ...blk,
          status: "fail",
          exitCode: action.code,
          endedAt: action.now,
          pendingSave: true,
        };
      }
      return {
        ...state,
        panes: {
          ...state.panes,
          [action.tid]: {
            ...p,
            blocks,
            rawOutput: p.rawOutput + `\n[terminal exited: ${action.code}]\n`,
            started: false,
          },
        },
      };
    }
    case "restartClear": {
      const p = state.panes[action.tid];
      if (!p) return state;
      return {
        ...state,
        panes: {
          ...state.panes,
          [action.tid]: blankPane(),
        },
      };
    }
    case "markSaved": {
      const p = state.panes[action.tid];
      if (!p) return state;
      const idx = p.blocks.findIndex((b) => b.id === action.blockId);
      if (idx < 0) return state;
      const blocks = p.blocks.slice();
      blocks[idx] = { ...blocks[idx]!, pendingSave: false };
      return {
        ...state,
        panes: { ...state.panes, [action.tid]: { ...p, blocks } },
      };
    }
    default:
      return state;
  }
}

export const { Provider, useStore, useState, useDispatch } = createPanelStore<
  State,
  Action
>(reducer, initial);
