import { Handle, Position, type NodeProps } from "@xyflow/react";

export interface AgentNodeData extends Record<string, unknown> {
  label: string;
  status?: "idle" | "thinking" | "blocked" | "done";
  hasDocBadge?: boolean;
}

const STATUS_FILL: Record<string, string> = {
  idle: "bg-cronymax-surface",
  thinking: "bg-emerald-700/30 border-emerald-500/60",
  blocked: "bg-amber-700/30 border-amber-500/60",
  done: "bg-cronymax-surface-2 border-cronymax-fg/30",
};

export function AgentNode({ data }: NodeProps) {
  const d = data as AgentNodeData;
  const status = d.status ?? "idle";
  const fill = STATUS_FILL[status] ?? STATUS_FILL.idle;
  return (
    <div
      className={`rounded-lg border border-cronymax-border ${fill} px-3 py-2 text-sm text-cronymax-fg shadow-sm relative min-w-[160px]`}
    >
      <Handle type="target" position={Position.Left} />
      <div className="flex items-center gap-2">
        <span
          className={
            "h-2 w-2 rounded-full " +
            (status === "thinking"
              ? "bg-emerald-400 animate-pulse"
              : status === "blocked"
                ? "bg-amber-400"
                : status === "done"
                  ? "bg-cronymax-fg/40"
                  : "bg-cronymax-fg/20")
          }
        />
        <div className="font-mono text-xs">{d.label}</div>
      </div>
      {d.hasDocBadge && (
        <div className="absolute -top-1 -right-1 h-3 w-3 rounded-full bg-blue-400 ring-2 ring-cronymax-bg" />
      )}
      <Handle type="source" position={Position.Right} />
    </div>
  );
}
