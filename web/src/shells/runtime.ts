/**
 * Typed wrappers around window.cronymax.runtime.
 *
 * All renderer↔Rust-runtime traffic goes through CEF process messages
 * (cronymax.runtime.ctrl / ctrl.reply / event) rather than cefQuery.
 * This module provides typed helpers so callers never touch the raw API.
 */

import { runtime, runtimeSend } from "./bridge";

/** Decode a base64-encoded PTY chunk to a proper UTF-8 string. */
export function b64ToUtf8(b64: string): string {
  const binary = atob(b64);
  const bytes = new Uint8Array(binary.length);
  for (let i = 0; i < binary.length; i++) bytes[i] = binary.charCodeAt(i);
  return new TextDecoder().decode(bytes);
}

// ---------------------------------------------------------------------------
// Terminal helpers
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// Agent run options
// ---------------------------------------------------------------------------

/** Per-message LLM overrides for an agent run (chat-UI selections, etc.). */
export interface AgentRunOptions {
  /** OpenAI reasoning_effort. Empty/undefined = don't override. */
  reasoning_effort?: string;
  /** Anthropic adaptive thinking effort
   * (`low` | `medium` | `high` | `max`). Empty/undefined = don't override. */
  anthropic_effort?: string;
  /** Override the active provider's default model. */
  model?: string;
  /** Override the active provider's wire kind for this run. Set when the
   * user picks a model belonging to a non-active provider group. */
  provider_kind?: string;
  /** Override the active provider's base URL for this run. */
  base_url?: string;
  /** Override the active provider's API key for this run. */
  api_key?: string;
  /** Continue an existing chat session by id. */
  session_id?: string;
  /** Name for a newly-created session. */
  session_name?: string;
  /** Authored agent id (chat agent selector). */
  agent_id?: string;
  /** ContributionKind of the picked agent. Required for new callers; legacy
   * sites that omit it fall through to the runtime's probing heuristic. */
  contribution_kind?: string;
  /** When set, starts a flow run with this flow id instead of a direct agent run. */
  flow_id?: string;
}

// ---------------------------------------------------------------------------
// Contribution registry helpers — unified surface across platform / workspace
// / extension contributions. Mirrors the Rust `ContributionRegistry`.
// ---------------------------------------------------------------------------

/** Owner classification on every contribution descriptor. */
export type ContributionOwner = { type: "platform" } | { type: "workspace" } | { type: "extension"; ext_id: string };

/** One entry returned by `contributionRegistry.list()`. */
export interface ContributionDescriptor {
  kind: string;
  id: string;
  owner: ContributionOwner;
  label: string;
  description?: string;
  icon?: string;
  metadata?: unknown;
}

/** One enumerable item under a descriptor (e.g. a model under an agent provider). */
export interface ContributionItem {
  id: string;
  label: string;
  description?: string;
  icon?: string;
  metadata?: unknown;
}

/** Known contribution kinds. Match `crate::extensions::contributions::kind`. */
export const ContributionKind = {
  AgentsBuiltin: "cronymax.agents.builtin",
  AgentsWorkspace: "cronymax.agents.workspace",
  AgentsProvider: "cronymax.agents.provider",
  Command: "cronymax.command",
  ConfigSchema: "cronymax.config.schema",
  ConfigPage: "cronymax.config.page",
  ContentRenderer: "cronymax.content.renderer",
  SidebarView: "cronymax.ui.sidebar.view",
} as const;
export type ContributionKindId = (typeof ContributionKind)[keyof typeof ContributionKind];

export const contributionRegistry = {
  async list(): Promise<{ contributions: ContributionDescriptor[] }> {
    return (await runtimeSend("contribution.list")) as { contributions: ContributionDescriptor[] };
  },
  async enumerate(contribution_kind: string, id: string): Promise<{ items: ContributionItem[] }> {
    return (await runtimeSend("contribution.enumerate", { contribution_kind, id })) as {
      items: ContributionItem[];
    };
  },
  async load(contribution_kind: string, id: string): Promise<{ descriptor: ContributionDescriptor; source: unknown }> {
    return (await runtimeSend("contribution.load", { contribution_kind, id })) as {
      descriptor: ContributionDescriptor;
      source: unknown;
    };
  },
  async save(contribution_kind: string, id: string, payload: Record<string, unknown>): Promise<{ ok: boolean }> {
    return (await runtimeSend("contribution.save", { contribution_kind, id, payload })) as { ok: boolean };
  },
  async delete(contribution_kind: string, id: string): Promise<{ ok: boolean }> {
    return (await runtimeSend("contribution.delete", { contribution_kind, id })) as { ok: boolean };
  },
};

// ---------------------------------------------------------------------------
// Flow helpers
// ---------------------------------------------------------------------------

export const flow = {
  async list(): Promise<{ flows: unknown[] }> {
    return (await runtimeSend("flow.list")) as { flows: unknown[] };
  },
  /** id is the bridge-layer "id" field; mapped to runtime "flow_id". */
  async load(id: string): Promise<unknown> {
    return await runtimeSend("flow.load", { flow_id: id });
  },
  async save(flow_id: string, graph: unknown): Promise<{ ok: boolean }> {
    return (await runtimeSend("flow.save", { flow_id, graph })) as { ok: boolean };
  },
};

// ---------------------------------------------------------------------------
// Doc type helpers
// ---------------------------------------------------------------------------

export const docType = {
  async list(): Promise<{ doc_types: unknown[] }> {
    return (await runtimeSend("doc.type.list")) as { doc_types: unknown[] };
  },
  async load(name: string): Promise<unknown> {
    return await runtimeSend("doc.type.load", { name });
  },
  async save(name: string, display_name: string, description: string): Promise<{ ok: boolean }> {
    return (await runtimeSend("doc.type.save", { name, display_name, description })) as { ok: boolean };
  },
  async delete(name: string): Promise<{ ok: boolean }> {
    return (await runtimeSend("doc.type.delete", { name })) as { ok: boolean };
  },
};

// ---------------------------------------------------------------------------
// Flow run helpers
// ---------------------------------------------------------------------------

/** A pending human document review from a flow run. */
export interface FlowDocReview {
  flow_run_id: string;
  node_id: string;
  port: string;
  doc_path: string;
  /** Document markdown content (null if file not yet readable). */
  content: string | null;
  /** Chat session that originated the flow run; null for legacy runs. */
  originating_session_id?: string | null;
}

/** Response from getSessionPendingActions. */
export interface SessionPendingActionsResponse {
  /** Pending doc reviews for flow runs bound to this session. */
  doc_reviews: FlowDocReview[];
  /** Pending tool-approval reviews for agent runs in this session. */
  approvals: unknown[];
}

/** A structured reviewer comment for request-changes. */
export interface FlowReviewComment {
  severity?: "error" | "warn" | "info";
  message: string;
  suggestion?: string;
}

export const flowRun = {
  async start(flow_id: string, initial_input?: string): Promise<{ run_id: string; subscription?: string }> {
    const pl: Record<string, unknown> = { flow_id };
    if (initial_input !== undefined) pl.initial_input = initial_input;
    return (await runtimeSend("start.run", { payload: pl })) as { run_id: string; subscription?: string };
  },
  async cancel(run_id: string): Promise<{ ok: boolean }> {
    return (await runtimeSend("cancel.run", { run_id })) as { ok: boolean };
  },
  async pause(run_id: string): Promise<{ ok: boolean }> {
    return (await runtimeSend("pause.run", { run_id })) as { ok: boolean };
  },
  async resume(run_id: string): Promise<{ ok: boolean }> {
    return (await runtimeSend("resume.run", { run_id })) as { ok: boolean };
  },
  async postInput(run_id: string, input: unknown): Promise<{ ok: boolean }> {
    return (await runtimeSend("post.input", { run_id, payload: input })) as { ok: boolean };
  },
  /** Return all document ports in InReview state for the given flow run. */
  async getPendingReviews(flow_run_id: string): Promise<{ pending_reviews: FlowDocReview[] }> {
    return (await runtimeSend("flow.run.get_pending_reviews", { flow_run_id })) as {
      pending_reviews: FlowDocReview[];
    };
  },
  /**
   * Scan ALL runs in the active workspace for pending reviews.
   * Safe to call on startup without knowing any flow_run_id.
   * Each returned item includes `flow_run_id` so approve/requestChanges work.
   */
  async getWorkspacePendingReviews(): Promise<{ pending_reviews: FlowDocReview[] }> {
    return (await runtimeSend("flow.run.get_pending_reviews", { flow_run_id: "" })) as {
      pending_reviews: FlowDocReview[];
    };
  },
  /**
   * Return all pending doc reviews AND tool-approval reviews that belong to
   * the given session in a single round-trip. workspace_root is injected by
   * the C++ enricher.
   */
  async getSessionPendingActions(sessionId: string): Promise<SessionPendingActionsResponse> {
    return (await runtimeSend("get.session.pending.actions", {
      session_id: sessionId,
    })) as SessionPendingActionsResponse;
  },
  /** Approve a pending document review, triggering downstream agents. */
  async approve(flow_run_id: string, node_id: string, port: string): Promise<void> {
    await runtimeSend("flow.run.approve", { flow_run_id, node_id, port });
  },
  /** Request changes on a pending document, re-queuing the producing agent. */
  async requestChanges(
    flow_run_id: string,
    node_id: string,
    port: string,
    comments: FlowReviewComment[],
  ): Promise<void> {
    await runtimeSend("flow.run.request_changes", { flow_run_id, node_id, port, comments });
  },
};

/**
 * Start an agent run for a given task string.
 * The browser process injects LLM config and workspace context. Per-message
 * overrides in `opts` (model, reasoning_effort) are forwarded through
 * `payload.llm.*` and merged with the active provider record on the C++
 * side — caller-supplied values win.
 * Returns the run_id assigned by the Rust runtime.
 *
 * Pass `session_id` (e.g. the chat tab id) to enable session continuity:
 * the runtime will seed the new run from the prior thread and flush the
 * updated thread back to the session on completion.
 *
 * Pass `agent_id` to route to a specific agent definition (e.g. `"crony"`).
 * Pass `model` to override the provider's default model for this run.
 */
export async function agentRun(task: string, opts: AgentRunOptions = {}): Promise<string> {
  const payload: Record<string, unknown> = { task };
  const llm: Record<string, unknown> = {};
  if (opts.reasoning_effort) llm.reasoning_effort = opts.reasoning_effort;
  if (opts.anthropic_effort) llm.anthropic_effort = opts.anthropic_effort;
  if (opts.model) llm.model = opts.model;
  if (opts.provider_kind) llm.provider_kind = opts.provider_kind;
  if (opts.base_url) llm.base_url = opts.base_url;
  if (opts.api_key) llm.api_key = opts.api_key;
  if (Object.keys(llm).length > 0) payload.llm = llm;
  if (opts.flow_id) payload.flow_id = opts.flow_id;
  const req: Record<string, unknown> = { payload };
  if (opts.session_id) req.session_id = opts.session_id;
  if (opts.session_name) req.session_name = opts.session_name;
  if (opts.agent_id) req.agent_id = opts.agent_id;
  if (opts.contribution_kind) req.contribution_kind = opts.contribution_kind;
  const res = (await runtimeSend("start.run", req)) as { run_id?: string };
  if (!res.run_id) throw new Error("runtime did not return run_id");
  return res.run_id;
}

// ---------------------------------------------------------------------------
// Terminal helpers
// ---------------------------------------------------------------------------

export const terminal = {
  /** Start the PTY for the given terminal id (cols/rows default to 100x30). */
  async start(tid: string, cols = 100, rows = 30): Promise<unknown> {
    return runtimeSend("terminal.start", { terminal_id: tid, cols, rows });
  },

  /** Write raw bytes to the PTY. Fire-and-forget; errors are swallowed. */
  input(tid: string, data: string): void {
    runtimeSend("terminal.input", { terminal_id: tid, data }).catch(() => {
      /* ignore */
    });
  },

  /** Write a command line (appends newline). Fire-and-forget. */
  async run(tid: string, command: string): Promise<unknown> {
    return runtimeSend("terminal.input", { terminal_id: tid, data: `${command}\n` });
  },

  /** Notify the PTY of a new terminal size. Fire-and-forget. */
  resize(tid: string, cols: number, rows: number): void {
    runtimeSend("terminal.resize", { terminal_id: tid, cols, rows }).catch(() => {
      /* ignore */
    });
  },

  /** Kill the running process in the PTY. */
  async stop(tid: string): Promise<unknown> {
    return runtimeSend("terminal.stop", { terminal_id: tid });
  },

  /**
   * Subscribe to PTY output for terminal `tid`.
   * `onData` receives decoded UTF-8 terminal output chunks.
   * Returns an unsubscribe function, or null if the runtime is unavailable.
   */
  subscribeOutput(tid: string, onData: (data: string) => void): (() => void) | null {
    return runtime.on(`terminal:${tid}`, (event: unknown) => {
      try {
        const ev = event as Record<string, unknown>;
        const pl = ev?.payload as Record<string, unknown> | undefined;
        if (pl?.kind !== "raw") return;
        const dataObj = pl?.data as Record<string, unknown> | undefined;
        const b64 = dataObj?.data as string | undefined;
        if (!b64) return;
        onData(b64ToUtf8(b64));
      } catch {
        // Ignore malformed events.
      }
    });
  },
};
