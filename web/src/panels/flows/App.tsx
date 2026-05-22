/**
 * Settings panel — opened as a popover from the title bar gear button.
 *
 * Tabs:
 *   Appearance — theme mode (System / Light / Dark)
 *   Providers  — LLM endpoint list + active selection + GitHub Copilot OAuth
 *   Agents     — per-agent YAML editor (.cronymax/agents/*.agent.yaml)
 *   Workspace  — per-Space sandbox profile
 *   Flows      — visual agent-flow editor
 *   Runner     — legacy ReAct runner (terminal Explain/Fix/Retry target)
 */
import { useCallback, useEffect, useRef, useState } from "react";
import { Flows } from "@/components/FlowEditor";
import { PanelWindowHeader } from "@/components/PanelWindowHeader";
import { Alert, AlertDescription } from "@/components/ui/alert";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs";
import { flow } from "@/shells/runtime";
import { Heading } from "../../components/ui/typography";
import { AgentsTab } from "./AgentsTab";
import { DocTypesTab } from "./DocTypesTab";

// ── shared input styles ───────────────────────────────────────────────────

export const inputCls =
  "w-full rounded border border-border bg-background px-2 py-1 text-xs text-foreground outline-none focus:border-ring";

// ── shared Field ──────────────────────────────────────────────────────────

export function Field({ label, children }: { label: string; children: React.ReactNode }) {
  return (
    <div className="mb-3">
      <div className="mb-1 text-xs uppercase tracking-wide text-muted-foreground">{label}</div>
      {children}
    </div>
  );
}

// ── SchemaTab ─────────────────────────────────────────────────────────────

/**
 * SchemaTab — plain YAML editor for the currently selected flow definition.
 * Cmd+S (or Ctrl+S) triggers `flow.saveYaml` to persist the YAML content.
 * Shows a warning banner when a save fails with a FlowHasActiveRun error.
 */
function SchemaTab({ flowId }: { flowId: string }) {
  const [content, setContent] = useState("");
  const [saveError, setSaveError] = useState<string | null>(null);
  const [saved, setSaved] = useState(false);
  const textareaRef = useRef<HTMLTextAreaElement>(null);

  useEffect(() => {
    // Load current YAML from the runtime when the flow changes.
    if (!flowId) {
      setContent("");
      return;
    }
    flow
      .load(flowId)
      .then((def) => {
        // Render a basic YAML representation of the definition.
        setContent(typeof def === "string" ? def : JSON.stringify(def, null, 2));
      })
      .catch(() => {
        setContent("# Failed to load flow definition");
      });
  }, [flowId]);

  const handleSave = useCallback(async () => {
    if (!flowId) return;
    setSaveError(null);
    try {
      await flow.saveYaml(flowId, content);
      setSaved(true);
      setTimeout(() => setSaved(false), 2000);
    } catch (e) {
      const msg = e instanceof Error ? e.message : String(e);
      setSaveError(
        msg.toLowerCase().includes("active run") ? "Cannot save: flow has an active run. Stop the run first." : msg,
      );
    }
  }, [flowId, content]);

  function handleKeyDown(e: React.KeyboardEvent) {
    if ((e.metaKey || e.ctrlKey) && e.key === "s") {
      e.preventDefault();
      void handleSave();
    }
  }

  if (!flowId) {
    return (
      <div className="flex h-full items-center justify-center text-sm text-muted-foreground">
        Select a flow to edit its schema.
      </div>
    );
  }

  return (
    <div className="flex h-full flex-col">
      {saveError && (
        <Alert variant="destructive" className="mx-4 mt-3 rounded-md">
          <AlertDescription className="text-xs">{saveError}</AlertDescription>
        </Alert>
      )}
      {saved && (
        <div className="mx-4 mt-3 rounded-md border border-green-500/30 bg-green-500/5 px-3 py-1.5 text-xs text-green-700 dark:text-green-400">
          Saved
        </div>
      )}
      <div className="px-4 pt-2 text-xs text-muted-foreground">
        Press <kbd className="rounded bg-muted px-1">⌘S</kbd> to save
      </div>
      <textarea
        ref={textareaRef}
        className="m-4 flex-1 rounded-md border border-input bg-background p-3 font-mono text-xs leading-relaxed text-foreground outline-none focus:ring-1 focus:ring-ring"
        value={content}
        onChange={(e) => setContent(e.target.value)}
        onKeyDown={handleKeyDown}
        spellCheck={false}
      />
    </div>
  );
}

// ── PreviewTab ─────────────────────────────────────────────────────────────

/**
 * PreviewTab — shows a read-only text summary of the selected flow's nodes.
 * A lightweight "preview" without the full canvas drag-and-drop UI.
 */
function PreviewTab({ flowId }: { flowId: string }) {
  const [summary, setSummary] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);

  useEffect(() => {
    if (!flowId) {
      setSummary(null);
      return;
    }
    setLoading(true);
    flow
      .load(flowId)
      .then((def) => {
        try {
          const d = def as {
            name?: string;
            description?: string;
            nodes?: Array<{ id?: string; owner?: string; description?: string }>;
          };
          const lines: string[] = [];
          if (d.name) lines.push(`# ${d.name}`);
          if (d.description) lines.push(`\n${d.description}\n`);
          if (d.nodes && d.nodes.length > 0) {
            lines.push("\n**Nodes:**");
            for (const n of d.nodes) {
              const id = n.id ?? "(unnamed)";
              const agent = n.owner ?? "(no agent)";
              const desc = n.description ? ` — ${n.description}` : "";
              lines.push(`- **${id}** (${agent})${desc}`);
            }
          }
          setSummary(lines.join("\n") || "(empty flow)");
        } catch {
          setSummary("(could not parse flow definition)");
        }
      })
      .catch(() => setSummary("Failed to load flow."))
      .finally(() => setLoading(false));
  }, [flowId]);

  if (!flowId) {
    return (
      <div className="flex h-full items-center justify-center text-sm text-muted-foreground">
        Select a flow to preview.
      </div>
    );
  }

  return (
    <div className="h-full overflow-y-auto px-4 py-4">
      {loading ? (
        <p className="text-xs text-muted-foreground">Loading…</p>
      ) : (
        <pre className="whitespace-pre-wrap text-xs text-foreground">{summary ?? "(no content)"}</pre>
      )}
    </div>
  );
}

// ── App ───────────────────────────────────────────────────────────────────

export function App() {
  const [activeFlowId] = useState<string>("");

  return (
    <Tabs defaultValue="canvas" className="relative flex h-screen w-screen flex-col bg-background text-foreground">
      <PanelWindowHeader className="flex shrink-0 items-center border-b border-border bg-card px-4 py-2">
        <Heading>Flows</Heading>
      </PanelWindowHeader>
      <TabsList className="mx-4">
        <TabsTrigger value="canvas" className="gap-1.5">
          Canvas
        </TabsTrigger>
        <TabsTrigger value="schema" className="gap-1.5">
          Schema
        </TabsTrigger>
        <TabsTrigger value="preview" className="gap-1.5">
          Preview
        </TabsTrigger>
        <TabsTrigger value="agents" className="gap-1.5">
          Agents
        </TabsTrigger>
        <TabsTrigger value="doc-types" className="gap-1.5">
          Doc Types
        </TabsTrigger>
      </TabsList>
      <TabsContent value="canvas" className="mt-0 flex-1 overflow-hidden data-[state=inactive]:hidden">
        <Flows />
      </TabsContent>
      <TabsContent value="schema" className="mt-0 flex-1 overflow-hidden data-[state=inactive]:hidden">
        <SchemaTab flowId={activeFlowId} />
      </TabsContent>
      <TabsContent value="preview" className="mt-0 flex-1 overflow-hidden data-[state=inactive]:hidden">
        <PreviewTab flowId={activeFlowId} />
      </TabsContent>
      <TabsContent value="agents" className="mt-0 flex-1 overflow-hidden data-[state=inactive]:hidden">
        <AgentsTab />
      </TabsContent>
      <TabsContent value="doc-types" className="mt-0 flex-1 overflow-hidden data-[state=inactive]:hidden">
        <DocTypesTab />
      </TabsContent>
    </Tabs>
  );
}
