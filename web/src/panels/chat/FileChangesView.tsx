/**
 * FileChangesView — collapsible panel showing all workspace file mutations
 * recorded across the current chat session's conversation blocks.
 *
 * File changes are recorded by the `appendFileChange` reducer action when
 * write-capable tools (write_file, edit_file, delete_file, move_file,
 * create_directory, etc.) complete successfully during a run.
 */

import { ChevronDown, FilePen, FilePlus, FileX, MoveRight } from "lucide-react";
import { useState } from "react";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Collapsible, CollapsibleContent, CollapsibleTrigger } from "@/components/ui/collapsible";
import { cn } from "@/lib/utils";
import type { FileChange } from "./store";

// Icon + label + colour for each operation. Colours follow the chat panel's
// established policy: `text-primary` (brand teal) for positive states,
// `text-destructive` for destructive, semantic muted for neutral, amber
// retained as the only intentional non-semantic accent (consistent with
// shell/run-in-progress affordances elsewhere).
const OPERATION_META: Record<FileChange["operation"], { label: string; color: string; Icon: typeof FilePlus }> = {
  created: { label: "new", color: "text-primary", Icon: FilePlus },
  modified: { label: "mod", color: "text-muted-foreground", Icon: FilePen },
  deleted: { label: "del", color: "text-destructive", Icon: FileX },
  moved: { label: "mv", color: "text-amber-500", Icon: MoveRight },
};

interface Props {
  changes: FileChange[];
}

export function FileChangesView({ changes }: Props) {
  const [open, setOpen] = useState(true);

  if (changes.length === 0) return null;

  // Deduplicate: if the same path has multiple operations, keep the last one.
  const deduped = new Map<string, FileChange>();
  for (const c of changes) {
    deduped.set(c.path, c);
  }
  const unique = [...deduped.values()];

  return (
    <Collapsible open={open} onOpenChange={setOpen} className="mb-1 rounded-md border border-border bg-muted/30">
      <CollapsibleTrigger asChild>
        <Button
          variant="ghost"
          className="flex h-auto w-full items-center justify-start gap-2 rounded-md px-2 py-1.5 hover:bg-muted/60"
        >
          <span className="text-[11px] font-semibold uppercase tracking-wide text-muted-foreground">File changes</span>
          <Badge variant="secondary" className="h-4 px-1.5 font-mono text-[10px]">
            {unique.length}
          </Badge>
          <ChevronDown
            data-state={open ? "open" : "closed"}
            className="ml-auto size-3 shrink-0 text-muted-foreground transition-transform data-[state=closed]:-rotate-90"
          />
        </Button>
      </CollapsibleTrigger>

      <CollapsibleContent>
        <div className="flex flex-col gap-px px-2 pb-2">
          {unique.map((c) => {
            const meta = OPERATION_META[c.operation] ?? {
              label: c.operation,
              color: "text-muted-foreground",
              Icon: FilePen,
            };
            const filename = c.path.split("/").pop() ?? c.path;
            const dir = c.path.includes("/") ? c.path.slice(0, c.path.lastIndexOf("/")) : "";
            return (
              <div key={c.path} className="flex items-center gap-1.5 py-0.5 text-[11px]">
                <meta.Icon className={cn("size-3 shrink-0", meta.color)} />
                <span className={cn("w-8 shrink-0 text-right font-mono font-semibold", meta.color)}>{meta.label}</span>
                <span className="truncate font-mono text-foreground" title={c.path}>
                  {filename}
                </span>
                {dir && <span className="flex-1 truncate font-mono text-muted-foreground opacity-60">{dir}</span>}
              </div>
            );
          })}
        </div>
      </CollapsibleContent>
    </Collapsible>
  );
}
