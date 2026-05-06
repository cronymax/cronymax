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
      patch: Partial<
        Pick<GraphEdge, "port" | "requires_human_approval" | "label">
      >;
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
      // eslint-disable-next-line @typescript-eslint/no-explicit-any
      let produces: ProducesEntry[] = Array.isArray((n as any).produces)
        ? // eslint-disable-next-line @typescript-eslint/no-explicit-any
          [...((n as any).produces as ProducesEntry[])]
        : [];
      if (produces.length === 0 && config["produces"]) {
        produces = [{ doc_type: config["produces"]!, reviewers: "" }];
      }
      // Strip old config keys.
      delete config["produces"];
      delete config["reviewers"];
      return { ...n, config, produces };
    });

  // Step 2: drain edge.reviewers → source node's matching ProducesEntry.
  const nodeMap = new Map(nodes.map((n) => [n.id, n]));
  const edges: GraphEdge[] = (raw.edges ?? []).map((e) => {
    // eslint-disable-next-line @typescript-eslint/no-explicit-any
    const edgeRevs = (e as any).reviewers as string | undefined;
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
      incoming.forEach((r) => existing.add(r));
      src.produces[entryIdx] = {
        ...src.produces[entryIdx]!,
        reviewers: Array.from(existing).join(","),
      };
    } else {
      src.produces.push({ doc_type: e.port, reviewers: edgeRevs });
    }
    // eslint-disable-next-line @typescript-eslint/no-explicit-any
    const { reviewers: _r, ...rest } = e as any;
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
  return (edges ?? []).filter(
    (e) => nodeIds.has(e.from_id) && nodeIds.has(e.to_id),
  );
}

function reducer(state: State, action: Action): State {
  switch (action.type) {
    case "setFlow": {
      const migrated = migrateFlowSpec(action.spec);
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
        edges: state.edges.filter(
          (e) => e.from_id !== action.id && e.to_id !== action.id,
        ),
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
        nodes: state.nodes.map((n) =>
          n.id === action.id ? { ...n, name: action.name } : n,
        ),
      };
    case "updateNodeConfig":
      return {
        ...state,
        nodes: state.nodes.map((n) =>
          n.id === action.id
            ? { ...n, config: { ...n.config, [action.key]: action.value } }
            : n,
        ),
      };
    case "updateNodeProduces":
      return {
        ...state,
        nodes: state.nodes.map((n) =>
          n.id === action.id ? { ...n, produces: action.produces } : n,
        ),
      };
    case "updateNodePosition":
      return {
        ...state,
        nodes: state.nodes.map((n) =>
          n.id === action.id ? { ...n, x: action.x, y: action.y } : n,
        ),
      };
    case "updateEdge":
      return {
        ...state,
        edges: state.edges.map((e, i) =>
          i === action.index ? { ...e, ...action.patch } : e,
        ),
      };
    case "deleteEdge":
      return {
        ...state,
        edges: state.edges.filter((_, i) => i !== action.index),
        selectedEdgeIndex:
          state.selectedEdgeIndex === action.index
            ? null
            : state.selectedEdgeIndex,
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

export const { Provider, useStore } = createPanelStore<State, Action>(
  reducer,
  initial,
);

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

export function migrateLegacy(
  flows: Record<string, FlowSpec>,
): Record<string, FlowSpec> {
  try {
    const raw = localStorage.getItem(LEGACY_KEY);
    if (!raw) return flows;
    const obj = JSON.parse(raw) as FlowSpec;
    if (!obj || !Array.isArray(obj.nodes)) return flows;
    if (!flows["default"]) {
      flows["default"] = { nodes: obj.nodes, edges: obj.edges || [] };
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
// opens with an empty flow store. It represents the simplest useful
// configuration: a single worker agent with no explicit tool restrictions
// (= the Space defaults, i.e. all registered skills). Users can freely
// rename, reconfigure, or delete it.
export const SEED_CHAT_FLOW: FlowSpec = {
  nodes: [
    {
      id: 1,
      type: "agent",
      name: "Chat",
      x: 80,
      y: 60,
      produces: [],
      // agent_name left blank so the user picks from the inspector once
      // the agent registry is populated. tools defaults to all skills.
      config: {
        agent_name: "Chat",
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
  nodes: [
    {
      id: 1,
      type: "agent",
      name: "pm",
      x: 80,
      y: 120,
      produces: [{ doc_type: "prd", reviewers: "critic" }],
      config: {
        agent_name: "pm",
      },
    },
    {
      id: 2,
      type: "agent",
      name: "rd",
      x: 380,
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
      id: 3,
      type: "agent",
      name: "qa",
      x: 680,
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
    { from_id: 1, to_id: 2, port: "prd", requires_human_approval: true },
    // RD → QA: tech-spec after human + critic + qa-critic approval
    { from_id: 2, to_id: 3, port: "tech-spec", requires_human_approval: true },
    // RD → QA: submit-for-testing handoff (no gate)
    { from_id: 2, to_id: 3, port: "submit-for-testing" },
    // QA → RD: bug reports (max 5 fix cycles)
    { from_id: 3, to_id: 2, port: "bug-report" },
    // RD → QA: patch notes after each fix
    { from_id: 2, to_id: 3, port: "patch-note" },
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
