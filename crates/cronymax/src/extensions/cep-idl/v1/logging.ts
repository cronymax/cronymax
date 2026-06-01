// SPDX-License-Identifier: Apache-2.0
//
// Cronymax Extension Platform · IDL v1 · logging
//
// Per-extension named log channels. Mirrors VS Code's `vscode.window.createOutputChannel`.
//
// Added in Phase 0 review (2026-05-20) as a v1-pre-freeze patch. See
// `docs/extensions/extension-logs.md` for the surrounding design.
//
// FROZEN ON V1 SHIP. Additions allowed; signature changes forbidden.

import type { Disposable } from "./primitives";

export type LogLevel = "trace" | "debug" | "info" | "warn" | "error";

/** Plain (level-less) output channel. */
export interface OutputChannel extends Disposable {
  readonly name: string;
  /** Append a line. A `\n` is added automatically. */
  appendLine(value: string): void;
  /** Append raw text without a trailing newline. */
  append(value: string): void;
  /**
   * Reveal this channel in the UI (settings panel → extension → Logs tab).
   * In `cronymax ext dev` mode this is a no-op; the terminal is already
   * showing everything.
   */
  show(preserveFocus?: boolean): void;
  /** Erase in-memory buffer + on-disk file. */
  clear(): Promise<void>;
}

/**
 * Log-level output channel; the recommended shape for new code. Returned
 * when callers pass `{ log: true }` to `createOutputChannel`. Each method
 * formats `args` JSON-stringified and writes one NDJSON line to the channel
 * file at `~/.cronymax/logs/<session>/extensions/<ext>/channels/<slug>.log`.
 */
export interface LogOutputChannel extends OutputChannel {
  trace(message: string, ...args: unknown[]): void;
  debug(message: string, ...args: unknown[]): void;
  info(message: string, ...args: unknown[]): void;
  warn(message: string, ...args: unknown[]): void;
  /** Accepts either a string or an Error (stack is preserved). */
  error(message: string | Error, ...args: unknown[]): void;
}

export interface CreateOutputChannelOptions {
  /**
   * When true, returns a `LogOutputChannel` with level methods + JSON line
   * formatting. Default false returns a plain `OutputChannel`.
   */
  log?: boolean;
}
