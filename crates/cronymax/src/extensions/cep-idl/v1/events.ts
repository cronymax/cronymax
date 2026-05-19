// SPDX-License-Identifier: Apache-2.0
//
// Cronymax Extension Platform · IDL v1 · events
//
// Topic-based pub/sub.
//
// Subscribing to `cronymax.*` topics requires the topic to be present in
// `capabilities.events.subscribe`. Emitting to `cronymax.*` is always
// rejected; extensions may only emit under their publisher namespace, and
// the topic must appear in `capabilities.events.emit`.

import type { Disposable } from "./primitives";

export type EventHandler<T = unknown> = (payload: T) => void | Promise<void>;

export interface Events {
  on<T = unknown>(topic: string, handler: EventHandler<T>): Disposable;
  emit(topic: string, payload: unknown): Promise<void>;
}

// ─── v1 platform topics (cronymax.*) ───────────────────────────────────────
//
// Every topic below is emitted by the platform. Extensions subscribe via
// `cronymax.events.on(topic, handler)`. Each platform topic must appear in
// `capabilities.events.subscribe` to be receivable.
//
// Wire payload shapes are stable for v1.

export interface SessionStartedPayload {
  sessionId: string;
  providerId: string;
  agentId?: string;
  model?: string;
}

export interface SessionEndedPayload {
  sessionId: string;
  reason: "user" | "error" | "timeout";
}

export interface MessageUserSentPayload {
  sessionId: string;
  turnId: string;
  text: string;
}

export interface MessageAssistantDeltaPayload {
  sessionId: string;
  turnId: string;
  textDelta: string;
}

export interface MessageAssistantDonePayload {
  sessionId: string;
  turnId: string;
  fullText: string;
  finishReason: "end_turn" | "max_tokens" | "tool_calls" | "cancelled" | "error";
}

export interface ToolInvokedPayload {
  sessionId: string;
  turnId: string;
  toolCallId: string;
  name: string;
  input: unknown;
  /** `cronymax.tool.shell`, `mcp:<server>:<tool>`, `agent:<id>`, etc. */
  source: string;
}

export interface ToolCompletedPayload {
  sessionId: string;
  toolCallId: string;
  status: "completed" | "failed" | "cancelled";
  output: unknown;
}

export interface PermissionRequestedPayload {
  sessionId: string;
  requestId: string;
  /** Tool / capability label the user is being prompted about. */
  target: string;
  options: unknown;
}

/** Enumerated platform topics. Add (never remove) for v1 patch releases. */
export const PlatformTopic = {
  SessionStarted: "cronymax.session.started",
  SessionEnded: "cronymax.session.ended",
  MessageUserSent: "cronymax.message.user.sent",
  MessageAssistantDelta: "cronymax.message.assistant.delta",
  MessageAssistantDone: "cronymax.message.assistant.done",
  ToolInvoked: "cronymax.tool.invoked",
  ToolCompleted: "cronymax.tool.completed",
  PermissionRequested: "cronymax.permission.requested",
} as const;

export type PlatformTopic = (typeof PlatformTopic)[keyof typeof PlatformTopic];

/** Maps each platform topic to its payload type. */
export interface PlatformTopicPayloads {
  "cronymax.session.started": SessionStartedPayload;
  "cronymax.session.ended": SessionEndedPayload;
  "cronymax.message.user.sent": MessageUserSentPayload;
  "cronymax.message.assistant.delta": MessageAssistantDeltaPayload;
  "cronymax.message.assistant.done": MessageAssistantDonePayload;
  "cronymax.tool.invoked": ToolInvokedPayload;
  "cronymax.tool.completed": ToolCompletedPayload;
  "cronymax.permission.requested": PermissionRequestedPayload;
}
