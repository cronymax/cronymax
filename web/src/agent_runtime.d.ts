/**
 * Ambient typings for the legacy ReAct runtime that lives in
 * `web/agent/{llm.js,tools.js,loop.js}` and is loaded as plain `<script>`
 * tags by panels that depend on it (currently `chat/` and `agent/`).
 *
 * Phase 7 keeps this runtime as JS globals; a future change can port it
 * into typed ESM modules.
 */
export interface AgentRunSnapshot {
  tool_calls?: unknown[];
  finish_reason?: string;
}

export interface AgentTraceDetail {
  type: string;
  content?: string;
  tool?: string;
  output?: string;
  message?: string;
  prompt?: string;
  request_id?: string;
  finish_reason?: string;
  node_type?: string;
  node_id?: string;
}

export interface AgentRunResult {
  output?: string;
  finish_reason?: string;
}

export interface AgentGraphInstance {
  addEventListener(
    event: "trace",
    handler: (e: { detail: AgentTraceDetail }) => void,
  ): void;
  addLLMNode(id: string, config: Record<string, unknown>): AgentGraphInstance;
  addToolNode(id: string, config: Record<string, unknown>): AgentGraphInstance;
  addConditionNode(
    id: string,
    fn: (run: AgentRunSnapshot) => string | null,
  ): AgentGraphInstance;
  addHumanNode(id: string, config: Record<string, unknown>): AgentGraphInstance;
  run(input: {
    task: string;
    getPermission?: (prompt: string, requestId: string) => Promise<boolean>;
  }): Promise<AgentRunResult>;
}

export interface LegacyLlmClient {
  baseUrl: string;
  apiKey: string;
  model: string;
  loadConfig(): Promise<void>;
  saveConfig(baseUrl: string, apiKey: string): Promise<void>;
}

declare global {
  interface Window {
    AgentGraph: new () => AgentGraphInstance;
    llmClient: LegacyLlmClient;
    /** Defined by the agent panel so legacy graph human-nodes can prompt the user. */
    __getPermission?: (prompt: string, requestId: string) => Promise<boolean>;
  }
}
