// SPDX-License-Identifier: Apache-2.0
//
// Cronymax Extension Platform · IDL v1 · window
//
// User-visible UI primitives (toasts, prompts, webview panels). Routed
// through Rust core → CEF renderer; payloads MessagePack-RPC.

import type { CreateOutputChannelOptions, LogOutputChannel, OutputChannel } from "./logging";
import type { Disposable, Event, URI } from "./primitives";

export interface MessageItem {
  title: string;
  /** Marks the item as cancel-like (rendered with affordance). */
  isCloseAffordance?: boolean;
}

export interface MessageOptions {
  /**
   * Make the message modal (blocks the user until they pick an item).
   * Default false → toast.
   */
  modal?: boolean;
  /** Optional detail text shown under the headline. */
  detail?: string;
}

export interface InputBoxOptions {
  title?: string;
  prompt?: string;
  placeHolder?: string;
  value?: string;
  password?: boolean;
  /** Return non-empty string to reject; undefined to accept. */
  validateInput?(value: string): string | undefined | Promise<string | undefined>;
}

export interface QuickPickItem {
  label: string;
  description?: string;
  detail?: string;
  picked?: boolean;
}

export interface QuickPickOptions {
  title?: string;
  placeHolder?: string;
  canPickMany?: boolean;
}

// ─── webview ───────────────────────────────────────────────────────────────

export type WebviewSlot = "sidebar" | "settings" | "tab";

export interface WebviewPanelOptions {
  /** Slot to host the webview in. */
  slot: WebviewSlot;
  /** Stable id used to find/replace existing panels. */
  id: string;
  title: string;
  /** Path relative to extension root for the entry HTML. */
  entry: string;
  /** When focus moves elsewhere, retain renderer context? Default false. */
  retainContextWhenHidden?: boolean;
}

export interface WebviewPanel extends Disposable {
  readonly id: string;
  readonly slot: WebviewSlot;
  readonly active: boolean;
  readonly visible: boolean;

  /** Send a message into the iframe. Resolves once the renderer ACKs receipt. */
  postMessage(payload: unknown): Promise<void>;

  /** Replace the panel's body. Returns once renderer reloaded. */
  setHtml(html: string): Promise<void>;

  readonly onDidReceiveMessage: Event<unknown>;
  readonly onDidChangeViewState: Event<{ active: boolean; visible: boolean }>;
  readonly onDidDispose: Event<void>;
}

// ─── window namespace ──────────────────────────────────────────────────────

export interface Window {
  showInformationMessage(
    message: string,
    options?: MessageOptions,
    ...items: MessageItem[]
  ): Promise<MessageItem | undefined>;
  showWarningMessage(
    message: string,
    options?: MessageOptions,
    ...items: MessageItem[]
  ): Promise<MessageItem | undefined>;
  showErrorMessage(
    message: string,
    options?: MessageOptions,
    ...items: MessageItem[]
  ): Promise<MessageItem | undefined>;

  showInputBox(options?: InputBoxOptions): Promise<string | undefined>;
  showQuickPick(
    items: QuickPickItem[],
    options?: QuickPickOptions,
  ): Promise<QuickPickItem | QuickPickItem[] | undefined>;

  createWebviewPanel(options: WebviewPanelOptions): Promise<WebviewPanel>;

  /** Open a config page contributed via `cronymax.config.page`. */
  openConfigPage(pageId: string): Promise<void>;

  /** Reveal an external URI in the system browser. */
  openExternal(uri: URI): Promise<boolean>;

  /**
   * Create a named log channel surfaced in the settings panel's "Logs" tab.
   * `name` may contain "/" to create a logical hierarchy (e.g. "Coco / ACP")
   * — the UI will render this as a single flat dropdown entry; consumers
   * looking for sibling channels match by prefix.
   *
   * Returned channel persists to
   * `~/.cronymax/logs/<session>/extensions/<ext>/channels/<slug>.log` where
   * `<slug>` is `name` slugified (`Coco / ACP` → `coco-acp`).
   *
   * Calling with `{ log: true }` returns a `LogOutputChannel` with
   * `trace/debug/info/warn/error` methods and NDJSON formatting; the
   * level-less overload returns a plain `OutputChannel`.
   */
  createOutputChannel(name: string): OutputChannel;
  createOutputChannel(name: string, options: CreateOutputChannelOptions & { log: true }): LogOutputChannel;
}
