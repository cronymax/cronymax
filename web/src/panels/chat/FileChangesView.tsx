/**
 * FileChangesView — collapsible panel showing all workspace file mutations
 * recorded across the current chat session's conversation blocks.
 *
 * File changes are recorded by the `appendFileChange` reducer action when
 * write-capable tools (write_file, edit_file, delete_file, move_file,
 * create_directory, etc.) complete successfully during a run.
 */

import { useState } from "react";
import type { FileChange } from "./store";

const OPERATION_LABEL: Record<FileChange["operation"], { label: string; color: string }> = {
  created: { label: "new", color: "text-green-600 dark:text-green-400" },
  modified: { label: "mod", color: "text-blue-600 dark:text-blue-400" },
  deleted: { label: "del", color: "text-red-600 dark:text-red-400" },
  moved: { label: "mv", color: "text-amber-600 dark:text-amber-400" },
};

interface Props {
  changes: FileChange[];
}

export function FileChangesView({ changes }: Props) {
  const [collapsed, setCollapsed] = useState(false);

  if (changes.length === 0) return null;

  // Deduplicate: if the same path has multiple operations, keep the last one
  const deduped = new Map<string, FileChange>();
  for (const c of changes) {
    deduped.set(c.path, c);
  }
  const unique = [...deduped.values()];

  return (
    <div className="mx-3 mb-1 rounded-md border border-border bg-muted/30">
      {/* Header */}
      <button
        type="button"
        onClick={() => setCollapsed((c) => !c)}
        className="flex w-full items-center gap-2 px-2 py-1.5 text-left hover:bg-muted/60 transition rounded-md"
      >
        <span className="text-[11px] font-semibold text-muted-foreground uppercase tracking-wide">File changes</span>
        <span className="rounded-full bg-primary/15 px-1.5 py-0 text-[10px] font-mono text-primary">
          {unique.length}
        </span>
        <svg
          className={`ml-auto h-3 w-3 shrink-0 text-muted-foreground transition-transform ${collapsed ? "" : "rotate-180"}`}
          viewBox="0 0 24 24"
          fill="none"
          stroke="currentColor"
          strokeWidth={2}
        >
          <polyline points="6 9 12 15 18 9" />
        </svg>
      </button>

      {/* File list */}
      {!collapsed && (
        <div className="flex flex-col gap-px px-2 pb-2">
          {unique.map((c) => {
            const op = OPERATION_LABEL[c.operation] ?? { label: c.operation, color: "text-muted-foreground" };
            const filename = c.path.split("/").pop() ?? c.path;
            const dir = c.path.includes("/") ? c.path.slice(0, c.path.lastIndexOf("/")) : "";
            return (
              <div key={c.path} className="flex items-baseline gap-1.5 py-0.5 text-[11px]">
                <span className={`shrink-0 font-mono font-semibold w-8 text-right ${op.color}`}>{op.label}</span>
                <span className="font-mono text-foreground truncate" title={c.path}>
                  {filename}
                </span>
                {dir && <span className="font-mono text-muted-foreground truncate flex-1 opacity-60">{dir}</span>}
              </div>
            );
          })}
        </div>
      )}
    </div>
  );
}
