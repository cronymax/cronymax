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
import { useCallback, useEffect, useState } from "react";
import { Icon } from "@/components/Icon";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs";
import { shells } from "@/shells/bridge";
import { AppearanceTab } from "./AppearanceTab";
import { ProfilesTab } from "./ProfilesTab";
import { ProvidersTab } from "./ProvidersTab";
import { RunnerTab } from "./RunnerTab";
import { type PermissionRequest, useStore } from "./store";

// ── types ─────────────────────────────────────────────────────────────────

type SettingsTab = "appearance" | "providers" | "agents" | "doc-types" | "profiles" | "flows" | "runner";

// ── ReAct graph builder ───────────────────────────────────────────────────

// ── shared input styles ───────────────────────────────────────────────────
// Kept for legacy references; new code should import Input from @/components/ui/input.
export const inputCls =
  "w-full rounded border border-border bg-background px-2 py-1 text-xs text-foreground outline-none focus:border-primary";

// ── shared Field ──────────────────────────────────────────────────────────

export function Field({ label, children, htmlFor }: { label: string; children: React.ReactNode; htmlFor?: string }) {
  return (
    <div className="mb-3">
      <label htmlFor={htmlFor} className="mb-1 block text-xs uppercase tracking-wide text-muted-foreground">
        {label}
      </label>
      {children}
    </div>
  );
}

// ── Permission overlay ────────────────────────────────────────────────────

function PermissionOverlay({ perm, onResolve }: { perm: PermissionRequest; onResolve: (allow: boolean) => void }) {
  return (
    <Dialog
      open
      onOpenChange={(open) => {
        if (!open) onResolve(false);
      }}
    >
      <DialogContent className="w-[340px]">
        <DialogHeader>
          <DialogTitle className="text-sm">Permission Request</DialogTitle>
          <DialogDescription className="whitespace-pre-wrap text-sm">{perm.prompt}</DialogDescription>
        </DialogHeader>
        <DialogFooter>
          <Button size="sm" variant="outline" onClick={() => onResolve(false)}>
            Deny
          </Button>
          <Button size="sm" onClick={() => onResolve(true)}>
            Allow
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}

// ── Tab labels ────────────────────────────────────────────────────────────

const TAB_LABELS: { id: SettingsTab; label: string }[] = [
  { id: "appearance", label: "Appearance" },
  { id: "providers", label: "Providers" },
  { id: "profiles", label: "Profiles" },
  { id: "runner", label: "Runner" },
];

// ── App ───────────────────────────────────────────────────────────────────

export function App() {
  const [tab, setTab] = useState<SettingsTab>("appearance");
  const [state, dispatch] = useStore();

  // Load LLM config on mount so providers panel has initial values.
  useEffect(() => {
    void (async () => {
      try {
        const provRes = await shells.browser.llm.providers.get();
        const providers = JSON.parse(provRes.raw || "[]") as Array<{
          id: string;
          base_url?: string;
          api_key?: string;
          default_model?: string;
        }>;
        const active = providers.find((p) => p.id === provRes.active_id) || providers[0];
        if (active) {
          dispatch({
            type: "setLlmConfig",
            baseUrl: active.base_url ?? "",
            apiKey: active.api_key ?? "",
            model: active.default_model ?? "",
          });
        }
      } catch {
        /* ignore */
      }
    })();
  }, [dispatch]);

  // Permission gate — resolves runtime permission_request events via the
  // permission.respond bridge channel.
  // (The legacy window.__getPermission hook for the in-process ReAct runtime
  // has been removed; permission requests now arrive as capability_call events
  // and are handled by the host capability adapter.)

  const onResolvePermission = useCallback(
    (allow: boolean) => {
      const perm = state.permission;
      if (!perm) return;
      if (perm.requestId) {
        shells.browser.permission
          .respond({
            request_id: perm.requestId,
            decision: allow ? "allow" : "deny",
          })
          .catch(() => undefined);
      }
      perm.resolve?.(allow);
      dispatch({ type: "clearPermission" });
    },
    [state.permission, dispatch],
  );

  const onClose = useCallback(() => {
    shells.browser.shell.popover_close().catch(() => {
      /* ignore */
    });
  }, []);

  return (
    <main className="relative flex h-screen w-screen flex-col bg-background text-foreground">
      <header className="flex shrink-0 items-center justify-between border-b border-border bg-card px-4 py-2">
        <h1 className="text-sm font-semibold">Settings</h1>
        <Button variant="ghost" size="icon" className="h-6 w-6" onClick={onClose} title="Close" aria-label="Close">
          <Icon name="close" size={12} aria-hidden="true" />
        </Button>
      </header>
      <Tabs
        value={tab}
        onValueChange={(v) => setTab(v as SettingsTab)}
        className="flex flex-1 flex-col overflow-hidden"
      >
        <TabsList className="h-auto shrink-0 justify-start rounded-none border-b border-border bg-card px-1">
          {TAB_LABELS.map((t) => (
            <TabsTrigger
              key={t.id}
              value={t.id}
              className="rounded-none border-b-2 border-transparent px-3 py-1.5 text-xs data-[state=active]:border-primary data-[state=active]:bg-transparent data-[state=active]:shadow-none"
            >
              {t.label}
            </TabsTrigger>
          ))}
        </TabsList>
        <TabsContent value="appearance" className="mt-0 flex-1 overflow-y-auto data-[state=inactive]:hidden">
          <AppearanceTab />
        </TabsContent>
        <TabsContent value="providers" className="mt-0 flex-1 overflow-hidden data-[state=inactive]:hidden">
          <ProvidersTab />
        </TabsContent>
        <TabsContent value="profiles" className="mt-0 flex-1 overflow-y-auto data-[state=inactive]:hidden">
          <ProfilesTab />
        </TabsContent>
        <TabsContent value="runner" className="mt-0 flex-1 overflow-y-auto data-[state=inactive]:hidden">
          <RunnerTab />
        </TabsContent>
      </Tabs>
      {state.permission && <PermissionOverlay perm={state.permission} onResolve={onResolvePermission} />}
    </main>
  );
}
