/**
 * Flow editor panel — agent-centric flow designer.
 *
 * Each node on the canvas represents one Agent placement (a worker or a
 * reviewer drawn from the agent registry). Per-node configuration covers
 * the doc-type the agent produces and the reviewer agents attached to it.
 * Edges carry a typed document (a `port`) downstream and may optionally
 * gate on human approval — matching `FlowEdge` in
 * `app/flow/flow_definition.h`.
 *
 * Drag state is held in a ref to avoid re-rendering on every mousemove;
 * node positions are committed to the store on mouseup.
 */
import {
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
  type CSSProperties,
  type MouseEvent as ReactMouseEvent,
} from "react";
import { bridge } from "@/bridge";
import {
  Provider,
  loadAllFlows,
  saveAllFlows,
  getActiveFlowName,
  setActiveFlowName,
  migrateLegacy,
  syncLegacyKey,
  useStore,
  SEED_CHAT_FLOW,
  leadNodeId,
  type FlowSpec,
  type GraphEdge,
  type GraphNode,
} from "./store";

// Re-export Provider so main.tsx can keep importing it from here if desired.
export { Provider };

// ── constants ─────────────────────────────────────────────────────────────
const NODE_W = 200;
const NODE_H = 72;

const KIND_BG: Record<string, string> = {
  worker: "bg-cronymax-primary/15 border-cronymax-primary/40",
  reviewer: "bg-purple-500/15 border-purple-500/40",
  unknown: "bg-cronymax-float border-cronymax-border",
};

function kindBg(kind: string | undefined): string {
  return KIND_BG[kind ?? "unknown"] ?? KIND_BG.unknown!;
}

// ── helpers ───────────────────────────────────────────────────────────────
function bezierPath(from: GraphNode, to: GraphNode): string {
  const x1 = from.x + NODE_W / 2;
  const y1 = from.y + NODE_H;
  const x2 = to.x + NODE_W / 2;
  const y2 = to.y;
  const cy = (y1 + y2) / 2;
  return `M ${x1} ${y1} C ${x1} ${cy}, ${x2} ${cy}, ${x2} ${y2}`;
}

function midpoint(from: GraphNode, to: GraphNode): { x: number; y: number } {
  return {
    x: (from.x + to.x) / 2 + NODE_W / 2,
    y: (from.y + to.y) / 2 + NODE_H / 2,
  };
}

function reviewerList(node: GraphNode): string[] {
  return (node.config.reviewers ?? "")
    .split(",")
    .map((s) => s.trim())
    .filter(Boolean);
}

function previewLine(node: GraphNode): string {
  const parts: string[] = [];
  if (node.config.produces) parts.push(`→ ${node.config.produces}`);
  const revs = reviewerList(node);
  if (revs.length > 0) parts.push(`reviewers: ${revs.join(", ")}`);
  return parts.join("  ·  ");
}

// ── inspector helpers ─────────────────────────────────────────────────────
function FieldGroup({
  label,
  children,
}: {
  label: string;
  children: React.ReactNode;
}) {
  return (
    <div className="mb-3">
      <div className="mb-1 text-[11px] uppercase tracking-wide text-cronymax-caption">
        {label}
      </div>
      {children}
    </div>
  );
}

const INPUT_CLS =
  "w-full rounded border border-cronymax-border bg-cronymax-base px-2 py-1 text-xs text-cronymax-title outline-none focus:border-cronymax-primary";

export function Flows() {
  // The graph panel exposes its own Provider so its store stays isolated
  // from the agent panel store.
  return (
    <Provider>
      <FlowEditor />
    </Provider>
  );
}
export function FlowEditor() {
  const [state, dispatch] = useStore();
  const [traceOpen, setTraceOpen] = useState(true);
  const [agentPickerOpen, setAgentPickerOpen] = useState<
    null | "worker" | "reviewer" | "any"
  >(null);

  // Drag state lives in a ref + local component state for live position.
  const dragRef = useRef<{
    nodeId: number;
    startX: number;
    startY: number;
    origX: number;
    origY: number;
  } | null>(null);
  const [livePos, setLivePos] = useState<Map<number, { x: number; y: number }>>(
    new Map(),
  );

  const traceLogRef = useRef<HTMLPreElement>(null);

  // ── init: load flows + remote catalogs ──────────────────────────────────
  useEffect(() => {
    const flows = migrateLegacy(loadAllFlows());
    // Seed a built-in "Chat" flow the first time the editor opens on a
    // fresh installation (no flows in localStorage).
    if (Object.keys(flows).length === 0) {
      flows["Chat"] = { ...SEED_CHAT_FLOW };
      saveAllFlows(flows);
    }
    const names = Object.keys(flows).sort();
    let active = getActiveFlowName();
    if (active && !flows[active]) active = "";
    if (!active && names.length > 0) active = names[0]!;
    dispatch({ type: "setFlowNames", names, active });
    dispatch({ type: "setFlowNameInput", value: active });
    if (active && flows[active]) {
      dispatch({ type: "setFlow", spec: flows[active]! });
      setActiveFlowName(active);
    } else {
      dispatch({ type: "setFlow", spec: { nodes: [], edges: [] } });
    }

    // Load agent + doc-type registries from the native bridge.
    // If the registry is empty on first run, auto-seed a default "Chat" agent.
    bridge
      .send("agent.registry.list")
      .then(async (res) => {
        let agents = res.agents ?? [];
        if (agents.length === 0) {
          try {
            await bridge.send("agent.registry.save", {
              name: "Chat",
              kind: "worker",
              llm: "",
              system_prompt: "You are a helpful assistant.",
              memory_namespace: "",
              tools_csv: "",
            });
            const refreshed = await bridge.send("agent.registry.list");
            agents = refreshed.agents ?? [];
          } catch {
            // Seeding failed (e.g. bridge not available); continue with empty catalog.
          }
        }
        dispatch({ type: "setAgentCatalog", agents });
      })
      .catch((err: Error) => {
        // eslint-disable-next-line no-console
        console.warn("[flow] agent.registry.list failed:", err.message);
      });
    bridge
      .send("doc_type.list")
      .then((res) => {
        dispatch({ type: "setDocTypeCatalog", docTypes: res.doc_types ?? [] });
      })
      .catch((err: Error) => {
        // eslint-disable-next-line no-console
        console.warn("[flow] doc_type.list failed:", err.message);
      });
  }, []);

  // ── auto-scroll trace ───────────────────────────────────────────────────
  useEffect(() => {
    const el = traceLogRef.current;
    if (el) el.scrollTop = el.scrollHeight;
  }, [state.trace]);

  // ── drag handlers (document-wide) ───────────────────────────────────────
  useEffect(() => {
    const onMove = (e: MouseEvent) => {
      const d = dragRef.current;
      if (!d) return;
      const x = d.origX + (e.clientX - d.startX);
      const y = d.origY + (e.clientY - d.startY);
      setLivePos((prev) => {
        const next = new Map(prev);
        next.set(d.nodeId, { x, y });
        return next;
      });
    };
    const onUp = () => {
      const d = dragRef.current;
      if (!d) return;
      const pos = livePosRef.current.get(d.nodeId);
      if (pos) {
        dispatch({
          type: "updateNodePosition",
          id: d.nodeId,
          x: pos.x,
          y: pos.y,
        });
      }
      dragRef.current = null;
      setLivePos(new Map());
    };
    document.addEventListener("mousemove", onMove);
    document.addEventListener("mouseup", onUp);
    return () => {
      document.removeEventListener("mousemove", onMove);
      document.removeEventListener("mouseup", onUp);
    };
  }, [dispatch]);

  // Mirror livePos into a ref so the mouseup handler reads the latest value.
  const livePosRef = useRef(livePos);
  useEffect(() => {
    livePosRef.current = livePos;
  }, [livePos]);

  // ── derived ─────────────────────────────────────────────────────────────
  const effectiveNodes = useMemo(() => {
    if (livePos.size === 0) return state.nodes;
    return state.nodes.map((n) => {
      const lp = livePos.get(n.id);
      return lp ? { ...n, x: lp.x, y: lp.y } : n;
    });
  }, [state.nodes, livePos]);

  const selectedNode = useMemo(
    () => state.nodes.find((n) => n.id === state.selectedId) ?? null,
    [state.nodes, state.selectedId],
  );
  const selectedEdge =
    state.selectedEdgeIndex != null
      ? (state.edges[state.selectedEdgeIndex] ?? null)
      : null;

  const workerAgents = useMemo(
    () => state.agentCatalog.filter((a) => a.kind === "worker"),
    [state.agentCatalog],
  );
  const reviewerAgents = useMemo(
    () => state.agentCatalog.filter((a) => a.kind === "reviewer"),
    [state.agentCatalog],
  );

  // ── node operations ─────────────────────────────────────────────────────
  const addAgentNode = useCallback(
    (agentName: string, kind: string) => {
      const id = state.nextId;
      const idx = state.nodes.length;
      const node: GraphNode = {
        id,
        type: "agent",
        name: agentName,
        config: { agent_name: agentName, agent_kind: kind },
        x: 80 + (idx % 5) * 220,
        y: 60 + Math.floor(idx / 5) * 160,
      };
      const prev = [...state.nodes].reverse().find((n) => n.id !== id);
      const edge: GraphEdge | undefined =
        prev && kind !== "reviewer"
          ? { from_id: prev.id, to_id: id, port: "" }
          : undefined;
      dispatch({ type: "addNode", node, nextId: id + 1, edge });
      setAgentPickerOpen(null);
    },
    [state.nodes, state.nextId, dispatch],
  );

  const onNodeMouseDown = useCallback(
    (e: ReactMouseEvent, n: GraphNode) => {
      const target = e.target as HTMLElement;
      if (target.dataset.role === "delete") return;
      e.preventDefault();
      dispatch({ type: "select", id: n.id });
      dragRef.current = {
        nodeId: n.id,
        startX: e.clientX,
        startY: e.clientY,
        origX: n.x,
        origY: n.y,
      };
    },
    [dispatch],
  );

  // ── flows (localStorage; backend persistence not yet wired) ─────────────
  const onSaveFlow = useCallback(() => {
    const name = (
      state.flowNameInput ||
      state.activeFlowName ||
      "default"
    ).trim();
    if (!name) {
      dispatch({ type: "appendTrace", chunk: "✗ Enter a flow name first.\n" });
      return;
    }
    const flows = loadAllFlows();
    const spec: FlowSpec = { nodes: state.nodes, edges: state.edges };
    flows[name] = spec;
    saveAllFlows(flows);
    syncLegacyKey(spec);
    setActiveFlowName(name);
    dispatch({
      type: "setFlowNames",
      names: Object.keys(flows).sort(),
      active: name,
    });
    dispatch({ type: "appendTrace", chunk: `✓ Flow "${name}" saved.\n` });
  }, [
    state.flowNameInput,
    state.activeFlowName,
    state.nodes,
    state.edges,
    dispatch,
  ]);

  const onSelectFlow = useCallback(
    (name: string) => {
      if (!name) return;
      const flows = loadAllFlows();
      const spec = flows[name];
      if (!spec) return;
      setActiveFlowName(name);
      dispatch({ type: "setActiveFlow", name });
      dispatch({ type: "setFlow", spec });
    },
    [dispatch],
  );

  const onDeleteFlow = useCallback(() => {
    const name = state.activeFlowName;
    if (!name) {
      dispatch({ type: "appendTrace", chunk: "✗ No flow selected.\n" });
      return;
    }
    // eslint-disable-next-line no-alert
    if (!confirm(`Delete flow "${name}"?`)) return;
    const flows = loadAllFlows();
    delete flows[name];
    saveAllFlows(flows);
    setActiveFlowName("");
    dispatch({
      type: "setFlowNames",
      names: Object.keys(flows).sort(),
      active: "",
    });
    dispatch({ type: "setFlowNameInput", value: "" });
    dispatch({ type: "setFlow", spec: { nodes: [], edges: [] } });
    dispatch({ type: "appendTrace", chunk: `✓ Flow "${name}" deleted.\n` });
  }, [state.activeFlowName, dispatch]);

  const onClear = useCallback(() => {
    // eslint-disable-next-line no-alert
    if (!confirm("Clear all nodes?")) return;
    dispatch({ type: "clear" });
  }, [dispatch]);

  // ── render ──────────────────────────────────────────────────────────────
  const btnCls =
    "rounded border border-cronymax-border bg-cronymax-base px-2 py-1 text-xs text-cronymax-title hover:bg-cronymax-float";
  const btnDangerCls =
    "rounded border border-red-500/50 bg-red-500/10 px-2 py-1 text-xs text-red-300 hover:bg-red-500/20";

  return (
    <main className="flex h-full flex-col bg-cronymax-base text-cronymax-title">
      {/* Toolbar */}
      <header className="flex flex-wrap items-center gap-2 border-b border-cronymax-border bg-cronymax-float px-3 py-2 text-xs">
        <span className="font-semibold">Flow</span>
        <select
          value={state.activeFlowName}
          onChange={(e) => onSelectFlow(e.target.value)}
          className="rounded border border-cronymax-border bg-cronymax-base px-1.5 py-0.5 text-xs"
          title="Switch flow"
        >
          {state.flowNames.length === 0 ? (
            <option value="">(no saved flows)</option>
          ) : (
            state.flowNames.map((n) => (
              <option key={n} value={n}>
                {n}
              </option>
            ))
          )}
        </select>
        <input
          type="text"
          value={state.flowNameInput}
          onChange={(e) =>
            dispatch({ type: "setFlowNameInput", value: e.target.value })
          }
          placeholder="flow name"
          className="rounded border border-cronymax-border bg-cronymax-base px-1.5 py-0.5 text-xs outline-none focus:border-cronymax-primary"
        />
        <div className="ml-auto flex items-center gap-1.5">
          <button
            type="button"
            onClick={() => setAgentPickerOpen("worker")}
            className={btnCls}
            title="Add a worker agent node"
          >
            + Agent
          </button>
          <button
            type="button"
            onClick={() => setAgentPickerOpen("reviewer")}
            className={btnCls}
            title="Add a reviewer agent node"
          >
            + Reviewer
          </button>
          <span className="mx-1 h-4 w-px bg-cronymax-border" />
          <button type="button" onClick={onSaveFlow} className={btnCls}>
            💾 Save
          </button>
          <button
            type="button"
            onClick={onDeleteFlow}
            className={btnCls}
            title="Delete this flow"
          >
            🗑
          </button>
          <button type="button" onClick={onClear} className={btnDangerCls}>
            Clear
          </button>
        </div>
      </header>

      {/* Body */}
      <div className="flex flex-1 overflow-hidden">
        {/* Canvas */}
        <div
          className="relative flex-1 overflow-auto bg-cronymax-base"
          onClick={(e) => {
            if (e.target === e.currentTarget) {
              dispatch({ type: "select", id: null });
            }
          }}
        >
          {/* Edge SVG fills the canvas. Edge labels capture clicks for selection. */}
          <svg
            className="absolute inset-0 h-full w-full"
            style={{ minWidth: "100%", minHeight: "100%" }}
          >
            {state.edges.map((edge, i) => {
              const from = effectiveNodes.find((n) => n.id === edge.from_id);
              const to = effectiveNodes.find((n) => n.id === edge.to_id);
              if (!from || !to) return null;
              const mid = midpoint(from, to);
              const portLabel = edge.port || "(no doc-type)";
              const gateLabel = edge.requires_human_approval ? " ✋" : "";
              const isSel = state.selectedEdgeIndex === i;
              return (
                <g key={i}>
                  <path
                    d={bezierPath(from, to)}
                    stroke={
                      isSel ? "rgb(124, 158, 255)" : "rgba(124, 124, 140, 0.6)"
                    }
                    strokeWidth={isSel ? 2 : 1.5}
                    fill="none"
                    pointerEvents="none"
                  />
                  <rect
                    x={mid.x - 60}
                    y={mid.y - 11}
                    width={120}
                    height={20}
                    rx={4}
                    fill={isSel ? "rgba(124,158,255,0.18)" : "rgba(0,0,0,0.45)"}
                    stroke={
                      isSel ? "rgb(124, 158, 255)" : "rgba(124,124,140,0.4)"
                    }
                    style={{ cursor: "pointer" }}
                    onClick={(e) => {
                      e.stopPropagation();
                      dispatch({ type: "selectEdge", index: i });
                    }}
                  />
                  <text
                    x={mid.x}
                    y={mid.y + 3}
                    fill={
                      edge.port
                        ? "rgba(224,224,230,0.9)"
                        : "rgba(224,224,230,0.5)"
                    }
                    fontSize={10}
                    textAnchor="middle"
                    pointerEvents="none"
                  >
                    {portLabel}
                    {gateLabel}
                  </text>
                </g>
              );
            })}
          </svg>

          {/* Nodes layer. */}
          <div className="relative" style={{ minWidth: 1200, minHeight: 800 }}>
            {effectiveNodes.map((n) => {
              const isSelected = state.selectedId === n.id;
              const isRunning = state.runningId === n.id;
              const isDone = state.doneId === n.id;
              const style: CSSProperties = {
                position: "absolute",
                left: n.x,
                top: n.y,
                width: NODE_W,
                minHeight: NODE_H,
              };
              const ring = isSelected
                ? "ring-2 ring-cronymax-primary"
                : isRunning
                  ? "ring-2 ring-yellow-400 animate-pulse"
                  : isDone
                    ? "ring-2 ring-green-400"
                    : "";
              const kind = n.config.agent_kind || "unknown";
              const isLead = leadNodeId(state.nodes) === n.id;
              return (
                <div
                  key={n.id}
                  style={style}
                  onMouseDown={(e) => onNodeMouseDown(e, n)}
                  className={
                    "cursor-move select-none rounded-md border p-2 text-xs shadow-sm transition " +
                    kindBg(kind) +
                    " " +
                    ring
                  }
                >
                  <div className="mb-1 flex items-center gap-1.5">
                    <span className="rounded bg-black/30 px-1.5 py-0.5 text-[10px] uppercase tracking-wide">
                      {kind === "reviewer" ? "Reviewer" : "Agent"}
                    </span>
                    {isLead && (
                      <span
                        title="Lead agent: handles unaddressed messages and cannot be deleted."
                        className="rounded bg-cronymax-primary/30 px-1.5 py-0.5 text-[10px] uppercase tracking-wide text-cronymax-primary"
                      >
                        Lead
                      </span>
                    )}
                    <span className="flex-1 truncate font-medium">
                      {n.name}
                    </span>
                    {!isLead && (
                      <button
                        type="button"
                        data-role="delete"
                        onClick={(e) => {
                          e.stopPropagation();
                          dispatch({ type: "deleteNode", id: n.id });
                        }}
                        className="text-cronymax-caption hover:text-red-300"
                        title="Delete"
                      >
                        ×
                      </button>
                    )}
                  </div>
                  <code className="block truncate text-[11px] text-cronymax-caption">
                    {previewLine(n) || "no doc-type / reviewers set"}
                  </code>
                </div>
              );
            })}
          </div>
        </div>

        {/* Inspector */}
        <Inspector
          state={state}
          node={selectedNode}
          edge={selectedEdge}
          edgeIndex={state.selectedEdgeIndex}
          onClose={() => {
            dispatch({ type: "select", id: null });
            dispatch({ type: "selectEdge", index: null });
          }}
          onChangeName={(name) => {
            if (state.selectedId != null)
              dispatch({ type: "updateNodeName", id: state.selectedId, name });
          }}
          onChangeConfig={(key, value) => {
            if (state.selectedId != null)
              dispatch({
                type: "updateNodeConfig",
                id: state.selectedId,
                key,
                value,
              });
          }}
          onChangeEdge={(patch) => {
            if (state.selectedEdgeIndex != null)
              dispatch({
                type: "updateEdge",
                index: state.selectedEdgeIndex,
                patch,
              });
          }}
          onDeleteEdge={() => {
            if (state.selectedEdgeIndex != null)
              dispatch({
                type: "deleteEdge",
                index: state.selectedEdgeIndex,
              });
          }}
        />
      </div>

      {/* Trace bar */}
      <section
        className={
          "border-t border-cronymax-border bg-cronymax-float transition-all " +
          (traceOpen ? "h-40" : "h-7")
        }
      >
        <div className="flex items-center justify-between border-b border-cronymax-border px-3 py-1 text-xs">
          <button
            type="button"
            onClick={() => setTraceOpen((v) => !v)}
            className="text-cronymax-caption hover:text-cronymax-title"
          >
            Trace {traceOpen ? "▾" : "▸"}
          </button>
          <button
            type="button"
            onClick={() => dispatch({ type: "clearTrace" })}
            className="text-cronymax-caption hover:text-cronymax-title"
          >
            Clear
          </button>
        </div>
        {traceOpen && (
          <pre
            ref={traceLogRef}
            className="h-[calc(100%-1.75rem)] overflow-auto whitespace-pre-wrap break-words p-2 font-mono text-[11px] text-cronymax-caption"
          >
            {state.trace}
          </pre>
        )}
      </section>

      {/* Agent picker modal */}
      {agentPickerOpen && (
        <AgentPicker
          mode={agentPickerOpen}
          workers={workerAgents}
          reviewers={reviewerAgents}
          onPick={addAgentNode}
          onClose={() => setAgentPickerOpen(null)}
        />
      )}
    </main>
  );
}

// ── inspector ─────────────────────────────────────────────────────────────
function Inspector({
  state,
  node,
  edge,
  edgeIndex,
  onClose,
  onChangeName,
  onChangeConfig,
  onChangeEdge,
  onDeleteEdge,
}: {
  state: ReturnType<typeof useStore>[0];
  node: GraphNode | null;
  edge: GraphEdge | null;
  edgeIndex: number | null;
  onClose: () => void;
  onChangeName: (name: string) => void;
  onChangeConfig: (key: string, value: string) => void;
  onChangeEdge: (
    patch: Partial<Pick<GraphEdge, "port" | "requires_human_approval">>,
  ) => void;
  onDeleteEdge: () => void;
}) {
  if (edge && edgeIndex != null) {
    return (
      <EdgeInspector
        state={state}
        edge={edge}
        onClose={onClose}
        onChangeEdge={onChangeEdge}
        onDelete={onDeleteEdge}
      />
    );
  }

  if (!node) {
    return (
      <aside className="flex h-full w-[320px] flex-col border-l border-cronymax-border bg-cronymax-float">
        <div className="flex items-center justify-between border-b border-cronymax-border px-3 py-2 text-sm">
          <span>Inspector</span>
        </div>
        <p className="px-3 py-2 text-xs text-cronymax-caption">
          Click a node to edit which Agent it represents, what doc-type it
          produces, and which reviewers should validate that document.
        </p>
        <p className="px-3 py-2 text-xs text-cronymax-caption">
          Click an edge label to set the doc-type carried over the edge or
          require human approval.
        </p>
      </aside>
    );
  }

  const cfg = node.config;
  const kind = cfg.agent_kind || "unknown";
  const reviewers = (cfg.reviewers ?? "")
    .split(",")
    .map((s) => s.trim())
    .filter(Boolean);

  function toggleReviewer(name: string): void {
    const set = new Set(reviewers);
    if (set.has(name)) set.delete(name);
    else set.add(name);
    onChangeConfig("reviewers", Array.from(set).join(","));
  }

  return (
    <aside className="flex h-full w-[320px] flex-col border-l border-cronymax-border bg-cronymax-float">
      <div className="flex items-center justify-between border-b border-cronymax-border px-3 py-2 text-sm">
        <span className="truncate">
          {kind === "reviewer" ? "Reviewer" : "Agent"}: {node.name}
        </span>
        <button
          type="button"
          onClick={onClose}
          className="text-cronymax-caption hover:text-cronymax-title"
        >
          ×
        </button>
      </div>
      <div className="flex-1 overflow-auto px-3 py-2">
        <FieldGroup label="Display Label">
          <input
            className={INPUT_CLS}
            value={node.name}
            onChange={(e) => onChangeName(e.target.value)}
          />
        </FieldGroup>

        <FieldGroup label="Agent">
          <select
            className={INPUT_CLS}
            value={cfg.agent_name ?? ""}
            onChange={(e) => {
              const name = e.target.value;
              const entry = state.agentCatalog.find((a) => a.name === name);
              onChangeConfig("agent_name", name);
              if (entry) onChangeConfig("agent_kind", entry.kind);
            }}
          >
            <option value="">(choose an agent)</option>
            {state.agentCatalog.map((a) => (
              <option key={a.name} value={a.name}>
                {a.name} — {a.kind}
              </option>
            ))}
          </select>
        </FieldGroup>

        {kind !== "reviewer" && (
          <>
            <FieldGroup label="Produces (doc-type)">
              <select
                className={INPUT_CLS}
                value={cfg.produces ?? ""}
                onChange={(e) => onChangeConfig("produces", e.target.value)}
              >
                <option value="">(no document)</option>
                {state.docTypeCatalog.map((d) => (
                  <option key={d.name} value={d.name}>
                    {d.display_name} ({d.name})
                  </option>
                ))}
              </select>
            </FieldGroup>

            <FieldGroup label="Reviewers">
              {state.agentCatalog.filter((a) => a.kind === "reviewer")
                .length === 0 ? (
                <div className="text-[11px] text-cronymax-caption">
                  No reviewer agents registered.
                </div>
              ) : (
                <div className="flex flex-col gap-1">
                  {state.agentCatalog
                    .filter((a) => a.kind === "reviewer")
                    .map((a) => (
                      <label
                        key={a.name}
                        className="flex items-center gap-2 text-xs"
                      >
                        <input
                          type="checkbox"
                          checked={reviewers.includes(a.name)}
                          onChange={() => toggleReviewer(a.name)}
                        />
                        <span>{a.name}</span>
                        <span className="text-[10px] text-cronymax-caption">
                          {a.llm}
                        </span>
                      </label>
                    ))}
                </div>
              )}
            </FieldGroup>
          </>
        )}

        <div className="mt-4 text-[11px] text-cronymax-caption">
          Node #{node.id} · kind={kind}
        </div>
      </div>
    </aside>
  );
}

function EdgeInspector({
  state,
  edge,
  onClose,
  onChangeEdge,
  onDelete,
}: {
  state: ReturnType<typeof useStore>[0];
  edge: GraphEdge;
  onClose: () => void;
  onChangeEdge: (
    patch: Partial<Pick<GraphEdge, "port" | "requires_human_approval">>,
  ) => void;
  onDelete: () => void;
}) {
  const from = state.nodes.find((n) => n.id === edge.from_id);
  const to = state.nodes.find((n) => n.id === edge.to_id);
  return (
    <aside className="flex h-full w-[320px] flex-col border-l border-cronymax-border bg-cronymax-float">
      <div className="flex items-center justify-between border-b border-cronymax-border px-3 py-2 text-sm">
        <span className="truncate">Edge</span>
        <button
          type="button"
          onClick={onClose}
          className="text-cronymax-caption hover:text-cronymax-title"
        >
          ×
        </button>
      </div>
      <div className="flex-1 overflow-auto px-3 py-2">
        <FieldGroup label="From → To">
          <div className="text-xs">
            {from?.name ?? `#${edge.from_id}`} → {to?.name ?? `#${edge.to_id}`}
          </div>
        </FieldGroup>

        <FieldGroup label="Doc-type carried (port)">
          <select
            className={INPUT_CLS}
            value={edge.port ?? ""}
            onChange={(e) => onChangeEdge({ port: e.target.value })}
          >
            <option value="">(no document)</option>
            {state.docTypeCatalog.map((d) => (
              <option key={d.name} value={d.name}>
                {d.display_name} ({d.name})
              </option>
            ))}
          </select>
        </FieldGroup>

        <FieldGroup label="Approval">
          <label className="flex items-center gap-2 text-xs">
            <input
              type="checkbox"
              checked={!!edge.requires_human_approval}
              onChange={(e) =>
                onChangeEdge({ requires_human_approval: e.target.checked })
              }
            />
            <span>Require human approval before transition</span>
          </label>
        </FieldGroup>

        <button
          type="button"
          onClick={onDelete}
          className="mt-4 rounded border border-red-500/50 bg-red-500/10 px-2 py-1 text-xs text-red-300 hover:bg-red-500/20"
        >
          Delete edge
        </button>
      </div>
    </aside>
  );
}

// ── agent picker modal ────────────────────────────────────────────────────
function AgentPicker({
  mode,
  workers,
  reviewers,
  onPick,
  onClose,
}: {
  mode: "worker" | "reviewer" | "any";
  workers: { name: string; kind: string; llm: string }[];
  reviewers: { name: string; kind: string; llm: string }[];
  onPick: (name: string, kind: string) => void;
  onClose: () => void;
}) {
  const list =
    mode === "worker"
      ? workers
      : mode === "reviewer"
        ? reviewers
        : [...workers, ...reviewers];
  const title =
    mode === "worker"
      ? "Add Agent"
      : mode === "reviewer"
        ? "Add Reviewer"
        : "Add Agent";

  return (
    <div
      className="fixed inset-0 z-50 flex items-center justify-center bg-black/40"
      onClick={onClose}
    >
      <div
        className="w-[420px] max-h-[70vh] overflow-auto rounded-md border border-cronymax-border bg-cronymax-float p-3 shadow-xl"
        onClick={(e) => e.stopPropagation()}
      >
        <div className="mb-2 flex items-center justify-between">
          <span className="text-sm font-semibold">{title}</span>
          <button
            type="button"
            onClick={onClose}
            className="text-cronymax-caption hover:text-cronymax-title"
          >
            ×
          </button>
        </div>
        {list.length === 0 ? (
          <p className="text-xs text-cronymax-caption">
            No matching agents are registered. Define agents under your
            workspace's <code>agents/</code> directory and reload.
          </p>
        ) : (
          <ul className="flex flex-col gap-1">
            {list.map((a) => (
              <li key={a.name}>
                <button
                  type="button"
                  onClick={() => onPick(a.name, a.kind)}
                  className="flex w-full items-center justify-between rounded border border-cronymax-border bg-cronymax-base px-2 py-1.5 text-left text-xs hover:bg-cronymax-float"
                >
                  <span className="font-medium">{a.name}</span>
                  <span className="text-[10px] text-cronymax-caption">
                    {a.kind} · {a.llm}
                  </span>
                </button>
              </li>
            ))}
          </ul>
        )}
      </div>
    </div>
  );
}
