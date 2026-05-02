import { useEffect, useMemo, useState } from "react";
import {
  ReactFlow,
  ReactFlowProvider,
  Background,
  Controls,
  type Node,
  type Edge,
  type EdgeMarker,
  MarkerType,
} from "@xyflow/react";
import { bridge } from "@/bridge";
import type { AppEvent } from "@/types/events";
import { AgentNode } from "./AgentNode";
import { layoutNodes } from "./layout";

interface FlowDef {
  id: string;
  name: string;
  agents: string[];
  edges: Array<{
    from: string;
    to: string;
    port: string;
    requires_human_approval?: boolean;
  }>;
}

const nodeTypes = { agent: AgentNode };

function readFlowIdFromUrl(): string {
  const p = new URLSearchParams(window.location.search);
  return p.get("flow") ?? p.get("flow_id") ?? "";
}

function readModeFromUrl(): "edit" | "run" {
  const p = new URLSearchParams(window.location.search);
  return p.get("mode") === "run" ? "run" : "edit";
}

function buildGraph(def: FlowDef): { nodes: Node[]; edges: Edge[] } {
  const nodes: Node[] = def.agents.map((a) => ({
    id: a,
    type: "agent",
    position: { x: 0, y: 0 },
    data: { label: a, status: "idle" },
  }));
  const edges: Edge[] = def.edges.map((e, i) => ({
    id: `${e.from}->${e.to}#${e.port}#${i}`,
    source: e.from,
    target: e.to,
    label: e.port + (e.requires_human_approval ? " (approval)" : ""),
    markerEnd: { type: MarkerType.ArrowClosed } as EdgeMarker,
    animated: false,
    style: { stroke: "var(--cronymax-fg, #888)" },
  }));
  return { nodes, edges };
}

export function App() {
  const flowId = useMemo(readFlowIdFromUrl, []);
  const [mode, setMode] = useState<"edit" | "run">(readModeFromUrl);
  const [def, setDef] = useState<FlowDef | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [nodes, setNodes] = useState<Node[]>([]);
  const [edges, setEdges] = useState<Edge[]>([]);

  // Load flow definition + dagre layout.
  useEffect(() => {
    if (!flowId) {
      setError("Missing ?flow=<id> in URL");
      return;
    }
    let cancelled = false;
    (async () => {
      try {
        const res = await bridge.send("flow.load", { id: flowId });
        if (cancelled) return;
        const d: FlowDef = {
          id: res.id,
          name: res.name,
          agents: res.agents,
          edges: res.edges,
        };
        setDef(d);
        const built = buildGraph(d);
        const laidOut = layoutNodes(built.nodes, built.edges);
        setNodes(laidOut);
        setEdges(built.edges);
      } catch (err) {
        if (!cancelled) {
          setError(err instanceof Error ? err.message : String(err));
        }
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [flowId]);

  // Run-mode: subscribe to AppEvents and project agent_status / handoff /
  // document_event into node fill, edge animation, doc badge.
  useEffect(() => {
    if (mode !== "run" || !flowId) return;
    let off = () => {};
    (async () => {
      try {
        await bridge.send("events.subscribe", { flow_id: flowId });
      } catch {
        // best-effort
      }
      off = bridge.on("event", (payload) => {
        const e = payload as AppEvent;
        if (e.flow_id && e.flow_id !== flowId) return;
        applyRunEvent(e);
      });
    })();
    return () => off();
  }, [mode, flowId]);

  function applyRunEvent(e: AppEvent) {
    if (e.kind === "agent_status" && e.agent_id) {
      setNodes((prev) =>
        prev.map((n) =>
          n.id === e.agent_id
            ? { ...n, data: { ...n.data, status: e.payload.status } }
            : n,
        ),
      );
    } else if (e.kind === "handoff") {
      const sourceId = e.payload.from_agent;
      const targetId = e.payload.to_agent;
      setEdges((prev) =>
        prev.map((edge) =>
          edge.source === sourceId && edge.target === targetId
            ? { ...edge, animated: true, style: { stroke: "#34d399" } }
            : edge,
        ),
      );
      // De-animate after a short window.
      setTimeout(() => {
        setEdges((prev) =>
          prev.map((edge) =>
            edge.source === sourceId && edge.target === targetId
              ? {
                  ...edge,
                  animated: false,
                  style: { stroke: "var(--cronymax-fg, #888)" },
                }
              : edge,
          ),
        );
      }, 1500);
    } else if (e.kind === "document_event") {
      const producer = e.payload.producer;
      setNodes((prev) =>
        prev.map((n) =>
          n.id === producer
            ? { ...n, data: { ...n.data, hasDocBadge: true } }
            : n,
        ),
      );
    }
  }

  const isRun = mode === "run";

  return (
    <div className="flex h-screen flex-col bg-cronymax-bg text-cronymax-fg">
      <header className="flex items-center gap-2 border-b border-cronymax-border bg-cronymax-surface px-3 py-2">
        <div className="text-sm font-medium">Editor</div>
        <div className="text-xs opacity-60 font-mono">
          {def?.name ?? flowId ?? "(no flow)"}
        </div>
        <div className="flex-1" />
        <div className="flex rounded border border-cronymax-border overflow-hidden">
          <button
            type="button"
            className={
              "px-2 py-1 text-xs " +
              (mode === "edit"
                ? "bg-cronymax-accent text-white"
                : "bg-cronymax-surface hover:bg-cronymax-surface-2")
            }
            onClick={() => setMode("edit")}
          >
            Edit
          </button>
          <button
            type="button"
            className={
              "px-2 py-1 text-xs " +
              (mode === "run"
                ? "bg-cronymax-accent text-white"
                : "bg-cronymax-surface hover:bg-cronymax-surface-2")
            }
            onClick={() => setMode("run")}
          >
            Run
          </button>
        </div>
      </header>

      {error && (
        <div className="border-b border-red-500/40 bg-red-900/30 px-3 py-1 text-xs text-red-200">
          {error}
        </div>
      )}

      <div className="flex-1 relative">
        <ReactFlowProvider>
          <ReactFlow
            nodes={nodes}
            edges={edges}
            nodeTypes={nodeTypes}
            nodesDraggable={!isRun}
            nodesConnectable={!isRun}
            elementsSelectable={!isRun}
            fitView
          >
            <Background gap={16} size={1} />
            <Controls position="bottom-right" />
          </ReactFlow>
        </ReactFlowProvider>
        {isRun && (
          <div className="absolute top-2 left-2 rounded bg-emerald-700/40 px-2 py-1 text-[10px] uppercase text-emerald-200">
            Run mode · read-only
          </div>
        )}
      </div>
    </div>
  );
}
