/**
 * llm.js — OpenAI-compatible LLM client for the AI Desktop agent panel.
 *
 * Usage:
 *   const client = new LLMClient();
 *   await client.loadConfig();          // fetches base_url + api_key via bridge
 *
 *   // Streaming chat:
 *   const stream = client.chat(model, messages, tools);
 *   for await (const chunk of stream) {
 *     // chunk: { type: "delta", content } | { type: "tool_call", name, id, input }
 *     //        | { type: "done", finish_reason }
 *   }
 */

class LLMClient {
  constructor() {
    this.baseUrl = "";
    this.apiKey = "";
    this.model = "gpt-4o-mini";
  }

  // ---------------------------------------------------------------------------
  // Config
  // ---------------------------------------------------------------------------

  async loadConfig() {
    try {
      const raw = await window.aiDesktop.send("llm.config.get", "");
      const cfg = JSON.parse(raw);
      if (cfg.base_url) this.baseUrl = cfg.base_url;
      if (cfg.api_key) this.apiKey = cfg.api_key;
    } catch (e) {
      console.warn("LLMClient: failed to load config", e);
    }
  }

  async saveConfig(baseUrl, apiKey) {
    this.baseUrl = baseUrl;
    this.apiKey = apiKey;
    await window.aiDesktop.send(
      "llm.config.set",
      JSON.stringify({ base_url: baseUrl, api_key: apiKey })
    );
  }

  // ---------------------------------------------------------------------------
  // Core streaming chat
  // ---------------------------------------------------------------------------

  /**
   * @param {string} model
   * @param {Array<{role:string, content:string|Array}>} messages
   * @param {Array} [tools]   OpenAI tool definitions
   * @yields {{ type: "delta", content: string }
   *          | { type: "tool_call", id: string, name: string, input: string }
   *          | { type: "done", finish_reason: string }}
   */
  async *chat(model, messages, tools = []) {
    const endpoint = (this.baseUrl || "http://localhost:11434") + "/v1/chat/completions";

    const body = {
      model: model || this.model,
      messages,
      stream: true,
    };
    if (tools.length > 0) {
      body.tools = tools;
      body.tool_choice = "auto";
    }

    const headers = { "Content-Type": "application/json" };
    if (this.apiKey) headers["Authorization"] = "Bearer " + this.apiKey;

    const resp = await fetch(endpoint, {
      method: "POST",
      headers,
      body: JSON.stringify(body),
    });

    if (!resp.ok) {
      const text = await resp.text();
      throw new Error(`LLM request failed ${resp.status}: ${text}`);
    }

    const reader = resp.body.getReader();
    const decoder = new TextDecoder();
    let buf = "";

    // Accumulate partial tool call inputs across chunks.
    /** @type {Map<number, {id:string, name:string, input:string}>} */
    const toolCalls = new Map();

    while (true) {
      const { done, value } = await reader.read();
      if (done) break;

      buf += decoder.decode(value, { stream: true });

      const lines = buf.split("\n");
      buf = lines.pop() ?? "";

      for (const line of lines) {
        const trimmed = line.trim();
        if (!trimmed || trimmed === "data: [DONE]") {
          if (trimmed === "data: [DONE]") {
            yield { type: "done", finish_reason: "stop" };
          }
          continue;
        }
        if (!trimmed.startsWith("data: ")) continue;

        let chunk;
        try {
          chunk = JSON.parse(trimmed.slice(6));
        } catch {
          continue;
        }

        for (const choice of chunk.choices ?? []) {
          const delta = choice.delta ?? {};

          // Text delta
          if (delta.content) {
            yield { type: "delta", content: delta.content };
          }

          // Tool call deltas
          for (const tc of delta.tool_calls ?? []) {
            const idx = tc.index ?? 0;
            if (!toolCalls.has(idx)) {
              toolCalls.set(idx, { id: "", name: "", input: "" });
            }
            const acc = toolCalls.get(idx);
            if (tc.id) acc.id = tc.id;
            if (tc.function?.name) acc.name = tc.function.name;
            if (tc.function?.arguments) acc.input += tc.function.arguments;
          }

          // On finish, emit accumulated tool calls
          const fr = choice.finish_reason;
          if (fr === "tool_calls") {
            for (const [, tc] of toolCalls) {
              yield { type: "tool_call", id: tc.id, name: tc.name, input: tc.input };
            }
            toolCalls.clear();
            yield { type: "done", finish_reason: "tool_calls" };
          } else if (fr === "stop" || fr === "length") {
            yield { type: "done", finish_reason: fr };
          }
        }
      }
    }
  }

  // ---------------------------------------------------------------------------
  // Non-streaming convenience (collects full response)
  // ---------------------------------------------------------------------------

  async complete(model, messages, tools = []) {
    let text = "";
    const collected_tool_calls = [];
    let finish_reason = "stop";

    for await (const chunk of this.chat(model, messages, tools)) {
      if (chunk.type === "delta") text += chunk.content;
      else if (chunk.type === "tool_call") collected_tool_calls.push(chunk);
      else if (chunk.type === "done") finish_reason = chunk.finish_reason;
    }

    return { text, tool_calls: collected_tool_calls, finish_reason };
  }
}

// Singleton for the agent panel.
window.llmClient = new LLMClient();
