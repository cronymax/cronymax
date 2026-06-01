import { Check, ChevronsUpDown } from "lucide-react";
import { useState } from "react";
import { Button } from "@/components/ui/button";
import { Command, CommandEmpty, CommandGroup, CommandInput, CommandItem, CommandList } from "@/components/ui/command";
import { Popover, PopoverContent, PopoverTrigger } from "@/components/ui/popover";
import { cn } from "@/lib/utils";
import type { ModelGroup } from "./modelGroups";

/** The current picker selection: a model within a group, or `null` = the
 *  "provider default" item. `groupId === ""` matches any group by model name
 *  (used when the caller only tracks a bare model string). */
export interface ModelSelection {
  groupId: string;
  model: string;
}

/**
 * Unified provider/model picker shared by the chat panel and the Agents editor.
 * Renders one grouped combobox over [`ModelGroup`]s (an LLM group per provider
 * plus one per extension AgentProvider). The caller owns selection state and
 * side effects via `onPick`.
 */
export function ModelGroupCombobox({
  groups,
  value,
  triggerLabel,
  defaultItemLabel = "provider default",
  onPick,
  triggerClassName,
  contentClassName = "w-[300px] p-0",
  side = "bottom",
}: {
  groups: ModelGroup[];
  value: ModelSelection | null;
  triggerLabel: string;
  defaultItemLabel?: string;
  /** `group === null` → the "provider default" item was chosen. */
  onPick: (group: ModelGroup | null, model: string) => void;
  /** Extra classes for the trigger button (compact toolbar vs full-width field). */
  triggerClassName?: string;
  contentClassName?: string;
  side?: "top" | "bottom";
}) {
  const [open, setOpen] = useState(false);
  const isSelected = (g: ModelGroup, m: string) =>
    value !== null && value.model === m && (value.groupId === "" || value.groupId === g.id);

  return (
    <Popover open={open} onOpenChange={setOpen}>
      <PopoverTrigger asChild>
        <Button
          type="button"
          variant="outline"
          size="sm"
          className={cn("w-full justify-between font-normal", triggerClassName)}
        >
          <span className="truncate">{triggerLabel}</span>
          <ChevronsUpDown className="shrink-0 opacity-50" />
        </Button>
      </PopoverTrigger>
      <PopoverContent className={contentClassName} align="start" side={side}>
        <Command>
          <CommandInput placeholder="Search models…" />
          <CommandList>
            <CommandEmpty>No models found.</CommandEmpty>
            <CommandGroup>
              <CommandItem
                value="provider default"
                onSelect={() => {
                  onPick(null, "");
                  setOpen(false);
                }}
                className="text-xs"
              >
                <Check className={cn("mr-2 size-3 shrink-0", value === null ? "opacity-100" : "opacity-0")} />
                <span className="italic text-muted-foreground">{defaultItemLabel}</span>
              </CommandItem>
            </CommandGroup>
            {groups.map((g) => (
              <CommandGroup key={g.id} heading={g.kind === "extension" ? `${g.label} (extension)` : g.label}>
                {g.models.map((m) => (
                  <CommandItem
                    key={`${g.id}:${m}`}
                    value={`${g.id} ${m}`}
                    onSelect={() => {
                      onPick(g, m);
                      setOpen(false);
                    }}
                    className="text-xs"
                  >
                    <Check className={cn("mr-2 size-3 shrink-0", isSelected(g, m) ? "opacity-100" : "opacity-0")} />
                    <span className="truncate font-mono">{m}</span>
                  </CommandItem>
                ))}
              </CommandGroup>
            ))}
          </CommandList>
        </Command>
      </PopoverContent>
    </Popover>
  );
}
