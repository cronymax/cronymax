// SPDX-License-Identifier: Apache-2.0
//
// Cronymax Extension Platform · IDL v1 · agents
//
// THE central contract for L2 EP `cronymax.agents.provider`.
//
// FROZEN. Any change here is a v1 → v2 break. The `AgentEvent` discriminated
// union below is mirrored exactly in the Rust runtime and over the wire.

import type { ContributionItem } from "./contributions";
import type { CancellationToken, Disposable } from "./primitives";

// ─── descriptive metadata ──────────────────────────────────────────────────

/**
 * Convenience alias used by `SessionOptions.model` so existing call sites
 * keep reading naturally. The picker actually exchanges full
 * `ContributionItem` objects (label, description, …); the id of the
 * selected item is what's passed to `createSession({ model: itemId })`.
 */
export type ModelInfo = ContributionItem;

// ─── session inputs ────────────────────────────────────────────────────────

export interface McpServerSpec {
  /** Stable identifier within the session (for routing tool calls). */
  name: string;
  /**
   * "stdio" launches a subprocess; "http" connects to a URL; "sse"
   * subscribes to a streaming endpoint. Provider may ignore unsupported
   * transports.
   */
  transport: "stdio" | "http" | "sse";
  /** stdio: command path. http/sse: base URL. */
  endpoint: string;
  /** stdio: argv. */
  args?: readonly string[];
  /** Environment overrides applied on stdio spawn. */
  env?: Readonly<Record<string, string>>;
  /** http/sse: extra headers (e.g. Authorization). */
  headers?: Readonly<Record<string, string>>;
}

export interface SessionOptions {
  /** Working directory for tools / MCP servers that need one. */
  cwd: string;
  /**
   * `ContributionItem.id` of the user-picked item — see `AgentProvider.enumerate`.
   * Provider semantics are kind-dependent: for a "model" provider this is the
   * model id; for a multi-personality agent this is the personality id; etc.
   */
  model?: string;
  /** MCP servers to attach to the session at start. */
  mcpServers?: readonly McpServerSpec[];
  /** Override the system prompt (provider may merge with its default). */
  systemPrompt?: string;
  /**
   * Whitelist of platform tool ids the session is allowed to invoke.
   * The platform enforces this; providers see only the filtered set.
   */
  allowedTools?: readonly string[];
}

export interface PromptMessage {
  /** Plain-text user turn. */
  text: string;
  /**
   * Attachments — files / images / data blobs. Providers MAY support a
   * subset; non-fatal silent drop is permitted in v1.
   */
  attachments?: readonly PromptAttachment[];
}

export interface PromptAttachment {
  kind: "file" | "image" | "blob";
  /** For "file"/"image", a URI; for "blob", a data URL. */
  uri?: string;
  data?: Uint8Array;
  mimeType?: string;
}

// ─── streamed events (the wire format) ─────────────────────────────────────

/**
 * Discriminated union of everything an `AgentSession.prompt(...)` async
 * iterator can yield. MUST match the Rust `AgentEvent` enum 1:1.
 *
 * Any new variant added to v1 MUST have a fallback rendering rule so older
 * clients that don't understand it can no-op gracefully.
 */
export type AgentEvent =
  | AgentEventText
  | AgentEventThinking
  | AgentEventToolCall
  | AgentEventToolCallUpdate
  | AgentEventPermissionRequest
  | AgentEventDone;

export interface AgentEventText {
  kind: "text";
  text: string;
}

export interface AgentEventThinking {
  kind: "thinking";
  text: string;
}

export interface AgentEventToolCall {
  kind: "toolCall";
  id: string;
  name: string;
  input: unknown;
  /** Where the tool came from. `cronymax.tool.shell`, `mcp:<server>:<tool>`, `agent:<id>`, etc. */
  source: string;
  status: "in_progress";
}

export interface AgentEventToolCallUpdate {
  kind: "toolCallUpdate";
  id: string;
  status: "completed" | "failed";
  output: unknown;
}

export interface AgentEventPermissionRequest {
  kind: "permissionRequest";
  /** Reply with `resolvePermission(requestId, ...)` on the session. */
  requestId: string;
  tool: string;
  options: unknown;
}

export interface AgentEventDone {
  kind: "done";
  stopReason: "end_turn" | "max_tokens" | "tool_calls" | "cancelled" | "error";
  /** Present when stopReason === "error". */
  errorMessage?: string;
}

// ─── runtime interfaces ────────────────────────────────────────────────────

export interface AgentProvider {
  /**
   * Enumerate the selectable items inside this provider (models, modes,
   * personalities, …). Each item appears as a row under this provider's
   * group in the chat panel's picker. The user's selection is forwarded to
   * `createSession({ model: itemId })`.
   *
   * Return `[]` for providers that have nothing to pick — the platform
   * will surface them as a single "default" entry instead.
   *
   * Replaces the v1-pre `listModels()` + `modes?` pair: contribution items
   * are now the unified dimension.
   */
  enumerate(): Promise<readonly ContributionItem[]>;

  createSession(opts: SessionOptions): Promise<AgentSession>;
}

export interface AgentSession extends Disposable {
  readonly id: string;

  /**
   * Stream the assistant turn(s) triggered by `message`. The iterator MUST
   * complete with exactly one `{ kind: "done" }` event. Cancellation via
   * `token` results in `stopReason: "cancelled"`; the iterator must then end.
   */
  prompt(message: PromptMessage, token: CancellationToken): AsyncIterable<AgentEvent>;

  /**
   * Resolve a permission request previously yielded as
   * `kind: "permissionRequest"`. Calling with a stale requestId is a no-op.
   */
  resolvePermission(requestId: string, decision: PermissionDecision): Promise<void>;

  /** Cooperative cancellation of any in-flight prompt(). */
  cancel(): Promise<void>;
}

export interface PermissionDecision {
  /** Whether to allow the operation. */
  allow: boolean;
  /**
   * If true and `allow` is true, remember the decision for the session
   * (provider responsible for honouring; platform reflects in UI).
   */
  rememberForSession?: boolean;
  /** Provider-defined extra args (e.g. selected option index). */
  data?: unknown;
}

// ─── namespace exposed to extensions ───────────────────────────────────────

export interface AgentsNamespace {
  registerProvider(id: string, impl: AgentProvider): Disposable;
  /**
   * Lookup a previously-registered provider. Useful for cross-extension
   * composition (`acme.router` orchestrating `bytedance.coco`).
   */
  getProvider(id: string): AgentProvider | undefined;
}
