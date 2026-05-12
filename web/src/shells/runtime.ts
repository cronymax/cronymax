/**
 * Typed wrappers around window.cronymax.runtime.
 *
 * All renderer↔Rust-runtime traffic goes through CEF process messages
 * (cronymax.runtime.ctrl / ctrl.reply / event) rather than cefQuery.
 * This module provides typed helpers so callers never touch the raw API.
 */

import { runtime } from "./bridge";

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
// Agent registry helpers
// ---------------------------------------------------------------------------

export interface AgentEntry {
  name: string;
  kind: string;
  llm: string;
  builtin?: boolean;
  prompt_sealed?: boolean;
}

export interface AgentDetail extends AgentEntry {
  system_prompt: string;
  memory_namespace: string;
  tools: string[];
}

export const agentRegistry = {
  async list(): Promise<{ agents: AgentEntry[] }> {
    const raw = await runtime.send({ kind: "agent_registry_list" });
    return JSON.parse(raw) as { agents: AgentEntry[] };
  },
  async load(name: string): Promise<AgentDetail> {
    const raw = await runtime.send({ kind: "agent_registry_load", name });
    return JSON.parse(raw) as AgentDetail;
  },
  async save(fields: Record<string, unknown>): Promise<{ ok: boolean }> {
    const raw = await runtime.send({ kind: "agent_registry_save", ...fields });
    return JSON.parse(raw) as { ok: boolean };
  },
  async delete(name: string): Promise<{ ok: boolean }> {
    const raw = await runtime.send({ kind: "agent_registry_delete", name });
    return JSON.parse(raw) as { ok: boolean };
  },
};

// ---------------------------------------------------------------------------
// Flow helpers
// ---------------------------------------------------------------------------

export const flow = {
  async list(): Promise<{ flows: unknown[] }> {
    const raw = await runtime.send({ kind: "flow_list" });
    return JSON.parse(raw) as { flows: unknown[] };
  },
  /** id is the bridge-layer "id" field; mapped to runtime "flow_id". */
  async load(id: string): Promise<unknown> {
    const raw = await runtime.send({ kind: "flow_load", flow_id: id });
    return JSON.parse(raw);
  },
  async save(flow_id: string, graph: unknown): Promise<{ ok: boolean }> {
    const raw = await runtime.send({ kind: "flow_save", flow_id, graph });
    return JSON.parse(raw) as { ok: boolean };
  },
};

// ---------------------------------------------------------------------------
// Doc type helpers
// ---------------------------------------------------------------------------

export const docType = {
  async list(): Promise<{ doc_types: unknown[] }> {
    const raw = await runtime.send({ kind: "doc_type_list" });
    return JSON.parse(raw) as { doc_types: unknown[] };
  },
  async load(name: string): Promise<unknown> {
    const raw = await runtime.send({ kind: "doc_type_load", name });
    return JSON.parse(raw);
  },
  async save(
    name: string,
    display_name: string,
    description: string,
  ): Promise<{ ok: boolean }> {
    const raw = await runtime.send({
      kind: "doc_type_save",
      name,
      display_name,
      description,
    });
    return JSON.parse(raw) as { ok: boolean };
  },
  async delete(name: string): Promise<{ ok: boolean }> {
    const raw = await runtime.send({ kind: "doc_type_delete", name });
    return JSON.parse(raw) as { ok: boolean };
  },
};

// ---------------------------------------------------------------------------
// Flow run helpers
// ---------------------------------------------------------------------------

export const flowRun = {
  async start(
    flow_id: string,
    initial_input?: string,
  ): Promise<{ run_id: string; subscription?: string }> {
    const pl: Record<string, unknown> = { flow_id };
    if (initial_input !== undefined) pl.initial_input = initial_input;
    const raw = await runtime.send({ kind: "start_run", payload: pl });
    return JSON.parse(raw) as { run_id: string; subscription?: string };
  },
  async cancel(run_id: string): Promise<{ ok: boolean }> {
    const raw = await runtime.send({ kind: "cancel_run", run_id });
    return JSON.parse(raw) as { ok: boolean };
  },
  async pause(run_id: string): Promise<{ ok: boolean }> {
    const raw = await runtime.send({ kind: "pause_run", run_id });
    return JSON.parse(raw) as { ok: boolean };
  },
  async resume(run_id: string): Promise<{ ok: boolean }> {
    const raw = await runtime.send({ kind: "resume_run", run_id });
    return JSON.parse(raw) as { ok: boolean };
  },
  async postInput(run_id: string, input: unknown): Promise<{ ok: boolean }> {
    const raw = await runtime.send({
      kind: "post_input",
      run_id,
      payload: input,
    });
    return JSON.parse(raw) as { ok: boolean };
  },
};

/**
 * Start an agent run for a given task string.
 * The browser process injects LLM config and workspace context.
 * Returns the run_id assigned by the Rust runtime.
 *
 * Pass `session_id` (e.g. the chat tab id) to enable session continuity:
 * the runtime will seed the new run from the prior thread and flush the
 * updated thread back to the session on completion.
 *
 * Pass `agent_id` to route to a specific agent definition (e.g. `"crony"`).
 * Pass `model_override` to override the provider's default model for this run.
 */
export async function agentRun(
  task: string,
  opts?: {
    session_id?: string;
    session_name?: string;
    agent_id?: string;
    model_override?: string;
  },
): Promise<string> {
  const payload: Record<string, unknown> = { task };
  if (opts?.model_override) payload.model_override = opts.model_override;
  const req: Record<string, unknown> = { kind: "start_run", payload };
  if (opts?.session_id) req.session_id = opts.session_id;
  if (opts?.session_name) req.session_name = opts.session_name;
  if (opts?.agent_id) req.agent_id = opts.agent_id;
  const raw = await runtime.send(req);
  const res = JSON.parse(raw) as { run_id?: string };
  if (!res.run_id) throw new Error("runtime did not return run_id");
  return res.run_id;
}

// ---------------------------------------------------------------------------
// Terminal helpers
// ---------------------------------------------------------------------------

export const terminal = {
  /** Start the PTY for the given terminal id (cols/rows default to 100x30). */
  start(tid: string, cols = 100, rows = 30): Promise<string> {
    return runtime.send({
      kind: "terminal_start",
      terminal_id: tid,
      cols,
      rows,
    });
  },

  /** Write raw bytes to the PTY. Fire-and-forget; errors are swallowed. */
  input(tid: string, data: string): void {
    runtime
      .send({ kind: "terminal_input", terminal_id: tid, data })
      .catch(() => {});
  },

  /** Write a command line (appends newline). Fire-and-forget. */
  run(tid: string, command: string): Promise<string> {
    return runtime.send({
      kind: "terminal_input",
      terminal_id: tid,
      data: command + "\n",
    });
  },

  /** Notify the PTY of a new terminal size. Fire-and-forget. */
  resize(tid: string, cols: number, rows: number): void {
    runtime
      .send({
        kind: "terminal_resize",
        terminal_id: tid,
        cols,
        rows,
      })
      .catch(() => {});
  },

  /** Kill the running process in the PTY. */
  stop(tid: string): Promise<string> {
    return runtime.send({ kind: "terminal_stop", terminal_id: tid });
  },

  /**
   * Subscribe to PTY output for terminal `tid`.
   * `onData` receives decoded UTF-8 terminal output chunks.
   * Returns an unsubscribe function, or null if the runtime is unavailable.
   */
  subscribeOutput(
    tid: string,
    onData: (data: string) => void,
  ): (() => void) | null {
    return runtime.subscribe(`terminal:${tid}`, (eventJson: string) => {
      try {
        const ev = JSON.parse(eventJson) as Record<string, unknown>;
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
