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
import { browser } from "@/shells/bridge";
import { AppearanceTab } from "./AppearanceTab";
import { ProfilesTab } from "./ProfilesTab";
import { ProvidersTab } from "./ProvidersTab";
import { RunnerTab } from "./RunnerTab";
import { type PermissionRequest, useStore } from "./store";

// ── types ─────────────────────────────────────────────────────────────────

type SettingsTab = "appearance" | "providers" | "agents" | "doc-types" | "profiles" | "flows" | "runner";

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

// ── Permission overlay ────────────────────────────────────────────────────

function PermissionOverlay({ perm, onResolve }: { perm: PermissionRequest; onResolve: (allow: boolean) => void }) {
  return (
    <div className="absolute inset-0 z-50 flex items-center justify-center bg-black/40">
      <div className="w-[340px] rounded-md border border-cronymax-border bg-cronymax-float p-4 text-sm text-cronymax-title shadow-lg">
        <p className="mb-3 whitespace-pre-wrap">{perm.prompt}</p>
        <div className="flex justify-end gap-2">
          <button
            type="button"
            onClick={() => onResolve(true)}
            className="rounded bg-cronymax-primary px-3 py-1 text-xs font-medium text-white hover:opacity-90"
          >
            Allow
          </button>
          <button
            type="button"
            onClick={() => onResolve(false)}
            className="rounded border border-cronymax-border bg-cronymax-base px-3 py-1 text-xs text-cronymax-title hover:bg-cronymax-float"
          >
            Deny
          </button>
        </div>
      </div>
    </div>
  );
}

// ── Tab bar ───────────────────────────────────────────────────────────────

const TAB_LABELS: { id: SettingsTab; label: string }[] = [
  { id: "appearance", label: "Appearance" },
  { id: "providers", label: "Providers" },
  { id: "profiles", label: "Profiles" },
  { id: "runner", label: "Runner" },
];

function TabBar({ tab, onChange }: { tab: SettingsTab; onChange: (t: SettingsTab) => void }) {
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
  const [tab, setTab] = useState<SettingsTab>("appearance");
  const [state, dispatch] = useStore();

  // Load LLM config on mount so providers panel has initial values.
  useEffect(() => {
    void (async () => {
      try {
        const provRes = await browser.send("llm.providers.get");
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
        browser
          .send("permission.respond", {
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
    browser.send("shell.popover_close").catch(() => {
      /* ignore */
    });
  }, []);

  return (
    <main className="relative flex h-screen w-screen flex-col bg-cronymax-base text-cronymax-title">
      <header className="flex items-center justify-between border-b border-cronymax-border bg-cronymax-float px-4 py-2">
        <h1 className="text-sm font-semibold tracking-wide">Settings</h1>
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
        {tab === "appearance" && <AppearanceTab />}
        {tab === "providers" && <ProvidersTab />}
        {tab === "profiles" && <ProfilesTab />}
        {tab === "runner" && <RunnerTab />}
      </div>

      {state.permission && <PermissionOverlay perm={state.permission} onResolve={onResolvePermission} />}
    </main>
  );
}
