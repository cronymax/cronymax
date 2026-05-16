/**
 * Flow editor store. Holds agent nodes/edges plus selection and trace log.
 *
 * Model
 * -----
 * The flow editor mirrors the runtime `FlowDefinition` defined in
 * `app/flow/flow_definition.h`: a flow is an ordered set of `agents`
 * connected by typed edges that carry a document of a specific `port`
 * (doc-type) from one agent to the next. There are no primitive
 * `LLM` / `Tool` / `Branch` / `Human` nodes in the runtime — every node
 * is an Agent (worker or reviewer) defined by an `AgentDefinition` in
 * the agent registry.
 *
 * Drag-in-progress state lives in a ref inside `App.tsx` to avoid
 * re-rendering on every mousemove.
 */
import { createPanelStore } from "@/hooks/usePanelStore";

/**
 * The editor only knows one node type now — every node is an Agent.
 * The worker vs reviewer distinction comes from the underlying
 * `AgentDefinition.kind` carried in node config.
 */
export type NodeType = "agent";

/**
 * One document type that this agent can emit, together with the reviewer
 * agents that must approve it before it can flow downstream.
 */
export interface ProducesEntry {
  doc_type: string;
  /** Comma-separated reviewer agent names. */
  reviewers: string;
}

export interface GraphNode {
  id: number;
  type: NodeType;
  /** Display label; defaults to the agent name. */
  name: string;
  x: number;
  y: number;
  /**
   * Documents this agent can emit. Each entry carries a doc-type and the
   * reviewer agents that must approve it before it flows downstream.
   */
  produces: ProducesEntry[];
  /**
   * Agent placement config. Keys:
   *   - `agent_name`: FK into AgentRegistry.
   */
  config: Record<string, string>;
}

export interface GraphEdge {
  from_id: number;
  to_id: number;
  /** doc-type carried over this edge (matches `FlowEdge::port`). */
  port?: string;
  /** Optional human-approval gate (matches `FlowEdge::requires_human_approval`). */
  requires_human_approval?: boolean;
  /** Free-form label, kept for back-compat with persisted localStorage flows. */
  label?: string;
}

export interface FlowSpec {
  nodes: GraphNode[];
  edges: GraphEdge[];
}

export interface AgentRegistryEntry {
  name: string;
  llm: string;
}

export interface DocTypeEntry {
  name: string;
  display_name: string;
  user_defined: boolean;
}

export interface State {
  nodes: GraphNode[];
  edges: GraphEdge[];
  selectedId: number | null;
  selectedEdgeIndex: number | null;
  nextId: number;
  flowNames: string[];
  activeFlowName: string;
  /** Editable input next to the flow selector. */
  flowNameInput: string;
  trace: string;
  running: boolean;
  /** Highlight set, derived from the trace stream. */
  runningId: number | null;
  doneId: number | null;
  /** Catalog of agents available to drop on the canvas. */
  agentCatalog: AgentRegistryEntry[];
  /** Catalog of doc-types available for `produces` / edge `port`. */
  docTypeCatalog: DocTypeEntry[];
}

export type Action =
  | { type: "setFlow"; spec: FlowSpec }
  | { type: "addNode"; node: GraphNode; nextId: number; edge?: GraphEdge }
  | { type: "deleteNode"; id: number }
  | { type: "select"; id: number | null }
  | { type: "selectEdge"; index: number | null }
  | { type: "updateNodeName"; id: number; name: string }
  | { type: "updateNodeConfig"; id: number; key: string; value: string }
  | { type: "updateNodePosition"; id: number; x: number; y: number }
  | {
      type: "updateEdge";
      index: number;
      patch: Partial<Pick<GraphEdge, "port" | "requires_human_approval" | "label">>;
    }
  | { type: "deleteEdge"; index: number }
  | { type: "addEdge"; edge: GraphEdge }
  | { type: "updateNodeProduces"; id: number; produces: ProducesEntry[] }
  | { type: "setFlowNames"; names: string[]; active: string }
  | { type: "setActiveFlow"; name: string }
  | { type: "setFlowNameInput"; value: string }
  | { type: "appendTrace"; chunk: string }
  | { type: "clearTrace" }
  | { type: "setRunning"; running: boolean }
  | { type: "setHighlight"; running: number | null; done: number | null }
  | { type: "setAgentCatalog"; agents: AgentRegistryEntry[] }
  | { type: "setDocTypeCatalog"; docTypes: DocTypeEntry[] }
  | { type: "clear" };

const initial: State = {
  nodes: [],
  edges: [],
  selectedId: null,
  selectedEdgeIndex: null,
  nextId: 1,
  flowNames: [],
  activeFlowName: "",
  flowNameInput: "",
  trace: "",
  running: false,
  runningId: null,
  doneId: null,
  agentCatalog: [],
  docTypeCatalog: [],
};

/**
 * Migrate a persisted FlowSpec to the current schema.
 * Handles two legacy shapes:
 *  1. `node.config.produces` (single string) → `node.produces[0]`
 *  2. `edge.reviewers` (comma string)        → merged into source node's
 *     matching `ProducesEntry.reviewers`.
 */
function migrateFlowSpec(raw: FlowSpec): FlowSpec {
  // Step 1: lift config.produces → produces array per node.
  const nodes: GraphNode[] = (raw.nodes ?? [])
    .filter((n) => n && n.type === "agent")
    .map((n) => {
      const config = { ...(n.config ?? {}) } as Record<string, string>;
      let produces: ProducesEntry[] = Array.isArray(n.produces) ? [...n.produces] : [];
      if (produces.length === 0 && config.produces) {
        produces = [{ doc_type: config.produces!, reviewers: "" }];
      }
      // Strip old config keys.
      delete config.produces;
      delete config.reviewers;
      return { ...n, config, produces };
    });

  // Step 2: rename legacy "Chat" lead-node agent_name to "Crony".
  // The lead node is the one with the smallest id.
  if (nodes.length > 0) {
    const leadIdx = nodes.reduce((minI, n, i) => (n.id < nodes[minI]!.id ? i : minI), 0);
    const lead = nodes[leadIdx]!;
    if ((lead.config?.agent_name ?? lead.name) === "Chat") {
      nodes[leadIdx] = {
        ...lead,
        name: "Crony",
        config: { ...(lead.config ?? {}), agent_name: "Crony" },
      };
    }
  }

  // Step 3: drain edge.reviewers → source node's matching ProducesEntry.
  const nodeMap = new Map(nodes.map((n) => [n.id, n]));
  const edges: GraphEdge[] = (raw.edges ?? []).map((e) => {
    const edgeRevs = (e as GraphEdge & { reviewers?: string }).reviewers;
    if (!edgeRevs || !e.port) return e;
    const src = nodeMap.get(e.from_id);
    if (!src) return e;
    const entryIdx = src.produces.findIndex((p) => p.doc_type === e.port);
    const incoming = edgeRevs
      .split(",")
      .map((s) => s.trim())
      .filter(Boolean);
    if (entryIdx >= 0) {
      const existing = new Set(
        src.produces[entryIdx]!.reviewers.split(",")
          .map((s) => s.trim())
          .filter(Boolean),
      );
      for (const r of incoming) existing.add(r);
      src.produces[entryIdx] = {
        ...src.produces[entryIdx]!,
        reviewers: Array.from(existing).join(","),
      };
    } else {
      src.produces.push({ doc_type: e.port, reviewers: edgeRevs });
    }
    const { reviewers: _r, ...rest } = e as GraphEdge & { reviewers?: string };
    return rest as GraphEdge;
  });

  return { nodes, edges };
}

/**
 * Drop nodes that the editor no longer knows how to render. Persisted
 * legacy flows (LLM / Tool / Branch / Human / Start) are stripped
 * silently so opening an old flow shows an empty canvas instead of
 * crashing.
 */
function sanitizeNodes(nodes: GraphNode[]): GraphNode[] {
  return (nodes ?? [])
    .filter((n) => n && n.type === "agent")
    .map((n) => ({
      ...n,
      config: { ...(n.config ?? {}) },
      produces: Array.isArray(n.produces) ? n.produces : [],
    }));
}

function sanitizeEdges(edges: GraphEdge[], nodeIds: Set<number>): GraphEdge[] {
  return (edges ?? []).filter((e) => nodeIds.has(e.from_id) && nodeIds.has(e.to_id));
}

/**
 * Ensure the Crony agent is always node id=1 (the non-deletable lead node).
 *
 * If the lead node (smallest id) already has `config.agent_name === "Crony"`,
 * the spec is returned unchanged. Otherwise all existing node ids are shifted
 * by +1, their x-positions are shifted right by NODE_X_STEP, and a Crony node
 * is prepended at id=1.
 *
 * Note: legacy "Chat" lead nodes are already converted to "Crony" by
 * `migrateFlowSpec` before this function is called.
 */
const NODE_X_STEP_INTERNAL = 300;
const NODE_Y_CRONY = 120;

function ensureCronyLeadNode(spec: FlowSpec): FlowSpec {
  const nodes = spec.nodes ?? [];
  if (nodes.length === 0) {
    // Empty flow → just the Crony node
    return {
      nodes: [
        {
          id: 1,
          type: "agent",
          name: "Crony",
          x: 80,
          y: NODE_Y_CRONY,
          produces: [],
          config: { agent_name: "Crony" },
        },
      ],
      edges: spec.edges ?? [],
    };
  }
  const sorted = [...nodes].sort((a, b) => a.id - b.id);
  const lead = sorted[0]!;
  if ((lead.config?.agent_name ?? lead.name) === "Crony") {
    // Crony is already the lead node — ensure there's an edge from Crony to
    // the first non-Crony workflow node (may be missing in flows loaded from
    // localStorage that were saved before this requirement was added).
    const firstWorkflow = sorted[1];
    if (!firstWorkflow) return spec;
    const edges = spec.edges ?? [];
    const hasLeadEdge = edges.some((e) => e.from_id === lead.id && e.to_id === firstWorkflow.id);
    if (hasLeadEdge) return spec;
    return {
      ...spec,
      edges: [{ from_id: lead.id, to_id: firstWorkflow.id }, ...edges],
    };
  }
  // Shift every existing node id and x position to make room for Crony at id=1.
  const idShift = lead.id === 1 ? 1 : 0; // only shift when id=1 would collide
  const cronyNode: GraphNode = {
    id: 1,
    type: "agent",
    name: "Crony",
    x: lead.x === 80 ? 80 : lead.x - NODE_X_STEP_INTERNAL,
    y: NODE_Y_CRONY,
    produces: [],
    config: { agent_name: "Crony" },
  };
  const shiftedNodes: GraphNode[] = nodes.map((n) => ({
    ...n,
    id: n.id + idShift,
    x: idShift ? n.x + NODE_X_STEP_INTERNAL : n.x,
  }));
  const shiftedEdges: GraphEdge[] = (spec.edges ?? []).map((e) => ({
    ...e,
    from_id: e.from_id + idShift,
    to_id: e.to_id + idShift,
  }));
  // Add edge from Crony (id=1) to the first workflow node.
  const firstWorkflowId = lead.id + idShift;
  const cronyEdge: GraphEdge = { from_id: 1, to_id: firstWorkflowId };
  return {
    nodes: [cronyNode, ...shiftedNodes],
    edges: [cronyEdge, ...shiftedEdges],
  };
}

function reducer(state: State, action: Action): State {
  switch (action.type) {
    case "setFlow": {
      const migrated = ensureCronyLeadNode(migrateFlowSpec(action.spec));
      const ns = sanitizeNodes(migrated.nodes ?? []);
      const ids = new Set(ns.map((n) => n.id));
      const es = sanitizeEdges(migrated.edges ?? [], ids);
      const nextId = ns.reduce((m, n) => Math.max(m, Number(n.id) || 0), 0) + 1;
      return {
        ...state,
        nodes: ns,
        edges: es,
        selectedId: null,
        selectedEdgeIndex: null,
        nextId,
      };
    }
    case "addNode":
      return {
        ...state,
        nodes: [...state.nodes, action.node],
        edges: action.edge ? [...state.edges, action.edge] : state.edges,
        nextId: action.nextId,
        selectedId: action.node.id,
        selectedEdgeIndex: null,
      };
    case "deleteNode": {
      // Lead node (smallest id) is non-deletable: it owns the @-fallback
      // routing in flow chat mode.
      const lead = leadNodeId(state.nodes);
      if (action.id === lead) return state;
      return {
        ...state,
        nodes: state.nodes.filter((n) => n.id !== action.id),
        edges: state.edges.filter((e) => e.from_id !== action.id && e.to_id !== action.id),
        selectedId: state.selectedId === action.id ? null : state.selectedId,
      };
    }
    case "select":
      return { ...state, selectedId: action.id, selectedEdgeIndex: null };
    case "selectEdge":
      return { ...state, selectedEdgeIndex: action.index, selectedId: null };
    case "updateNodeName":
      return {
        ...state,
        nodes: state.nodes.map((n) => (n.id === action.id ? { ...n, name: action.name } : n)),
      };
    case "updateNodeConfig":
      return {
        ...state,
        nodes: state.nodes.map((n) =>
          n.id === action.id ? { ...n, config: { ...n.config, [action.key]: action.value } } : n,
        ),
      };
    case "updateNodeProduces":
      return {
        ...state,
        nodes: state.nodes.map((n) => (n.id === action.id ? { ...n, produces: action.produces } : n)),
      };
    case "updateNodePosition":
      return {
        ...state,
        nodes: state.nodes.map((n) => (n.id === action.id ? { ...n, x: action.x, y: action.y } : n)),
      };
    case "updateEdge":
      return {
        ...state,
        edges: state.edges.map((e, i) => (i === action.index ? { ...e, ...action.patch } : e)),
      };
    case "deleteEdge":
      return {
        ...state,
        edges: state.edges.filter((_, i) => i !== action.index),
        selectedEdgeIndex: state.selectedEdgeIndex === action.index ? null : state.selectedEdgeIndex,
      };
    case "addEdge":
      return { ...state, edges: [...state.edges, action.edge] };
    case "setFlowNames":
      return {
        ...state,
        flowNames: action.names,
        activeFlowName: action.active,
      };
    case "setActiveFlow":
      return {
        ...state,
        activeFlowName: action.name,
        flowNameInput: action.name,
      };
    case "setFlowNameInput":
      return { ...state, flowNameInput: action.value };
    case "appendTrace":
      return { ...state, trace: state.trace + action.chunk };
    case "clearTrace":
      return { ...state, trace: "" };
    case "setRunning":
      return { ...state, running: action.running };
    case "setHighlight":
      return { ...state, runningId: action.running, doneId: action.done };
    case "setAgentCatalog":
      return { ...state, agentCatalog: action.agents };
    case "setDocTypeCatalog":
      return { ...state, docTypeCatalog: action.docTypes };
    case "clear":
      return {
        ...state,
        nodes: [],
        edges: [],
        selectedId: null,
        selectedEdgeIndex: null,
        nextId: 1,
      };
    default:
      return state;
  }
}

export const { Provider, useStore } = createPanelStore<State, Action>(reducer, initial);

// ── localStorage helpers ──────────────────────────────────────────────────
const FLOWS_KEY = "flows";
const ACTIVE_FLOW_KEY = "active_flow";
const LEGACY_KEY = "agent_graph";

export function loadAllFlows(): Record<string, FlowSpec> {
  try {
    const raw = localStorage.getItem(FLOWS_KEY);
    return raw ? (JSON.parse(raw) as Record<string, FlowSpec>) : {};
  } catch {
    return {};
  }
}

export function saveAllFlows(flows: Record<string, FlowSpec>): void {
  try {
    localStorage.setItem(FLOWS_KEY, JSON.stringify(flows));
  } catch {
    /* ignore */
  }
}

export function getActiveFlowName(): string {
  return localStorage.getItem(ACTIVE_FLOW_KEY) || "";
}

export function setActiveFlowName(name: string): void {
  if (name) localStorage.setItem(ACTIVE_FLOW_KEY, name);
  else localStorage.removeItem(ACTIVE_FLOW_KEY);
}

export function migrateLegacy(flows: Record<string, FlowSpec>): Record<string, FlowSpec> {
  try {
    const raw = localStorage.getItem(LEGACY_KEY);
    if (!raw) return flows;
    const obj = JSON.parse(raw) as FlowSpec;
    if (!obj || !Array.isArray(obj.nodes)) return flows;
    if (!flows.default) {
      flows.default = { nodes: obj.nodes, edges: obj.edges || [] };
      saveAllFlows(flows);
    }
  } catch {
    /* ignore */
  }
  return flows;
}

export function syncLegacyKey(spec: FlowSpec): void {
  try {
    localStorage.setItem(LEGACY_KEY, JSON.stringify(spec));
  } catch {
    /* ignore */
  }
}

// ── Built-in "Chat" seed flow ─────────────────────────────────────────────
//
// This preset is written to localStorage the first time the flow editor
// opens with an empty flow store. It uses the Crony orchestrator as the
// single lead node so the user can chat with it and delegate to workspace
// agents. Users can freely rename, reconfigure, or delete it.
export const SEED_CHAT_FLOW: FlowSpec = {
  nodes: [
    {
      id: 1,
      type: "agent",
      name: "Crony",
      x: 80,
      y: 60,
      produces: [],
      config: {
        agent_name: "Crony",
      },
    },
  ],
  edges: [],
};
// ── Built-in "software-dev-cycle" seed flow ──────────────────────────────
//
// Full PM → RD → QA preset. Seeded into localStorage whenever it is
// absent (including for users who already have other flows). Users can
// freely edit or delete it.
export const SEED_SOFTWARE_DEV_CYCLE_FLOW: FlowSpec = {
  // Crony is always id=1 (the non-deletable lead node).
  // PM/RD/QA are id=2/3/4 respectively.
  nodes: [
    {
      id: 1,
      type: "agent",
      name: "Crony",
      x: 80,
      y: 120,
      produces: [],
      config: { agent_name: "Crony" },
    },
    {
      id: 2,
      type: "agent",
      name: "pm",
      x: 380,
      y: 120,
      produces: [{ doc_type: "prd", reviewers: "critic" }],
      config: {
        agent_name: "pm",
      },
    },
    {
      id: 3,
      type: "agent",
      name: "rd",
      x: 680,
      y: 120,
      produces: [
        { doc_type: "tech-spec", reviewers: "critic,qa-critic" },
        { doc_type: "patch-note", reviewers: "critic" },
      ],
      config: {
        agent_name: "rd",
      },
    },
    {
      id: 4,
      type: "agent",
      name: "qa",
      x: 980,
      y: 120,
      produces: [
        { doc_type: "test-report", reviewers: "critic" },
        { doc_type: "bug-report", reviewers: "" },
      ],
      config: {
        agent_name: "qa",
      },
    },
  ],
  edges: [
    // PM → RD: PRD after human + critic approval
    { from_id: 2, to_id: 3, port: "prd", requires_human_approval: true },
    // RD → QA: tech-spec after human + critic + qa-critic approval
    { from_id: 3, to_id: 4, port: "tech-spec", requires_human_approval: true },
    // RD → QA: submit-for-testing handoff (no gate)
    { from_id: 3, to_id: 4, port: "submit-for-testing" },
    // QA → RD: bug reports (max 5 fix cycles)
    { from_id: 4, to_id: 3, port: "bug-report" },
    // RD → QA: patch notes after each fix
    { from_id: 3, to_id: 4, port: "patch-note" },
  ],
};

// Lead-agent convention: the node with the smallest id in a flow is the
// "lead" — it cannot be deleted, and it receives messages in flow chat
// mode that don't address a specific agent via @mention.
export function leadNodeId(nodes: GraphNode[]): number | null {
  if (nodes.length === 0) return null;
  let lead = nodes[0]!;
  for (const n of nodes) if (n.id < lead.id) lead = n;
  return lead.id;
}

// ── Convert a runtime flow definition (from flow_load IPC) to a FlowSpec ──
//
// Used when the flow editor discovers workspace flows that aren't in
// localStorage yet. Nodes are arranged left-to-right in the order they
// appear in the YAML definition.

interface RuntimeFlowNode {
  id: string;
  owner: string;
  outputs?: Array<{
    port: string;
    routes_to?: string | null;
    reviewers?: string[];
  }>;
}

interface RuntimeFlowDef {
  id: string;
  name: string;
  agents: string[];
  /** Current-schema nodes (preferred). */
  nodes?: RuntimeFlowNode[];
  /** Legacy edges (fallback). */
  edges?: Array<{
    from: string;
    to: string;
    port: string;
    requires_human_approval?: boolean;
  }>;
}

const NODE_X_STEP = 300;
const NODE_Y_BASE = 120;

/**
 * The Crony node is always injected as node id=1 (the lead node) so that
 * workspace flows loaded from YAML still expose an orchestrator entry point.
 * All YAML-defined nodes are offset to start from id=2.
 */
const CRONY_LEAD_NODE: GraphNode = {
  id: 1,
  type: "agent",
  name: "Crony",
  x: 80,
  y: NODE_Y_BASE,
  produces: [],
  config: { agent_name: "Crony" },
};

/** Convert a runtime flow definition to a visual `FlowSpec` with auto-layout. */
export function flowSpecFromDef(def: RuntimeFlowDef): FlowSpec {
  // All YAML-defined nodes are offset by +1 so that the injected Chat lead
  // node always occupies id=1 (the lead slot).
  const ID_OFFSET = 1;

  if (def.nodes && def.nodes.length > 0) {
    // Current node-centric schema
    const nodeIdToNum = new Map<string, number>();
    const yamlNodes: GraphNode[] = def.nodes.map((n, i) => {
      const numId = i + 1 + ID_OFFSET;
      nodeIdToNum.set(n.id, numId);
      const produces: ProducesEntry[] = (n.outputs ?? [])
        .filter((o) => o.reviewers && o.reviewers.length > 0)
        .map((o) => ({
          doc_type: o.port,
          reviewers: (o.reviewers ?? []).filter((r) => r !== "human").join(","),
        }));
      return {
        id: numId,
        type: "agent" as const,
        name: n.id,
        // Chat is at x=80; YAML nodes start one step further right.
        x: 80 + (i + 1) * NODE_X_STEP,
        y: NODE_Y_BASE,
        produces,
        config: { agent_name: n.owner },
      };
    });
    const graphEdges: GraphEdge[] = [];
    for (const n of def.nodes) {
      const fromNum = nodeIdToNum.get(n.id);
      if (fromNum == null) continue;
      for (const o of n.outputs ?? []) {
        if (o.routes_to) {
          const toNum = nodeIdToNum.get(o.routes_to);
          if (toNum != null) {
            graphEdges.push({ from_id: fromNum, to_id: toNum, port: o.port });
          }
        }
      }
    }
    return {
      nodes: [{ ...CRONY_LEAD_NODE }, ...yamlNodes],
      edges: [{ from_id: 1, to_id: 1 + ID_OFFSET }, ...graphEdges],
    };
  }

  // Legacy edge schema: use agent list as nodes
  const agents = def.agents ?? [];
  const agentToNum = new Map<string, number>();
  const yamlNodes: GraphNode[] = agents.map((agentId, i) => {
    const numId = i + 1 + ID_OFFSET;
    agentToNum.set(agentId, numId);
    return {
      id: numId,
      type: "agent" as const,
      name: agentId,
      x: 80 + (i + 1) * NODE_X_STEP,
      y: NODE_Y_BASE,
      produces: [],
      config: { agent_name: agentId },
    };
  });
  const graphEdges: GraphEdge[] = (def.edges ?? []).flatMap((e) => {
    const from_id = agentToNum.get(e.from);
    const to_id = agentToNum.get(e.to);
    if (from_id == null || to_id == null) return [];
    return [
      {
        from_id,
        to_id,
        port: e.port,
        requires_human_approval: e.requires_human_approval,
      },
    ];
  });
  return {
    nodes: [{ ...CRONY_LEAD_NODE }, ...yamlNodes],
    edges: [{ from_id: 1, to_id: 1 + ID_OFFSET }, ...graphEdges],
  };
}
