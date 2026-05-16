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
  type CSSProperties,
  type MouseEvent as ReactMouseEvent,
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
} from "react";
import { Icon } from "@/components/Icon";
import { agentRegistry, docType, flow, flowRun } from "@/shells/runtime";
import {
  type FlowSpec,
  flowSpecFromDef,
  type GraphEdge,
  type GraphNode,
  getActiveFlowName,
  leadNodeId,
  loadAllFlows,
  migrateLegacy,
  type ProducesEntry,
  Provider,
  SEED_CHAT_FLOW,
  SEED_SOFTWARE_DEV_CYCLE_FLOW,
  saveAllFlows,
  setActiveFlowName,
  syncLegacyKey,
  useStore,
} from "./store";

// Re-export Provider so main.tsx can keep importing it from here if desired.
export { Provider };

// ── constants ─────────────────────────────────────────────────────────────
const NODE_W = 200;
const NODE_H = 72;

/** Uniform node style — all agents look the same regardless of role. */
const NODE_BG_CLS = "bg-cronymax-primary/15 border-cronymax-primary/40";

// ── edge routing helpers ─────────────────────────────────────────────────
/**
 * Build per-edge stagger offsets so parallel edges between the same pair of
 * nodes spread apart instead of overlapping.
 */
function buildEdgeOffsets(edges: GraphEdge[]): number[] {
  const groupMap = new Map<string, number[]>();
  edges.forEach((e, i) => {
    const key = [Math.min(e.from_id, e.to_id), Math.max(e.from_id, e.to_id)].join("-");
    const grp = groupMap.get(key) ?? [];
    grp.push(i);
    groupMap.set(key, grp);
  });
  const offsets = new Array<number>(edges.length).fill(0);
  groupMap.forEach((group) => {
    const n = group.length;
    const step = 14;
    group.forEach((idx, pos) => {
      offsets[idx] = (pos - (n - 1) / 2) * step;
    });
  });
  return offsets;
}

/**
 * SVG path `d` for one edge.
 * - Forward (to is to the right): right-centre of from → left-centre of to.
 * - Backward: arcs below both nodes to keep feedback loops visually separate.
 * `vOffset` staggers parallel edges.
 */
function edgePath(from: GraphNode, to: GraphNode, vOffset: number): string {
  const isForward = to.x + NODE_W / 2 >= from.x + NODE_W / 2;
  if (isForward) {
    const x1 = from.x + NODE_W;
    const y1 = from.y + NODE_H / 2 + vOffset;
    const x2 = to.x;
    const y2 = to.y + NODE_H / 2 + vOffset;
    const gap = x2 - x1;
    const cx = Math.max(Math.abs(gap) * 0.5, 60);
    return `M ${x1},${y1} C ${x1 + cx},${y1} ${x2 - cx},${y2} ${x2},${y2}`;
  } else {
    const x1 = from.x + NODE_W / 2 + vOffset;
    const y1 = from.y + NODE_H;
    const x2 = to.x + NODE_W / 2 + vOffset;
    const y2 = to.y + NODE_H;
    const depth = 60 + Math.abs(vOffset) * 2;
    const arcY = Math.max(from.y, to.y) + NODE_H + depth;
    return `M ${x1},${y1} C ${x1},${arcY} ${x2},${arcY} ${x2},${y2}`;
  }
}

/** Label anchor position above the visual midpoint of the edge. */
function edgeLabelPos(from: GraphNode, to: GraphNode, vOffset: number): { x: number; y: number } {
  const isForward = to.x + NODE_W / 2 >= from.x + NODE_W / 2;
  if (isForward) {
    const x1 = from.x + NODE_W;
    const y1 = from.y + NODE_H / 2 + vOffset;
    const x2 = to.x;
    const y2 = to.y + NODE_H / 2 + vOffset;
    return { x: (x1 + x2) / 2, y: (y1 + y2) / 2 };
  } else {
    const x1 = from.x + NODE_W / 2 + vOffset;
    const x2 = to.x + NODE_W / 2 + vOffset;
    const depth = 60 + Math.abs(vOffset) * 2;
    const arcY = Math.max(from.y, to.y) + NODE_H + depth;
    return { x: (x1 + x2) / 2, y: arcY - 6 };
  }
}

function previewLine(node: GraphNode): string {
  if (!node.produces || node.produces.length === 0) return "";
  return node.produces.map((p) => `→ ${p.doc_type || "?"}`).join("  ·  ");
}

// ── inspector helpers ─────────────────────────────────────────────────────
/**
 * A compact pill-trigger dropdown that shows a checkbox list of options.
 * Used for reviewer selection — keeps the inspector dense without showing
 * a flat long list of checkboxes.
 */
function CheckboxDropdown({
  label,
  options,
  selected,
  onToggle,
}: {
  label: string;
  options: { value: string; label: string }[];
  selected: string[];
  onToggle: (value: string) => void;
}) {
  const [open, setOpen] = useState(false);
  const ref = useRef<HTMLDivElement>(null);
  useEffect(() => {
    if (!open) return;
    function handler(e: MouseEvent) {
      if (ref.current && !ref.current.contains(e.target as Node)) {
        setOpen(false);
      }
    }
    document.addEventListener("mousedown", handler);
    return () => document.removeEventListener("mousedown", handler);
  }, [open]);

  const displayText =
    selected.length === 0
      ? label
      : selected.length <= 2
        ? selected.join(", ")
        : `${selected.slice(0, 2).join(", ")} +${selected.length - 2}`;

  return (
    <div ref={ref} className="relative">
      <button
        type="button"
        onClick={() => setOpen((v) => !v)}
        className={
          "flex w-full items-center justify-between rounded border px-2 py-1 text-xs " +
          (selected.length > 0
            ? "border-cronymax-primary/60 bg-cronymax-primary/10 text-cronymax-title"
            : "border-cronymax-border bg-cronymax-base text-cronymax-caption hover:text-cronymax-title")
        }
      >
        <span className="truncate">{displayText}</span>
        <Icon name={open ? "chevron-up" : "chevron-down"} size={10} aria-hidden="true" />
      </button>
      {open && options.length > 0 && (
        <div className="absolute right-0 z-20 mt-0.5 min-w-[160px] rounded border border-cronymax-border bg-cronymax-float p-1 shadow-lg">
          {options.map((opt) => (
            <label
              key={opt.value}
              className="flex cursor-pointer items-center gap-2 rounded px-2 py-1 text-xs hover:bg-cronymax-base"
            >
              <input
                type="checkbox"
                checked={selected.includes(opt.value)}
                onChange={() => onToggle(opt.value)}
                className="shrink-0"
              />
              <span>{opt.label}</span>
            </label>
          ))}
        </div>
      )}
    </div>
  );
}

function FieldGroup({ label, children }: { label: string; children: React.ReactNode }) {
  return (
    <div className="mb-3">
      <div className="mb-1 text-[11px] uppercase tracking-wide text-cronymax-caption">{label}</div>
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
  const [agentPickerOpen, setAgentPickerOpen] = useState(false);
  const [inspectorOpen, setInspectorOpen] = useState(true);
  // Active flow run state — set when a run is started, cleared on cancel.
  const [activeRunId, setActiveRunId] = useState<string | null>(null);

  // Drag state lives in a ref + local component state for live position.
  const dragRef = useRef<{
    nodeId: number;
    startX: number;
    startY: number;
    origX: number;
    origY: number;
  } | null>(null);
  const [livePos, setLivePos] = useState<Map<number, { x: number; y: number }>>(new Map());

  const traceLogRef = useRef<HTMLPreElement>(null);

  // ── init: load flows + remote catalogs ──────────────────────────────────
  useEffect(() => {
    const flows = migrateLegacy(loadAllFlows());
    let needsSave = false;
    // Seed the "Chat" flow on first install (empty store).
    if (Object.keys(flows).length === 0) {
      flows.Chat = { ...SEED_CHAT_FLOW };
      needsSave = true;
    }
    // Always ensure the built-in software-dev-cycle preset is present,
    // including for users who already have other flows.
    if (!flows["software-dev-cycle"]) {
      flows["software-dev-cycle"] = { ...SEED_SOFTWARE_DEV_CYCLE_FLOW };
      needsSave = true;
    }
    if (needsSave) saveAllFlows(flows);
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

    // Merge workspace flows from the native runtime (async).
    // Flows already in localStorage are kept as-is; workspace-only flows
    // are converted from their YAML definition and added to localStorage.
    flow
      .list()
      .then(async (res) => {
        const wsFlows = (res as { flows?: { id: string; name: string; builtin?: boolean }[] }).flows ?? [];
        const current = loadAllFlows();
        let changed = false;
        for (const meta of wsFlows) {
          if (meta.builtin) continue; // skip bundle presets
          if (current[meta.id]) continue; // workspace copy already in localStorage
          try {
            const def = (await flow.load(meta.id)) as Parameters<typeof flowSpecFromDef>[0];
            current[meta.id] = flowSpecFromDef(def);
            changed = true;
          } catch {
            // Ignore unloadable flows
          }
        }
        if (changed) {
          saveAllFlows(current);
          const allNames = Object.keys(current).sort();
          const currentActive = getActiveFlowName();
          dispatch({
            type: "setFlowNames",
            names: allNames,
            active: currentActive || allNames[0] || "",
          });
        }
      })
      .catch(() => {
        /* runtime not available (web preview mode) */
      });

    // Load agent + doc-type registries from the native bridge.
    // If the registry is empty on first run, auto-seed a default "Chat" agent.
    agentRegistry
      .list()
      .then(async (res) => {
        let agents = res.agents ?? [];
        if (agents.length === 0) {
          try {
            await agentRegistry.save({
              name: "Chat",
              llm: "",
              system_prompt: "You are a helpful assistant.",
              memory_namespace: "",
              tools_csv: "",
            });
            const refreshed = await agentRegistry.list();
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
    docType
      .list()
      .then((res) => {
        dispatch({
          type: "setDocTypeCatalog",
          docTypes:
            (res.doc_types as Array<{
              name: string;
              display_name: string;
              user_defined: boolean;
            }>) ?? [],
        });
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

  // Canvas dimensions: always at least the viewport; grow to fit all nodes.
  const canvasSize = useMemo(() => {
    const PAD = 120;
    let w = 1200;
    let h = 700;
    for (const n of effectiveNodes) {
      w = Math.max(w, n.x + NODE_W + PAD);
      h = Math.max(h, n.y + NODE_H + PAD);
    }
    return { width: w, height: h };
  }, [effectiveNodes]);

  const selectedNode = useMemo(
    () => state.nodes.find((n) => n.id === state.selectedId) ?? null,
    [state.nodes, state.selectedId],
  );
  const selectedEdge = state.selectedEdgeIndex != null ? (state.edges[state.selectedEdgeIndex] ?? null) : null;

  // ── node operations ─────────────────────────────────────────────────────
  const edgeOffsets = useMemo(() => buildEdgeOffsets(state.edges), [state.edges]);

  const addAgentNode = useCallback(
    (agentName: string) => {
      const id = state.nextId;
      const idx = state.nodes.length;
      const node: GraphNode = {
        id,
        type: "agent",
        name: agentName,
        produces: [],
        config: { agent_name: agentName },
        x: 80 + (idx % 5) * 220,
        y: 60 + Math.floor(idx / 5) * 160,
      };
      const prev = [...state.nodes].reverse().find((n) => n.id !== id);
      // Auto-connect any new agent to the previous node regardless of kind.
      const edge: GraphEdge | undefined = prev ? { from_id: prev.id, to_id: id, port: "" } : undefined;
      dispatch({ type: "addNode", node, nextId: id + 1, edge });
      setAgentPickerOpen(false);
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
    const name = (state.flowNameInput || state.activeFlowName || "default").trim();
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
  }, [state.flowNameInput, state.activeFlowName, state.nodes, state.edges, dispatch]);

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
          onChange={(e) => dispatch({ type: "setFlowNameInput", value: e.target.value })}
          placeholder="flow name"
          className="rounded border border-cronymax-border bg-cronymax-base px-1.5 py-0.5 text-xs outline-none focus:border-cronymax-primary"
        />
        <div className="ml-auto flex items-center gap-1.5">
          <button type="button" onClick={() => setAgentPickerOpen(true)} className={btnCls} title="Add a node">
            + Node
          </button>
          <span className="mx-1 h-4 w-px bg-cronymax-border" />
          <button type="button" onClick={onSaveFlow} className={`${btnCls} inline-flex items-center gap-1`}>
            <Icon name="save" size={12} aria-hidden="true" /> Save
          </button>
          <button
            type="button"
            onClick={onDeleteFlow}
            className={`${btnCls} inline-flex items-center gap-1`}
            title="Delete this flow"
            aria-label="Delete this flow"
          >
            <Icon name="trash" size={12} aria-hidden="true" />
          </button>
          <button type="button" onClick={onClear} className={btnDangerCls}>
            Clear
          </button>
          <span className="mx-1 h-4 w-px bg-cronymax-border" />
          {activeRunId && (
            <>
              <span className="text-cronymax-description">
                run: <code className="font-mono text-[10px]">{activeRunId.slice(0, 8)}…</code>
              </span>
              <button
                type="button"
                onClick={async () => {
                  try {
                    await flowRun.cancel(activeRunId);
                  } catch {
                    // ignore
                  }
                  setActiveRunId(null);
                  dispatch({ type: "setRunning", running: false });
                }}
                className={`${btnDangerCls} inline-flex items-center gap-1`}
              >
                Cancel
              </button>
            </>
          )}
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
            className="absolute inset-0 overflow-visible"
            style={{ width: canvasSize.width, height: canvasSize.height }}
          >
            <defs>
              <marker id="arrowhead" viewBox="0 0 8 8" refX="7" refY="4" markerWidth="5" markerHeight="5" orient="auto">
                <path d="M0,0 L8,4 L0,8 Z" fill="rgba(124,124,140,0.75)" />
              </marker>
              <marker
                id="arrowhead-sel"
                viewBox="0 0 8 8"
                refX="7"
                refY="4"
                markerWidth="5"
                markerHeight="5"
                orient="auto"
              >
                <path d="M0,0 L8,4 L0,8 Z" fill="rgb(124,158,255)" />
              </marker>
            </defs>
            {state.edges.map((edge, i) => {
              const from = effectiveNodes.find((n) => n.id === edge.from_id);
              const to = effectiveNodes.find((n) => n.id === edge.to_id);
              if (!from || !to) return null;
              const vOff = edgeOffsets[i] ?? 0;
              const lp = edgeLabelPos(from, to, vOff);
              const portLabel = edge.port || "(no doc-type)";
              const gateLabel = edge.requires_human_approval ? " ✋" : "";
              const sourceNode = effectiveNodes.find((n) => n.id === edge.from_id);
              const producesEntry = sourceNode?.produces?.find((p) => p.doc_type === edge.port);
              const revList = (producesEntry?.reviewers ?? "")
                .split(",")
                .map((s) => s.trim())
                .filter(Boolean);
              const revLabel = revList.length > 0 ? revList.join(", ") : null;
              const boxH = revLabel ? 28 : 18;
              const isSel = state.selectedEdgeIndex === i;
              const strokeColor = isSel ? "rgb(124,158,255)" : "rgba(124,124,140,0.65)";
              return (
                <g key={i}>
                  <path
                    d={edgePath(from, to, vOff)}
                    stroke={strokeColor}
                    strokeWidth={isSel ? 2 : 1.5}
                    fill="none"
                    markerEnd={isSel ? "url(#arrowhead-sel)" : "url(#arrowhead)"}
                    pointerEvents="none"
                  />
                  <rect
                    x={lp.x - 52}
                    y={lp.y - 10}
                    width={104}
                    height={boxH}
                    rx={3}
                    fill={isSel ? "rgba(124,158,255,0.18)" : "rgba(0,0,0,0.5)"}
                    stroke={isSel ? "rgb(124,158,255)" : "rgba(124,124,140,0.35)"}
                    style={{ cursor: "pointer" }}
                    onClick={(e) => {
                      e.stopPropagation();
                      dispatch({ type: "selectEdge", index: i });
                    }}
                  />
                  <text
                    x={lp.x}
                    y={lp.y + 3}
                    fill={edge.port ? "rgba(224,224,230,0.9)" : "rgba(224,224,230,0.45)"}
                    fontSize={10}
                    textAnchor="middle"
                    pointerEvents="none"
                  >
                    {portLabel}
                    {gateLabel}
                  </text>
                  {revLabel && (
                    <text
                      x={lp.x}
                      y={lp.y + 14}
                      fill="rgba(180,180,210,0.65)"
                      fontSize={9}
                      textAnchor="middle"
                      pointerEvents="none"
                    >
                      👁 {revLabel}
                    </text>
                  )}
                </g>
              );
            })}
          </svg>

          {/* Nodes layer. */}
          <div className="relative" style={{ width: canvasSize.width, height: canvasSize.height }}>
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
              const isLead = leadNodeId(state.nodes) === n.id;
              return (
                <div
                  key={n.id}
                  style={style}
                  onMouseDown={(e) => onNodeMouseDown(e, n)}
                  className={
                    "cursor-move select-none rounded-md border p-2 text-xs shadow-sm transition " +
                    NODE_BG_CLS +
                    " " +
                    ring
                  }
                >
                  <div className="mb-1 flex items-center gap-1.5">
                    <span className="rounded bg-black/30 px-1.5 py-0.5 text-[10px] uppercase tracking-wide">Agent</span>
                    {isLead && (
                      <span
                        title="Lead agent: handles unaddressed messages and cannot be deleted."
                        className="rounded bg-cronymax-primary/30 px-1.5 py-0.5 text-[10px] uppercase tracking-wide text-cronymax-primary"
                      >
                        Lead
                      </span>
                    )}
                    <span className="flex-1 truncate font-medium">{n.name}</span>
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
                        aria-label="Delete"
                      >
                        <Icon name="close" size={12} aria-hidden="true" />
                      </button>
                    )}
                  </div>
                  <code className="block truncate text-[11px] text-cronymax-caption">
                    {previewLine(n) || "no doc-type set"}
                  </code>
                </div>
              );
            })}
          </div>
        </div>

        {/* Inspector */}
        {inspectorOpen ? (
          <Inspector
            state={state}
            node={selectedNode}
            edge={selectedEdge}
            edgeIndex={state.selectedEdgeIndex}
            onToggleCollapse={() => setInspectorOpen(false)}
            onClose={() => {
              dispatch({ type: "select", id: null });
              dispatch({ type: "selectEdge", index: null });
            }}
            onChangeName={(name) => {
              if (state.selectedId != null)
                dispatch({
                  type: "updateNodeName",
                  id: state.selectedId,
                  name,
                });
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
            onChangeProduces={(produces) => {
              if (state.selectedId != null)
                dispatch({
                  type: "updateNodeProduces",
                  id: state.selectedId,
                  produces,
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
        ) : (
          <div className="flex h-full w-7 shrink-0 flex-col items-center border-l border-cronymax-border bg-cronymax-float">
            <button
              type="button"
              onClick={() => setInspectorOpen(true)}
              className="mt-2 rounded p-1 text-cronymax-caption hover:text-cronymax-title"
              title="Expand inspector"
              aria-label="Expand inspector"
            >
              <Icon name="chevron-left" size={12} aria-hidden="true" />
            </button>
          </div>
        )}
      </div>

      {/* Trace bar */}
      <section
        className={`border-t border-cronymax-border bg-cronymax-float transition-all ${traceOpen ? "h-40" : "h-7"}`}
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
        <AgentPicker agents={state.agentCatalog} onPick={addAgentNode} onClose={() => setAgentPickerOpen(false)} />
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
  onToggleCollapse,
  onClose,
  onChangeName,
  onChangeConfig,
  onChangeProduces,
  onChangeEdge,
  onDeleteEdge,
}: {
  state: ReturnType<typeof useStore>[0];
  node: GraphNode | null;
  edge: GraphEdge | null;
  edgeIndex: number | null;
  onToggleCollapse: () => void;
  onClose: () => void;
  onChangeName: (name: string) => void;
  onChangeConfig: (key: string, value: string) => void;
  onChangeProduces: (produces: ProducesEntry[]) => void;
  onChangeEdge: (patch: Partial<Pick<GraphEdge, "port" | "requires_human_approval">>) => void;
  onDeleteEdge: () => void;
}) {
  if (edge && edgeIndex != null) {
    return (
      <EdgeInspector
        state={state}
        edge={edge}
        onToggleCollapse={onToggleCollapse}
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
          <button
            type="button"
            onClick={onToggleCollapse}
            className="text-cronymax-caption hover:text-cronymax-title"
            aria-label="Collapse inspector"
          >
            <Icon name="chevron-right" size={12} aria-hidden="true" />
          </button>
        </div>
        <p className="px-3 py-2 text-xs text-cronymax-caption">
          Click a node to edit the Agent it represents and its produced documents (each with its own reviewer list).
        </p>
        <p className="px-3 py-2 text-xs text-cronymax-caption">
          Click an edge label to set the doc-type carried over the edge or require human approval.
        </p>
      </aside>
    );
  }

  const cfg = node.config;
  const produces = node.produces ?? [];

  function updateEntry(idx: number, patch: Partial<ProducesEntry>): void {
    const next = produces.map((p, i) => (i === idx ? { ...p, ...patch } : p));
    onChangeProduces(next);
  }

  function removeEntry(idx: number): void {
    onChangeProduces(produces.filter((_, i) => i !== idx));
  }

  function addEntry(): void {
    onChangeProduces([...produces, { doc_type: "", reviewers: "" }]);
  }

  function toggleEntryReviewer(idx: number, agentName: string): void {
    const entry = produces[idx];
    if (!entry) return;
    const set = new Set(
      entry.reviewers
        .split(",")
        .map((s) => s.trim())
        .filter(Boolean),
    );
    if (set.has(agentName)) set.delete(agentName);
    else set.add(agentName);
    updateEntry(idx, { reviewers: Array.from(set).join(",") });
  }

  function batchToggleReviewer(agentName: string, checked: boolean): void {
    onChangeProduces(
      produces.map((p) => {
        const set = new Set(
          p.reviewers
            .split(",")
            .map((s) => s.trim())
            .filter(Boolean),
        );
        if (checked) set.add(agentName);
        else set.delete(agentName);
        return { ...p, reviewers: Array.from(set).join(",") };
      }),
    );
  }

  return (
    <aside className="flex h-full w-[320px] flex-col border-l border-cronymax-border bg-cronymax-float">
      <div className="flex items-center justify-between border-b border-cronymax-border px-3 py-2 text-sm">
        <span className="truncate">Agent: {node.name}</span>
        <div className="flex items-center gap-1">
          <button
            type="button"
            onClick={onToggleCollapse}
            className="text-cronymax-caption hover:text-cronymax-title"
            aria-label="Collapse inspector"
          >
            <Icon name="chevron-right" size={12} aria-hidden="true" />
          </button>
          <button
            type="button"
            onClick={onClose}
            className="text-cronymax-caption hover:text-cronymax-title"
            aria-label="Close"
          >
            <Icon name="close" size={12} aria-hidden="true" />
          </button>
        </div>
      </div>
      <div className="flex-1 overflow-auto px-3 py-2">
        <FieldGroup label="Display Label">
          <input className={INPUT_CLS} value={node.name} onChange={(e) => onChangeName(e.target.value)} />
        </FieldGroup>

        <FieldGroup label="Agent">
          <select
            className={INPUT_CLS}
            value={cfg.agent_name ?? ""}
            onChange={(e) => {
              const name = e.target.value;
              onChangeConfig("agent_name", name);
            }}
          >
            <option value="">(choose an agent)</option>
            {state.agentCatalog.map((a) => (
              <option key={a.name} value={a.name}>
                {a.name}
              </option>
            ))}
          </select>
        </FieldGroup>

        <FieldGroup label="Produces">
          <div className="flex flex-col gap-2">
            {produces.map((entry, idx) => (
              <div key={idx} className="rounded border border-cronymax-border/60 bg-cronymax-base p-2">
                {/* Doc-type row */}
                <div className="mb-1.5 flex items-center gap-1">
                  <select
                    className={`${INPUT_CLS} flex-1`}
                    value={entry.doc_type}
                    onChange={(e) => updateEntry(idx, { doc_type: e.target.value })}
                  >
                    <option value="">(choose doc-type)</option>
                    {entry.doc_type && !state.docTypeCatalog.find((d) => d.name === entry.doc_type) && (
                      <option value={entry.doc_type}>{entry.doc_type}</option>
                    )}
                    {state.docTypeCatalog.map((d) => (
                      <option key={d.name} value={d.name}>
                        {d.display_name} ({d.name})
                      </option>
                    ))}
                  </select>
                  <button
                    type="button"
                    onClick={() => removeEntry(idx)}
                    className="shrink-0 text-cronymax-caption hover:text-red-300"
                    aria-label="Remove"
                  >
                    <Icon name="close" size={11} aria-hidden="true" />
                  </button>
                </div>
                {/* Per-entry reviewers */}
                {state.agentCatalog.length > 0 && (
                  <>
                    <div className="mb-0.5 text-[10px] uppercase tracking-wide text-cronymax-caption">Reviewers</div>
                    <CheckboxDropdown
                      label="(no reviewers)"
                      options={state.agentCatalog.map((a) => ({
                        value: a.name,
                        label: a.name,
                      }))}
                      selected={(entry.reviewers ?? "")
                        .split(",")
                        .map((s) => s.trim())
                        .filter(Boolean)}
                      onToggle={(agentName) => toggleEntryReviewer(idx, agentName)}
                    />
                  </>
                )}
              </div>
            ))}
            <button
              type="button"
              onClick={addEntry}
              className="rounded border border-dashed border-cronymax-border/60 py-1 text-[11px] text-cronymax-caption hover:border-cronymax-primary hover:text-cronymax-primary"
            >
              + Add Document
            </button>
          </div>
        </FieldGroup>

        {produces.length > 1 && state.agentCatalog.length > 0 && (
          <FieldGroup label="Batch Reviewers">
            <p className="mb-1.5 text-[11px] text-cronymax-caption">Toggle a reviewer across all documents at once:</p>
            <div className="flex flex-wrap gap-2">
              {state.agentCatalog.map((a) => {
                const allHave =
                  produces.length > 0 &&
                  produces.every((p) =>
                    p.reviewers
                      .split(",")
                      .map((s) => s.trim())
                      .includes(a.name),
                  );
                const someHave = produces.some((p) =>
                  p.reviewers
                    .split(",")
                    .map((s) => s.trim())
                    .includes(a.name),
                );
                return (
                  <label
                    key={a.name}
                    className={
                      "flex cursor-pointer items-center gap-1 rounded px-2 py-0.5 text-[11px] " +
                      (allHave
                        ? "bg-cronymax-primary/20 text-cronymax-title"
                        : someHave
                          ? "bg-cronymax-primary/10 text-cronymax-caption"
                          : "bg-cronymax-base text-cronymax-caption hover:text-cronymax-title")
                    }
                  >
                    <input
                      type="checkbox"
                      checked={allHave}
                      ref={(el) => {
                        if (el) el.indeterminate = someHave && !allHave;
                      }}
                      onChange={(e) => batchToggleReviewer(a.name, e.target.checked)}
                    />
                    <span>{a.name}</span>
                  </label>
                );
              })}
            </div>
          </FieldGroup>
        )}

        <div className="mt-4 text-[11px] text-cronymax-caption">Node #{node.id}</div>
      </div>
    </aside>
  );
}

function EdgeInspector({
  state,
  edge,
  onToggleCollapse,
  onClose,
  onChangeEdge,
  onDelete,
}: {
  state: ReturnType<typeof useStore>[0];
  edge: GraphEdge;
  onToggleCollapse: () => void;
  onClose: () => void;
  onChangeEdge: (patch: Partial<Pick<GraphEdge, "port" | "requires_human_approval">>) => void;
  onDelete: () => void;
}) {
  const from = state.nodes.find((n) => n.id === edge.from_id);
  const to = state.nodes.find((n) => n.id === edge.to_id);
  return (
    <aside className="flex h-full w-[320px] flex-col border-l border-cronymax-border bg-cronymax-float">
      <div className="flex items-center justify-between border-b border-cronymax-border px-3 py-2 text-sm">
        <span className="truncate">Edge</span>
        <div className="flex items-center gap-1">
          <button
            type="button"
            onClick={onToggleCollapse}
            className="text-cronymax-caption hover:text-cronymax-title"
            aria-label="Collapse inspector"
          >
            <Icon name="chevron-right" size={12} aria-hidden="true" />
          </button>
          <button
            type="button"
            onClick={onClose}
            className="text-cronymax-caption hover:text-cronymax-title"
            aria-label="Close"
          >
            <Icon name="close" size={12} aria-hidden="true" />
          </button>
        </div>
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
            {edge.port && !state.docTypeCatalog.find((d) => d.name === edge.port) && (
              <option value={edge.port}>{edge.port}</option>
            )}
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
              onChange={(e) => onChangeEdge({ requires_human_approval: e.target.checked })}
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
  agents,
  onPick,
  onClose,
}: {
  agents: { name: string; llm: string }[];
  onPick: (name: string) => void;
  onClose: () => void;
}) {
  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/40" onClick={onClose}>
      <div
        className="w-[420px] max-h-[70vh] overflow-auto rounded-md border border-cronymax-border bg-cronymax-float p-3 shadow-xl"
        onClick={(e) => e.stopPropagation()}
      >
        <div className="mb-2 flex items-center justify-between">
          <span className="text-sm font-semibold">Add Agent</span>
          <button
            type="button"
            onClick={onClose}
            className="text-cronymax-caption hover:text-cronymax-title"
            aria-label="Close"
          >
            <Icon name="close" size={12} aria-hidden="true" />
          </button>
        </div>
        {agents.length === 0 ? (
          <p className="text-xs text-cronymax-caption">
            No agents registered. Define agents under your workspace’s <code>agents/</code> directory and reload.
          </p>
        ) : (
          <ul className="flex flex-col gap-1">
            {agents.map((a) => (
              <li key={a.name}>
                <button
                  type="button"
                  onClick={() => onPick(a.name)}
                  className="flex w-full items-center justify-between rounded border border-cronymax-border bg-cronymax-base px-2 py-1.5 text-left text-xs hover:bg-cronymax-float"
                >
                  <span className="font-medium">{a.name}</span>
                  <span className="text-[10px] text-cronymax-caption">{a.llm}</span>
                </button>
              </li>
            ))}
          </ul>
        )}
      </div>
    </div>
  );
}
