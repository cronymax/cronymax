/**
 * dagre auto-layout for React Flow nodes/edges.
 *
 * We compute (x, y) for each node using dagre's `network-simplex` ranker with
 * a compact node separation. The result is suitable when no `flow.layout.json`
 * sidecar is available. Node sizes are fixed estimates (180×56) — good
 * enough for the agent-card style used by `AgentNode`.
 */

import dagre from "dagre";
import type { Node, Edge } from "@xyflow/react";

const NODE_W = 200;
const NODE_H = 64;

export function layoutNodes(nodes: Node[], edges: Edge[]): Node[] {
  const g = new dagre.graphlib.Graph();
  g.setDefaultEdgeLabel(() => ({}));
  g.setGraph({ rankdir: "LR", nodesep: 40, ranksep: 80 });

  for (const n of nodes) g.setNode(n.id, { width: NODE_W, height: NODE_H });
  for (const e of edges) g.setEdge(e.source, e.target);

  dagre.layout(g);

  return nodes.map((n) => {
    const pos = g.node(n.id);
    return {
      ...n,
      position: { x: pos.x - NODE_W / 2, y: pos.y - NODE_H / 2 },
    };
  });
}
