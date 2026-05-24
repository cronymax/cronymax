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
import { Check, Palette, Play, Plug, ShieldCheck, Wrench, X } from "lucide-react";
import { useCallback, useEffect, useState } from "react";
import { PanelWindowHeader } from "@/components/PanelWindowHeader";
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
import { FieldLabel, Heading } from "@/components/ui/typography";
import { shells } from "@/shells/bridge";
import { AppearanceTab } from "./AppearanceTab";
import { MaintenanceTab } from "./MaintenanceTab";
import { ProfilesTab } from "./ProfilesTab";
import { ProvidersTab } from "./ProvidersTab";
import { RunnerTab } from "./RunnerTab";
import { type PermissionRequest, useStore } from "./store";

// ── types ─────────────────────────────────────────────────────────────────

type SettingsTab =
  | "appearance"
  | "providers"
  | "agents"
  | "doc-types"
  | "profiles"
  | "flows"
  | "runner"
  | "maintenance";

// ── shared Field ──────────────────────────────────────────────────────────

export function Field({ label, children, htmlFor }: { label: string; children: React.ReactNode; htmlFor?: string }) {
  return (
    <div className="mb-3">
      <FieldLabel htmlFor={htmlFor} className="mb-1">
        {label}
      </FieldLabel>
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
          <Button variant="outline" onClick={() => onResolve(false)}>
            <X />
            Deny
          </Button>
          <Button onClick={() => onResolve(true)}>
            <Check />
            Allow
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}

// ── Tab labels ────────────────────────────────────────────────────────────

const TAB_LABELS: { id: SettingsTab; label: string; icon: typeof Palette }[] = [
  { id: "appearance", label: "Appearance", icon: Palette },
  { id: "providers", label: "Providers", icon: Plug },
  { id: "profiles", label: "Profiles", icon: ShieldCheck },
  { id: "runner", label: "Runner", icon: Play },
  { id: "maintenance", label: "Maintenance", icon: Wrench },
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

  return (
    <main className="relative flex h-screen w-screen flex-col bg-background text-foreground">
      <PanelWindowHeader className="flex shrink-0 items-center justify-between border-b border-border bg-card px-4 py-2">
        <Heading>Settings</Heading>
        <button
          type="button"
          aria-label="Close settings"
          className="flex size-7 items-center justify-center rounded-md text-muted-foreground transition-colors hover:bg-accent hover:text-foreground"
          onClick={() => void shells.browser.shell.close_overlay()}
        >
          <X className="size-4" aria-hidden="true" />
        </button>
      </PanelWindowHeader>
      <Tabs
        value={tab}
        onValueChange={(v) => setTab(v as SettingsTab)}
        className="flex flex-1 flex-col overflow-hidden"
      >
        <div className="shrink-0 px-4 pt-3">
          <TabsList>
            {TAB_LABELS.map((t) => {
              const TabIcon = t.icon;
              return (
                <TabsTrigger key={t.id} value={t.id} className="gap-1.5">
                  <TabIcon className="size-3.5" aria-hidden="true" />
                  {t.label}
                </TabsTrigger>
              );
            })}
          </TabsList>
        </div>
        <TabsContent value="appearance" className="flex-1 overflow-y-auto data-[state=inactive]:hidden">
          <AppearanceTab />
        </TabsContent>
        <TabsContent value="providers" className="flex-1 overflow-hidden data-[state=inactive]:hidden">
          <ProvidersTab />
        </TabsContent>
        <TabsContent value="profiles" className="flex-1 overflow-y-auto data-[state=inactive]:hidden">
          <ProfilesTab />
        </TabsContent>
        <TabsContent value="runner" className="flex-1 overflow-y-auto data-[state=inactive]:hidden">
          <RunnerTab />
        </TabsContent>
        <TabsContent value="maintenance" className="flex-1 overflow-y-auto data-[state=inactive]:hidden">
          <MaintenanceTab />
        </TabsContent>
      </Tabs>
      {state.permission && <PermissionOverlay perm={state.permission} onResolve={onResolvePermission} />}
    </main>
  );
}
