import type { AppEvent } from "@/types/events";

interface Props {
  event: Extract<AppEvent, { kind: "text" }>;
}

export function TextBubble({ event }: Props) {
  const author = event.payload.author ?? "unknown";
  const isMe = author === "me";
  return (
    <div
      className={
        "max-w-[80%] rounded-lg px-3 py-2 text-sm " +
        (isMe ? "self-end bg-primary text-white" : "self-start bg-card text-foreground")
      }
    >
      <div className="text-xs uppercase tracking-wide opacity-60">{author}</div>
      <div className="whitespace-pre-wrap">{renderBody(event.payload.body)}</div>
    </div>
  );
}

function renderBody(body: string) {
  const parts = body.split(/(@[\w./-]+)/g);
  return parts.map((p, i) =>
    p.startsWith("@") ? (
      <span key={i} className="rounded bg-black/20 px-1 font-mono text-xs">
        {p}
      </span>
    ) : (
      <span key={i}>{p}</span>
    ),
  );
}
