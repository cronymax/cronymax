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
import { useCallback, useState } from "react";
import { Flows } from "@/components/FlowEditor";
import { Icon } from "@/components/Icon";
import { browser } from "@/shells/bridge";
import { AgentsTab } from "./AgentsTab";
import { DocTypesTab } from "./DocTypesTab";

// ── types ─────────────────────────────────────────────────────────────────

type FlowsTab = "flows" | "agents" | "doc-types";

// ── ReAct graph builder ───────────────────────────────────────────────────

// ── shared input styles ───────────────────────────────────────────────────

export const inputCls =
  "w-full rounded border border-cronymax-border bg-cronymax-base px-2 py-1 text-xs text-cronymax-title outline-none focus:border-cronymax-primary";

// ── shared Field ──────────────────────────────────────────────────────────

export function Field({ label, children }: { label: string; children: React.ReactNode }) {
  return (
    <div className="mb-3">
      <div className="mb-1 text-[11px] uppercase tracking-wide text-cronymax-caption">{label}</div>
      {children}
    </div>
  );
}

// ── Tab bar ───────────────────────────────────────────────────────────────

const TAB_LABELS: { id: FlowsTab; label: string }[] = [
  { id: "flows", label: "Flows" },
  { id: "agents", label: "Agents" },
  { id: "doc-types", label: "Doc Types" },
];

function TabBar({ tab, onChange }: { tab: FlowsTab; onChange: (t: FlowsTab) => void }) {
  return (
    <nav className="flex items-center gap-0 border-b border-cronymax-border bg-cronymax-float px-1">
      {TAB_LABELS.map((t) => (
        <button
          key={t.id}
          type="button"
          onClick={() => onChange(t.id)}
          className={
            "border-b-2 px-3 py-1.5 text-xs transition " +
            (tab === t.id
              ? "border-cronymax-primary text-cronymax-title"
              : "border-transparent text-cronymax-caption hover:text-cronymax-title")
          }
        >
          {t.label}
        </button>
      ))}
    </nav>
  );
}

// ── App ───────────────────────────────────────────────────────────────────

export function App() {
  const [tab, setTab] = useState<FlowsTab>("flows");

  const onClose = useCallback(() => {
    browser.send("shell.popover_close").catch(() => {
      /* ignore */
    });
  }, []);

  return (
    <main className="relative flex h-screen w-screen flex-col bg-cronymax-base text-cronymax-title">
      <header className="flex items-center justify-between border-b border-cronymax-border bg-cronymax-float px-4 py-2">
        <h1 className="text-sm font-semibold tracking-wide">Flows</h1>
        <button
          type="button"
          onClick={onClose}
          className="rounded px-2 py-0.5 text-xs text-cronymax-caption hover:bg-cronymax-base hover:text-cronymax-title"
          title="Close"
          aria-label="Close"
        >
          <Icon name="close" size={12} aria-hidden="true" />
        </button>
      </header>

      <TabBar tab={tab} onChange={setTab} />

      <div className="flex-1 overflow-hidden">
        {tab === "flows" && <Flows />}
        {tab === "agents" && <AgentsTab />}
        {tab === "doc-types" && <DocTypesTab />}
      </div>
    </main>
  );
}
