/**
 * ExecutionCanvas — read-only execution-mode view of a flow graph.
 *
 * Wraps FlowEditor with mode="execution" and subscribes to session events to
 * track per-node status. Seeded nodes are displayed with a "⊘ skip" overlay.
 *
 * supervisor-session-ux task 8.5
 */

import { useEffect, useRef, useState } from "react";
import { FlowEditor, Provider } from "@/components/FlowEditor";
import { runtime } from "@/shells/bridge";

// ── Types ──────────────────────────────────────────────────────────────────

interface Props {
  /** Flow id used to load the graph (matches the flow name in localStorage). */
  flowId: string;
  /** Child session id for subscribing to live node-status events. */
  childSessionId: string;
}

// ── Component ──────────────────────────────────────────────────────────────

function ExecutionCanvasInner({ flowId, childSessionId }: Props) {
  const [nodeStatuses, setNodeStatuses] = useState<Record<string, string>>({});
  const latestRef = useRef(nodeStatuses);
  latestRef.current = nodeStatuses;

  useEffect(() => {
    if (!childSessionId) return;

    const off = runtime.on(`session:${childSessionId}`, (raw: unknown) => {
      const ev = raw as Record<string, unknown> | null;
      if (!ev) return;
      const pl = (ev.payload as Record<string, unknown> | undefined) ?? {};
      const kind = pl.kind as string | undefined;

      if (kind === "run_status") {
        const agentId = (pl.agent_id as string | undefined) ?? "";
        const runStatus = (pl.status as string | undefined) ?? "";
        if (!agentId || !runStatus) return;
        setNodeStatuses((prev) => ({ ...prev, [agentId]: runStatus }));
      }
    });

    return () => off?.();
  }, [childSessionId]);

  return <FlowEditor mode="execution" initialFlowId={flowId} nodeStatuses={nodeStatuses} />;
}

export function ExecutionCanvas(props: Props) {
  return (
    <Provider>
      <ExecutionCanvasInner {...props} />
    </Provider>
  );
}
