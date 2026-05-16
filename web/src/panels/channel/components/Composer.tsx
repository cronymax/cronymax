import { type FormEvent, type KeyboardEvent, useState } from "react";
import { shells } from "@/shells/bridge";

interface Props {
  flowId: string;
  runId?: string;
  knownAgents?: string[];
}

interface MentionToken {
  text: string;
  known: boolean;
}

function tokenize(body: string, known: Set<string>): MentionToken[] {
  const out: MentionToken[] = [];
  for (const m of body.matchAll(/@([\w./-]+)/g)) {
    const tok = m[1] ?? "";
    if (!tok) continue;
    out.push({ text: tok, known: known.size === 0 || known.has(tok) });
  }
  return out;
}

export function Composer({ flowId, runId, knownAgents = [] }: Props) {
  const [body, setBody] = useState("");
  const [sending, setSending] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const knownSet = new Set(knownAgents);
  const mentions = tokenize(body, knownSet);
  const unknown = mentions.filter((m) => !m.known).map((m) => m.text);
  const canSend = body.trim().length > 0 && !sending && !!flowId;

  async function submit() {
    if (!canSend) return;
    setSending(true);
    setError(null);
    try {
      await shells.browser.events.append({
        kind: "text",
        flow_id: flowId,
        run_id: runId,
        body,
        mentions: mentions.map((m) => m.text),
      });
      setBody("");
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    } finally {
      setSending(false);
    }
  }

  function onKey(e: KeyboardEvent<HTMLTextAreaElement>) {
    if ((e.metaKey || e.ctrlKey) && e.key === "Enter") {
      e.preventDefault();
      void submit();
    }
  }

  function onSubmit(e: FormEvent) {
    e.preventDefault();
    void submit();
  }

  return (
    <form onSubmit={onSubmit} className="border-t border-border bg-background p-2">
      {mentions.length > 0 && (
        <div className="mb-1 flex flex-wrap gap-1">
          {mentions.map((m, i) => (
            <span
              key={i}
              className={
                "rounded px-1.5 py-0.5 text-xs font-mono " +
                (m.known ? "bg-card text-foreground/80" : "bg-red-900/40 text-red-200 ring-1 ring-red-500/40")
              }
              title={m.known ? undefined : "unknown agent"}
            >
              @{m.text}
            </span>
          ))}
        </div>
      )}
      <textarea
        value={body}
        onChange={(e) => setBody(e.target.value)}
        onKeyDown={onKey}
        rows={2}
        placeholder="Message channel… (Cmd/Ctrl+Enter to send, @mention agents)"
        className="w-full resize-none rounded border border-border bg-card px-2 py-1 text-sm text-foreground outline-none focus:border-primary"
      />
      <div className="mt-1 flex items-center justify-between">
        <div className="text-xs text-foreground/60">
          {unknown.length > 0 ? `Unknown: ${unknown.map((u) => `@${u}`).join(", ")}` : error || ""}
        </div>
        <button
          type="submit"
          disabled={!canSend}
          className="rounded bg-primary px-3 py-1 text-xs text-white disabled:opacity-40"
        >
          {sending ? "Sending…" : "Send"}
        </button>
      </div>
    </form>
  );
}
