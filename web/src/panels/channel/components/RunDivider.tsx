import type { AppEvent } from "@/types/events";

export function RunDivider({ event }: { event: Extract<AppEvent, { kind: "system" }> }) {
  const label = event.payload.subkind.replace("_", " ");
  return (
    <div className="self-stretch flex items-center gap-2 text-[11px] uppercase tracking-wide text-cronymax-title/40 my-2">
      <div className="flex-1 h-px bg-cronymax-border" />
      <span>{label}</span>
      <div className="flex-1 h-px bg-cronymax-border" />
    </div>
  );
}
