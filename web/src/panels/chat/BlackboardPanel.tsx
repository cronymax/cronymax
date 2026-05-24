/**
 * BlackboardPanel — shows the current blackboard entries for an active flow
 * run and allows human injection of new entries.
 *
 * supervisor-session-ux tasks 8.2, 8.3
 */

import { Plus } from "lucide-react";
import { useState } from "react";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Textarea } from "@/components/ui/textarea";
import { blackboard } from "@/shells/runtime";

// ── BlackboardInjectEditor ─────────────────────────────────────────────────

interface InjectEditorProps {
  flowRunId: string;
  /** Called on successful injection with the injected key. */
  onDone: (key: string) => void;
}

function BlackboardInjectEditor({ flowRunId, onDone }: InjectEditorProps) {
  const [key, setKey] = useState("");
  const [content, setContent] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  async function handleInject() {
    if (!key.trim() || !content.trim()) return;
    setBusy(true);
    setError(null);
    try {
      await blackboard.inject(flowRunId, key.trim(), content);
      onDone(key.trim());
    } catch (e) {
      setError(e instanceof Error ? e.message : "Injection failed");
    } finally {
      setBusy(false);
    }
  }

  return (
    <div className="flex flex-col gap-2 rounded-md border border-dashed border-primary/40 p-3">
      <p className="text-xs font-medium text-muted-foreground">Human-inject a blackboard entry</p>
      <input
        className="rounded-md border border-input bg-background px-2 py-1 text-sm outline-none focus:ring-1 focus:ring-ring"
        placeholder="Key (e.g. design_spec)"
        value={key}
        onChange={(e) => setKey(e.target.value)}
      />
      <Textarea
        className="min-h-[80px] text-sm"
        placeholder="Entry content (Markdown supported)"
        value={content}
        onChange={(e) => setContent(e.target.value)}
      />
      {error && <p className="text-xs text-destructive">{error}</p>}
      <div className="flex gap-2">
        <Button size="sm" onClick={handleInject} disabled={busy || !key.trim() || !content.trim()}>
          {busy ? "Injecting…" : "Inject"}
        </Button>
        <Button size="sm" variant="ghost" onClick={() => onDone("")}>
          Cancel
        </Button>
      </div>
    </div>
  );
}

// ── BlackboardPanel ────────────────────────────────────────────────────────

interface BlackboardEntry {
  key: string;
  summary: string;
  writtenBy: "agent" | "human" | "auto_seeded";
}

interface Props {
  flowRunId: string;
  /** Optional: list of known entries (populated by parent via event subscription). */
  entries?: BlackboardEntry[];
  /** Called after a successful human injection with the injected key (task 8.7). */
  onInjected?: (key: string) => void;
}

export function BlackboardPanel({ flowRunId, entries = [], onInjected }: Props) {
  const [showInject, setShowInject] = useState(false);

  return (
    <div className="flex flex-col gap-2">
      <div className="flex items-center gap-2">
        <span className="text-xs font-medium uppercase tracking-wide text-muted-foreground">Blackboard</span>
        <Button
          size="icon"
          variant="ghost"
          className="ml-auto h-5 w-5"
          onClick={() => setShowInject((v) => !v)}
          aria-label="Inject blackboard entry"
        >
          <Plus className="h-3 w-3" />
        </Button>
      </div>

      {showInject && (
        <BlackboardInjectEditor
          flowRunId={flowRunId}
          onDone={(key) => {
            setShowInject(false);
            onInjected?.(key);
          }}
        />
      )}

      {entries.length === 0 ? (
        <p className="text-xs text-muted-foreground italic">No entries yet.</p>
      ) : (
        <div className="flex flex-col gap-1">
          {entries.map((e) => (
            <div key={e.key} className="flex items-start gap-2 rounded-md border border-border/60 px-3 py-2">
              <div className="min-w-0 flex-1">
                <div className="flex items-center gap-2">
                  <span className="font-mono text-xs font-medium text-foreground">{e.key}</span>
                  {e.writtenBy === "human" && (
                    <Badge variant="outline" className="text-[10px] text-amber-600">
                      [HUMAN-PROVIDED]
                    </Badge>
                  )}
                  {e.writtenBy === "auto_seeded" && (
                    <Badge variant="outline" className="text-[10px] text-blue-500">
                      seeded
                    </Badge>
                  )}
                </div>
                <p className="mt-0.5 truncate text-xs text-muted-foreground">{e.summary}</p>
              </div>
            </div>
          ))}
        </div>
      )}
    </div>
  );
}
