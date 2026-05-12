/**
 * AppEvent typed-union schemas (agent-event-bus).
 *
 * Closed enum of event kinds mirroring the C++ `cronymax::event_bus::AppEvent`
 * tagged union. Every event carries id (UUIDv7), ts_ms, scope, and a
 * kind-specific payload. The bridge-layer parser refuses unknown kinds.
 */

import { z } from "zod";

const Uuid = z.string().min(1);

// ── Per-kind payload schemas ────────────────────────────────────────────

export const TextPayload = z.object({
  author: z.string().optional(), // populated by host on read; absent in events.append req
  body: z.string(),
  mentions: z.array(z.string()).default([]),
  doc_id: z.string().optional(),
});

export const AgentStatusPayload = z.object({
  status: z.enum(["idle", "thinking", "blocked", "done"]),
  reason: z.string().optional(),
});

export const DocumentEventPayload = z.object({
  doc_id: z.string().min(1),
  doc_path: z.string().min(1),
  doc_type: z.string().min(1),
  revision: z.number().int().positive(),
  producer: z.string().min(1),
  sha256_prefix: z.string().min(1),
});

export const ReviewEventPayload = z.object({
  doc_id: z.string().min(1),
  reviewer: z.string().min(1),
  verdict: z.enum(["approve", "request_changes", "comment"]),
  comment: z.string().optional(),
  round: z.number().int().nonnegative(),
  origin: z.enum(["workbench", "channel", "agent"]).default("agent"),
});

export const HandoffPayload = z.object({
  from_agent: z.string().min(1),
  to_agent: z.string().min(1),
  port: z.string().min(1),
  doc_id: z.string().optional(),
  reason: z.enum(["typed_port", "mention"]),
});

export const ErrorPayload = z.object({
  scope: z.enum(["flow_run", "agent", "tool", "bridge"]),
  code: z.string(),
  message: z.string(),
});

export const SystemPayload = z.object({
  subkind: z.enum(["run_started", "run_paused", "run_completed", "run_cancelled", "flow_updated"]),
  cause: z.string().optional(), // e.g. "human_approval" for run_paused
});

export const FileEditedPayload = z.object({
  path: z.string().min(1),
  /** Unified diff of the change. Empty for full-file writes. */
  diff: z.string().default(""),
  session_id: z.string().optional(),
});

export const GitCommitCreatedPayload = z.object({
  hash: z.string(),
  message: z.string(),
  files_changed: z.array(z.string()),
  session_id: z.string().optional(),
});

export const GitPushedPayload = z.object({
  remote: z.string(),
  branch: z.string(),
  commits_pushed: z.number().int().nonnegative(),
  session_id: z.string().optional(),
});

// ── AppEvent envelope (tagged union) ────────────────────────────────────

const Base = {
  id: Uuid,
  ts_ms: z.number().int().nonnegative(),
  space_id: z.string(),
  flow_id: z.string().nullable().optional(),
  run_id: z.string().nullable().optional(),
  agent_id: z.string().nullable().optional(),
};

export const AppEventSchema = z.discriminatedUnion("kind", [
  z.object({ ...Base, kind: z.literal("text"), payload: TextPayload }),
  z.object({
    ...Base,
    kind: z.literal("agent_status"),
    payload: AgentStatusPayload,
  }),
  z.object({
    ...Base,
    kind: z.literal("document_event"),
    payload: DocumentEventPayload,
  }),
  z.object({
    ...Base,
    kind: z.literal("review_event"),
    payload: ReviewEventPayload,
  }),
  z.object({ ...Base, kind: z.literal("handoff"), payload: HandoffPayload }),
  z.object({ ...Base, kind: z.literal("error"), payload: ErrorPayload }),
  z.object({ ...Base, kind: z.literal("system"), payload: SystemPayload }),
  z.object({
    ...Base,
    kind: z.literal("file_edited"),
    payload: FileEditedPayload,
  }),
  z.object({
    ...Base,
    kind: z.literal("git_commit_created"),
    payload: GitCommitCreatedPayload,
  }),
  z.object({
    ...Base,
    kind: z.literal("git_pushed"),
    payload: GitPushedPayload,
  }),
]);

// ── Runtime streaming events (not stored in EventBus) ───────────────────────

/** Thinking/reasoning token delta from an extended-thinking model turn. */
export const ThinkingTokenPayload = z.object({
  run_id: z.string(),
  turn_id: z.string(),
  delta: z.string(),
});
export type ThinkingTokenPayload = z.infer<typeof ThinkingTokenPayload>;

export type AppEvent = z.infer<typeof AppEventSchema>;
export type AppEventKind = AppEvent["kind"];

// ── Bridge channel payload schemas ──────────────────────────────────────

export const EventsListReq = z.object({
  flow_id: z.string().optional(),
  run_id: z.string().optional(),
  before_id: z.string().optional(),
  limit: z.number().int().positive().max(1000).default(200),
});

export const EventsListRes = z.object({
  events: z.array(AppEventSchema),
  cursor: z.string().default(""), // empty when no more pages
});

export const EventsSubscribeReq = z.object({
  flow_id: z.string().optional(),
  run_id: z.string().optional(),
});

export const EventsSubscribeRes = z.object({
  ok: z.boolean(),
});

export const EventsAppendReq = z.object({
  kind: z.literal("text"),
  flow_id: z.string().min(1),
  run_id: z.string().optional(),
  body: z.string(),
  mentions: z.array(z.string()).default([]),
  doc_id: z.string().optional(),
});

export const EventsAppendRes = z.object({
  id: Uuid,
});

// ── Inbox schemas ───────────────────────────────────────────────────────

export const InboxRowSchema = z.object({
  event_id: Uuid,
  state: z.enum(["unread", "read", "snoozed"]),
  snooze_until: z.number().int().nullable().optional(),
  flow_id: z.string().default(""),
  kind: z.string().default(""),
});

export const InboxListReq = z.object({
  state: z.enum(["unread", "read", "snoozed", "all"]).default("unread"),
  flow_id: z.string().optional(),
  limit: z.number().int().positive().max(500).default(100),
});

export const InboxListRes = z.object({
  rows: z.array(InboxRowSchema),
  unread_count: z.number().int().nonnegative(),
  needs_action_count: z.number().int().nonnegative(),
});

export const InboxStateChangeReq = z.object({
  event_id: Uuid,
});

export const InboxSnoozeReq = z.object({
  event_id: Uuid,
  snooze_until: z.number().int().positive(),
});
