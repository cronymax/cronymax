import type { AppEvent } from "@/types/events";

export function ReviewReply({ event }: { event: Extract<AppEvent, { kind: "review_event" }> }) {
  return (
    <div className="self-start text-xs text-foreground/70">
      <span className="font-mono">{event.payload.reviewer}</span> <span className="opacity-60">→</span>{" "}
      <span>{event.payload.verdict}</span>
      {event.payload.comment ? ` — ${event.payload.comment}` : ""}
    </div>
  );
}
