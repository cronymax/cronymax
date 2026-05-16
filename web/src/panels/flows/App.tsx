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
import { useCallback } from "react";
import { Flows } from "@/components/FlowEditor";
import { Icon } from "@/components/Icon";
import { Button } from "@/components/ui/button";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs";
import { shells } from "@/shells/bridge";
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

// ── App ───────────────────────────────────────────────────────────────────

export function App() {
  const onClose = useCallback(() => {
    shells.browser.shell.popover_close().catch(() => {
      /* ignore */
    });
  }, []);

  return (
    <Tabs defaultValue="flows" className="relative flex h-screen w-screen flex-col bg-background text-foreground">
      <header className="flex shrink-0 items-center justify-between border-b border-border bg-card px-4 py-2">
        <h1 className="text-sm font-semibold">Flows</h1>
        <Button variant="ghost" size="icon" className="h-6 w-6" onClick={onClose} title="Close" aria-label="Close">
          <Icon name="close" size={12} aria-hidden="true" />
        </Button>
      </header>
      <TabsList className="h-auto shrink-0 justify-start rounded-none border-b border-border bg-card px-1">
        <TabsTrigger
          value="flows"
          className="rounded-none border-b-2 border-transparent px-3 py-1.5 text-xs data-[state=active]:border-primary data-[state=active]:bg-transparent data-[state=active]:shadow-none"
        >
          Flows
        </TabsTrigger>
        <TabsTrigger
          value="agents"
          className="rounded-none border-b-2 border-transparent px-3 py-1.5 text-xs data-[state=active]:border-primary data-[state=active]:bg-transparent data-[state=active]:shadow-none"
        >
          Agents
        </TabsTrigger>
        <TabsTrigger
          value="doc-types"
          className="rounded-none border-b-2 border-transparent px-3 py-1.5 text-xs data-[state=active]:border-primary data-[state=active]:bg-transparent data-[state=active]:shadow-none"
        >
          Doc Types
        </TabsTrigger>
      </TabsList>
      <TabsContent value="flows" className="mt-0 flex-1 overflow-hidden data-[state=inactive]:hidden">
        <Flows />
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
