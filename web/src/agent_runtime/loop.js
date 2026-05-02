/**
 * loop.js — Multi-node agent graph executor.
 *
 * Graph node types
 * ────────────────
 *   llm       — call LLM, stream tokens, possibly emit tool_calls
 *   tool      — invoke ToolBridge for each tool call from LLM
 *   condition — route graph based on finish_reason / custom predicate
 *   human     — pause and wait for permission.respond
 *   subgraph  — run a nested AgentGraph
 *
 * State machine
 * ─────────────
 *   Each run() call creates a fresh GraphRun.
 *   Nodes are visited in order; the next node is determined by the
 *   condition node's routing function or by falling through to the
 *   next node in the nodes array.
 *
 * Trace events are emitted via EventTarget so the UI can subscribe.
 *
 *   const graph = new AgentGraph();
 *   graph.on("trace", e => console.log(e.detail));
 *   await graph.run({ model, system, task });
 */

// ---------------------------------------------------------------------------
// GraphRun (mutable state for a single run)
// ---------------------------------------------------------------------------

class GraphRun {
  constructor(input) {
    /** @type {Array<{role:string,content:string|Array}>} */
    this.messages = [];
    this.input = input;
    this.output = "";
    this.finish_reason = "";
    this.tool_calls = [];
    this.aborted = false;
    /** @type {string|null} next node id to jump to (set by condition node) */
    this.goto = null;
  }
}

// ---------------------------------------------------------------------------
// AgentGraph
// ---------------------------------------------------------------------------

class AgentGraph extends EventTarget {
  constructor() {
    super();
    /** @type {Map<string, {type:string, config:object}>} */
    this.nodes = new Map();
    /** @type {string[]} ordered node execution sequence */
    this.nodeOrder = [];
  }

  // -------------------------------------------------------------------------
  // Node registration
  // -------------------------------------------------------------------------

  addLLMNode(id, config = {}) {
    this.nodes.set(id, { type: "llm", config });
    this.nodeOrder.push(id);
    return this;
  }

  addToolNode(id, config = {}) {
    this.nodes.set(id, { type: "tool", config });
    this.nodeOrder.push(id);
    return this;
  }

  addConditionNode(id, routeFn) {
    this.nodes.set(id, { type: "condition", config: { routeFn } });
    this.nodeOrder.push(id);
    return this;
  }

  addHumanNode(id, config = {}) {
    this.nodes.set(id, { type: "human", config });
    this.nodeOrder.push(id);
    return this;
  }

  addSubgraphNode(id, subgraph) {
    this.nodes.set(id, { type: "subgraph", config: { subgraph } });
    this.nodeOrder.push(id);
    return this;
  }

  // -------------------------------------------------------------------------
  // Event helpers
  // -------------------------------------------------------------------------

  _emit(type, data) {
    this.dispatchEvent(new CustomEvent("trace", { detail: { type, ...data } }));
  }

  // -------------------------------------------------------------------------
  // Execution
  // -------------------------------------------------------------------------

  /**
   * @param {{model?:string, system?:string, task:string, tools?:Array}} input
   * @returns {Promise<GraphRun>}
   */
  async run(input) {
    const run = new GraphRun(input);

    // Build initial message list
    if (input.system) {
      run.messages.push({ role: "system", content: input.system });
    }
    run.messages.push({ role: "user", content: input.task });

    this._emit("start", { task: input.task });

    let idx = 0;
    while (idx < this.nodeOrder.length && !run.aborted) {
      // Handle goto (condition node redirect)
      if (run.goto) {
        const target = run.goto;
        run.goto = null;
        const ti = this.nodeOrder.indexOf(target);
        if (ti === -1) {
          this._emit("error", { message: `goto target '${target}' not found` });
          break;
        }
        idx = ti;
      }

      const nodeId = this.nodeOrder[idx];
      const node = this.nodes.get(nodeId);
      if (!node) {
        idx++;
        continue;
      }

      this._emit("node_enter", { node_id: nodeId, node_type: node.type });

      switch (node.type) {
        case "llm":
          await this._runLLMNode(nodeId, node.config, run);
          break;
        case "tool":
          await this._runToolNode(nodeId, node.config, run);
          break;
        case "condition":
          this._runConditionNode(nodeId, node.config, run);
          break;
        case "human":
          await this._runHumanNode(nodeId, node.config, run);
          break;
        case "subgraph":
          await this._runSubgraphNode(nodeId, node.config, run);
          break;
        default:
          this._emit("error", { message: `unknown node type: ${node.type}` });
      }

      this._emit("node_exit", { node_id: nodeId });

      // Only advance idx if no goto was set
      if (!run.goto) idx++;
    }

    this._emit("done", {
      output: run.output,
      finish_reason: run.finish_reason,
    });
    return run;
  }

  // -------------------------------------------------------------------------
  // LLM node — ReAct loop iteration
  // -------------------------------------------------------------------------

  async _runLLMNode(id, config, run) {
    const model =
      config.model ||
      run.input.model ||
      window.llmClient?.model ||
      "gpt-4o-mini";
    const tools = run.input.tools ?? window.TOOL_DEFINITIONS ?? [];

    this._emit("llm_start", { node_id: id, model });

    let textAccum = "";
    run.tool_calls = [];

    try {
      for await (const chunk of window.llmClient.chat(
        model,
        run.messages,
        tools,
      )) {
        if (chunk.type === "delta") {
          textAccum += chunk.content;
          this._emit("llm_delta", { content: chunk.content });
        } else if (chunk.type === "tool_call") {
          run.tool_calls.push(chunk);
        } else if (chunk.type === "done") {
          run.finish_reason = chunk.finish_reason;
        }
      }
    } catch (e) {
      this._emit("error", { message: String(e) });
      run.aborted = true;
      return;
    }

    if (textAccum) {
      run.output = textAccum;
      run.messages.push({ role: "assistant", content: textAccum });
    } else if (run.tool_calls.length > 0) {
      // Assistant message with tool_calls
      run.messages.push({
        role: "assistant",
        content: null,
        tool_calls: run.tool_calls.map((tc) => ({
          id: tc.id,
          type: "function",
          function: { name: tc.name, arguments: tc.input },
        })),
      });
    }

    this._emit("llm_done", {
      node_id: id,
      text: textAccum,
      tool_calls: run.tool_calls,
      finish_reason: run.finish_reason,
    });
  }

  // -------------------------------------------------------------------------
  // Tool node
  // -------------------------------------------------------------------------

  async _runToolNode(id, config, run) {
    if (run.tool_calls.length === 0) return;

    for (const tc of run.tool_calls) {
      this._emit("tool_start", { node_id: id, tool: tc.name, input: tc.input });

      let result;
      if (tc.name === "browser_get_active_page") {
        try {
          const raw = await window.aiDesktop.send(
            "browser.get_active_page",
            "",
          );
          result = { ok: true, output: raw };
        } catch (e) {
          result = { ok: false, error: String(e) };
        }
      } else {
        const nativeName = (window.TOOL_NAME_MAP ?? {})[tc.name] ?? tc.name;
        result = await window.toolBridge.invoke(nativeName, tc.input);
      }

      const content = result.ok ? result.output : `ERROR: ${result.error}`;
      this._emit("tool_done", {
        node_id: id,
        tool: tc.name,
        output: content,
        ok: result.ok,
      });

      // Add tool result to message history
      run.messages.push({
        role: "tool",
        tool_call_id: tc.id,
        content,
      });

      // Terminal tools: when an Agent completes its work by calling
      // `submit_document` (the canonical Agent exit per
      // `agent-document-orchestration`), the ReAct loop ends immediately.
      // We do NOT advance to another LLM turn — the document has been
      // produced, downstream routing is the FlowRuntime's job.
      if (result.ok && tc.name === "submit_document") {
        run.finish_reason = "submit_document";
        run.output = content;
        run.aborted = true;
        this._emit("terminal_tool", { node_id: id, tool: tc.name });
        return;
      }
    }
  }

  // -------------------------------------------------------------------------
  // Condition node
  // -------------------------------------------------------------------------

  _runConditionNode(id, config, run) {
    const next = config.routeFn(run);
    this._emit("condition", { node_id: id, goto: next ?? null });
    if (next) run.goto = next;
  }

  // -------------------------------------------------------------------------
  // Human node (permission gate)
  // -------------------------------------------------------------------------

  async _runHumanNode(id, config, run) {
    const requestId = `perm-${Date.now()}-${Math.random().toString(36).slice(2)}`;
    const prompt = config.prompt ?? "Agent requests permission to proceed.";

    this._emit("human_request", { node_id: id, request_id: requestId, prompt });

    const allowed = await new Promise((resolve) => {
      // The UI should dispatch "permission.respond" with { request_id, decision }
      // via bridge. We listen on the native event bus.
      const handler = (payload) => {
        let parsed;
        try {
          parsed = JSON.parse(payload);
        } catch {
          return;
        }
        if (parsed.request_id !== requestId) return;
        window.aiDesktop.on && cleanup();
        resolve(parsed.decision === "allow");
      };

      const cleanup = window.aiDesktop.on("permission.respond", handler);

      // Also respond to direct UI button if config.getPermission is provided
      if (typeof config.getPermission === "function") {
        config.getPermission(prompt, requestId).then((result) => {
          cleanup?.();
          resolve(result);
        });
      }
    });

    this._emit("human_response", {
      node_id: id,
      request_id: requestId,
      allowed,
    });

    if (!allowed) {
      run.aborted = true;
      run.output = "[Task cancelled by user]";
    }
  }

  // -------------------------------------------------------------------------
  // Subgraph node
  // -------------------------------------------------------------------------

  async _runSubgraphNode(id, config, run) {
    const sub = config.subgraph;
    this._emit("subgraph_start", { node_id: id });

    const subRun = await sub.run({
      ...run.input,
      task: run.output || run.input.task,
    });

    run.output = subRun.output;
    run.messages.push(...subRun.messages.filter((m) => m.role !== "system"));
    this._emit("subgraph_done", { node_id: id, output: subRun.output });
  }
}

// ---------------------------------------------------------------------------
// Default ReAct graph factory
// ---------------------------------------------------------------------------

/**
 * Build the standard ReAct (Reason + Act) graph:
 *
 *   [LLM] → [condition: tool_calls?] → [tool] → back to [LLM]
 *                                    ↘ [done]
 *
 * @param {number} [maxIter=10]
 * @returns {AgentGraph}
 */
function buildReActGraph(maxIter = 10) {
  let iter = 0;

  const graph = new AgentGraph();
  graph
    .addLLMNode("llm")
    .addConditionNode("route", (run) => {
      iter++;
      if (run.aborted) return "done";
      if (run.finish_reason === "tool_calls" && iter < maxIter) return "tool";
      return "done";
    })
    .addToolNode("tool")
    .addConditionNode("after_tool", () => "llm") // loop back
    .addLLMNode("done"); // terminal — just a label; loop exits before reaching it

  // Remove the "done" label node from order so it acts as a sentinel
  graph.nodeOrder.pop();

  return graph;
}

window.AgentGraph = AgentGraph;
window.buildReActGraph = buildReActGraph;
