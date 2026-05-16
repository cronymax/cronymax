import { useCallback, useEffect, useRef, useState } from "react";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import { type LlmProvider, ModelSelect } from "../../components/ModelSelect";
import { shells } from "../../shells/bridge";
import { Field } from "./App";
import { useStore } from "./store";

// ── Providers tab ─────────────────────────────────────────────────────────
type ProviderKind = "openai" | "anthropic" | "ollama" | "github_copilot" | "custom";
const KIND_PRESETS: Record<ProviderKind, { base_url: string; default_model: string; display: string }> = {
  openai: {
    base_url: "https://api.openai.com/v1",
    default_model: "gpt-4o-mini",
    display: "OpenAI",
  },
  anthropic: {
    base_url: "https://api.anthropic.com/v1",
    default_model: "claude-3-5-sonnet-latest",
    display: "Anthropic",
  },
  ollama: {
    base_url: "http://localhost:11434/v1",
    default_model: "llama3.1",
    display: "Ollama",
  },
  github_copilot: {
    base_url: "https://api.githubcopilot.com",
    default_model: "gpt-4o",
    display: "GitHub Copilot",
  },
  custom: { base_url: "", default_model: "", display: "Custom" },
};
function newProvider(kind: ProviderKind = "openai"): LlmProvider {
  const preset = KIND_PRESETS[kind];
  const id = `p_${Date.now().toString(36)}_${Math.random().toString(36).slice(2, 7)}`;
  return {
    id,
    name: preset.display,
    kind,
    base_url: preset.base_url,
    api_key: "",
    default_model: preset.default_model,
    reasoning_effort: "",
  };
}

// GitHub Copilot OAuth (device flow)
const GITHUB_COPILOT_CLIENT_ID = "Iv1.b507a08c87ecfe98";
interface DeviceCode {
  device_code: string;
  user_code: string;
  verification_uri: string;
  expires_in: number;
  interval: number;
}

async function startGithubDeviceCode(): Promise<DeviceCode> {
  const resp = await fetch("https://github.com/login/device/code", {
    method: "POST",
    headers: {
      Accept: "application/json",
      "Content-Type": "application/json",
    },
    body: JSON.stringify({
      client_id: GITHUB_COPILOT_CLIENT_ID,
      scope: "read:user",
    }),
  });
  if (!resp.ok) throw new Error(`device/code ${resp.status}`);
  const data = await resp.json();
  if (!data.device_code) throw new Error(data.error_description || "no device_code");
  return data as DeviceCode;
}

async function pollGithubAccessToken(
  device_code: string,
): Promise<{ ok: true; access_token: string } | { ok: false; retry: boolean; interval?: number; error: string }> {
  const resp = await fetch("https://github.com/login/oauth/access_token", {
    method: "POST",
    headers: {
      Accept: "application/json",
      "Content-Type": "application/json",
    },
    body: JSON.stringify({
      client_id: GITHUB_COPILOT_CLIENT_ID,
      device_code,
      grant_type: "urn:ietf:params:oauth:grant-type:device_code",
    }),
  });
  if (!resp.ok) return { ok: false, retry: false, error: `HTTP ${resp.status}` };
  const data = await resp.json();
  if (data.access_token) return { ok: true, access_token: data.access_token };
  const err = String(data.error || "unknown");
  const retry = err === "authorization_pending" || err === "slow_down";
  return { ok: false, retry, interval: data.interval, error: err };
}
interface OauthState {
  phase: "idle" | "starting" | "awaiting_user" | "polling" | "success" | "error";
  user_code?: string;
  verification_uri?: string;
  error?: string;
}
function CopilotOauthBlock({
  oauth,
  hasKey,
  onSignIn,
  onCancel,
}: {
  oauth: OauthState;
  hasKey: boolean;
  onSignIn: () => void;
  onCancel: () => void;
}) {
  const isActive = oauth.phase === "starting" || oauth.phase === "awaiting_user" || oauth.phase === "polling";
  return (
    <div className="mt-2 rounded border border-border bg-card p-2 text-xs">
      <div className="flex items-center justify-between gap-2">
        <span className="text-muted-foreground">
          {hasKey
            ? "Token present. Sign in again to refresh it."
            : "Sign in with your GitHub account to fetch a Copilot token."}
        </span>
        {!isActive ? (
          <Button type="button" size="sm" className="text-xs" onClick={onSignIn}>
            Sign in with GitHub
          </Button>
        ) : (
          <Button type="button" size="sm" variant="outline" className="text-xs" onClick={onCancel}>
            Cancel
          </Button>
        )}
      </div>
      {oauth.phase === "starting" && <p className="mt-1 text-muted-foreground">Requesting device code…</p>}
      {(oauth.phase === "awaiting_user" || oauth.phase === "polling") && oauth.user_code && (
        <div className="mt-2 space-y-1">
          <p className="text-muted-foreground">
            Enter this code on{" "}
            <a
              href={oauth.verification_uri}
              target="_blank"
              rel="noopener noreferrer"
              className="underline hover:text-primary"
            >
              {oauth.verification_uri}
            </a>
            :
          </p>
          <div className="flex items-center gap-2">
            <code className="select-all rounded bg-background px-2 py-1 font-mono text-sm tracking-widest">
              {oauth.user_code}
            </code>
            <Button
              type="button"
              variant="outline"
              size="sm"
              className="text-xs px-2 h-7"
              onClick={() => void navigator.clipboard.writeText(oauth.user_code ?? "").catch(() => undefined)}
            >
              Copy
            </Button>
            <span className="text-xs text-muted-foreground">
              {oauth.phase === "polling" ? "Waiting for authorization…" : ""}
            </span>
          </div>
        </div>
      )}
      {oauth.phase === "success" && <p className="mt-1 text-emerald-500">Signed in. Click Save to store.</p>}
      {oauth.phase === "error" && <p className="mt-1 text-red-500">Sign-in failed: {oauth.error}</p>}
    </div>
  );
}
function parseProviders(raw: string): LlmProvider[] {
  if (!raw) return [];
  try {
    const v = JSON.parse(raw);
    if (!Array.isArray(v)) return [];
    return v
      .filter((x) => x && typeof x === "object" && typeof x.id === "string")
      .map((x) => ({
        id: String(x.id),
        name: String(x.name ?? ""),
        kind: (x.kind as ProviderKind) ?? "custom",
        base_url: String(x.base_url ?? ""),
        api_key: String(x.api_key ?? ""),
        default_model: String(x.default_model ?? ""),
        reasoning_effort: typeof x.reasoning_effort === "string" ? x.reasoning_effort : "",
      }));
  } catch {
    return [];
  }
}
export function ProvidersTab() {
  const [, dispatch] = useStore();
  const [providers, setProviders] = useState<LlmProvider[]>([]);
  const [activeId, setActiveId] = useState("");
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [draft, setDraft] = useState<LlmProvider | null>(null);
  const [busy, setBusy] = useState(false);
  const [msg, setMsg] = useState<string | null>(null);
  const [oauth, setOauth] = useState<OauthState>({ phase: "idle" });
  const oauthCancelRef = useRef<{ cancelled: boolean } | null>(null);

  useEffect(
    () => () => {
      if (oauthCancelRef.current) oauthCancelRef.current.cancelled = true;
    },
    [],
  );

  const load = useCallback(async () => {
    setMsg(null);
    try {
      const res = await shells.browser.llm.providers.get();
      const list = parseProviders(res.raw);
      setProviders(list);
      setActiveId(res.active_id);
    } catch (err) {
      setMsg(`load failed: ${(err as Error).message}`);
    }
  }, []);

  useEffect(() => {
    void load();
  }, [load]);

  const onSelect = useCallback(
    (id: string) => {
      const p = providers.find((x) => x.id === id);
      if (!p) return;
      setSelectedId(id);
      setDraft({ ...p });
      setMsg(null);
    },
    [providers],
  );

  const persist = useCallback(async (next: LlmProvider[], nextActive: string) => {
    await shells.browser.llm.providers.set({
      raw: JSON.stringify(next),
      active_id: nextActive,
    });
  }, []);

  const onAdd = useCallback((kind: ProviderKind = "openai") => {
    const p = newProvider(kind);
    setSelectedId(p.id);
    setDraft(p);
    setMsg(null);
  }, []);

  const onSave = useCallback(async () => {
    if (!draft) return;
    if (!draft.name.trim()) {
      setMsg("Name required.");
      return;
    }
    if (!draft.base_url.trim()) {
      setMsg("Base URL required.");
      return;
    }
    setBusy(true);
    setMsg(null);
    try {
      const exists = providers.some((p) => p.id === draft.id);
      const next = exists ? providers.map((p) => (p.id === draft.id ? draft : p)) : [...providers, draft];
      await persist(next, activeId || draft.id);
      setProviders(next);
      if (!activeId) setActiveId(draft.id);
      setMsg("Saved.");
    } catch (err) {
      setMsg(`save failed: ${(err as Error).message}`);
    } finally {
      setBusy(false);
    }
  }, [draft, providers, activeId, persist]);

  const onDelete = useCallback(async () => {
    if (!draft) return;
    // eslint-disable-next-line no-alert
    if (!confirm(`Delete provider "${draft.name}"?`)) return;
    setBusy(true);
    setMsg(null);
    try {
      const next = providers.filter((p) => p.id !== draft.id);
      const nextActive = activeId === draft.id ? "" : activeId;
      await persist(next, nextActive);
      setProviders(next);
      setActiveId(nextActive);
      setSelectedId(null);
      setDraft(null);
    } catch (err) {
      setMsg(`delete failed: ${(err as Error).message}`);
    } finally {
      setBusy(false);
    }
  }, [draft, providers, activeId, persist]);

  const onActivate = useCallback(
    async (p: LlmProvider) => {
      setBusy(true);
      setMsg(null);
      try {
        await persist(providers, p.id);
        setActiveId(p.id);
        await shells.browser.llm.config.set({
          base_url: p.base_url,
          api_key: p.api_key,
        });
        dispatch({
          type: "setLlmConfig",
          baseUrl: p.base_url,
          apiKey: p.api_key,
          model: p.default_model,
        });
        setMsg(`Active: ${p.name}`);
      } catch (err) {
        setMsg(`activate failed: ${(err as Error).message}`);
      } finally {
        setBusy(false);
      }
    },
    [providers, persist, dispatch],
  );

  const onKindChange = useCallback(
    (kind: ProviderKind) => {
      if (!draft) return;
      const preset = KIND_PRESETS[kind];
      setDraft({
        ...draft,
        kind,
        base_url: draft.base_url || preset.base_url,
        default_model: draft.default_model || preset.default_model,
      });
    },
    [draft],
  );

  const signInGithub = useCallback(async () => {
    if (!draft) return;
    if (oauthCancelRef.current) oauthCancelRef.current.cancelled = true;
    const cancelToken = { cancelled: false };
    oauthCancelRef.current = cancelToken;
    setMsg(null);
    setOauth({ phase: "starting" });
    let dc: DeviceCode;
    try {
      dc = await startGithubDeviceCode();
    } catch (err) {
      setOauth({ phase: "error", error: (err as Error).message });
      return;
    }
    if (cancelToken.cancelled) return;
    setOauth({
      phase: "awaiting_user",
      user_code: dc.user_code,
      verification_uri: dc.verification_uri,
    });
    try {
      await navigator.clipboard.writeText(dc.user_code);
    } catch {
      /* ok */
    }
    try {
      await shells.browser.shell.open_external({ url: dc.verification_uri });
    } catch {
      try {
        window.open(dc.verification_uri, "_blank", "noopener,noreferrer");
      } catch {
        /* ok */
      }
    }
    setOauth((s) => ({ ...s, phase: "polling" }));
    let interval = Math.max(dc.interval || 5, 1) * 1000;
    const deadline = Date.now() + dc.expires_in * 1000;
    while (!cancelToken.cancelled && Date.now() < deadline) {
      await new Promise((r) => setTimeout(r, interval));
      if (cancelToken.cancelled) return;
      const r = await pollGithubAccessToken(dc.device_code);
      if (r.ok) {
        setDraft((d) =>
          d
            ? {
                ...d,
                api_key: r.access_token,
                kind: "github_copilot",
                base_url: d.base_url || KIND_PRESETS.github_copilot.base_url,
                default_model: d.default_model || KIND_PRESETS.github_copilot.default_model,
              }
            : d,
        );
        setOauth({ phase: "success" });
        setMsg("GitHub sign-in complete. Click Save to store the token.");
        return;
      }
      if (!r.retry) {
        setOauth({ phase: "error", error: r.error });
        return;
      }
      if (r.error === "slow_down" && r.interval) interval = r.interval * 1000;
    }
    if (!cancelToken.cancelled) setOauth({ phase: "error", error: "code expired — try again" });
  }, [draft]);

  const cancelOauth = useCallback(() => {
    if (oauthCancelRef.current) oauthCancelRef.current.cancelled = true;
    setOauth({ phase: "idle" });
  }, []);

  return (
    <div className="flex h-full">
      <aside className="flex w-[220px] flex-col border-r border-border bg-card">
        <div className="flex items-center justify-between border-b border-border px-2 py-1.5">
          <span className="text-xs font-semibold">Providers</span>
          <div className="flex items-center gap-1">
            <Button
              type="button"
              variant="outline"
              size="sm"
              className="h-6 px-1.5 text-xs"
              onClick={() => onAdd("github_copilot")}
              title="Quick-add GitHub Copilot"
            >
              + Copilot
            </Button>
            <Button
              type="button"
              size="sm"
              className="h-6 px-1.5 text-xs"
              onClick={() => onAdd("openai")}
              title="New provider"
            >
              +
            </Button>
          </div>
        </div>
        <ul className="flex-1 overflow-auto py-1">
          {providers.length === 0 && (
            <li className="px-2 py-1 text-xs text-muted-foreground">No providers configured.</li>
          )}
          {providers.map((p) => {
            const isActive = p.id === activeId;
            const isSelected = p.id === selectedId;
            return (
              <li key={p.id}>
                <Button
                  type="button"
                  variant="ghost"
                  onClick={() => onSelect(p.id)}
                  className={
                    "flex h-auto w-full flex-col items-start gap-0 px-2 py-1 text-left text-xs font-normal " +
                    (isSelected
                      ? "bg-primary/15 text-foreground"
                      : "text-muted-foreground hover:bg-accent hover:text-foreground")
                  }
                >
                  <span className="flex w-full items-center gap-1 font-medium">
                    <span className="flex-1 truncate">{p.name}</span>
                    {isActive && <span className="rounded bg-green-500/20 px-1 text-xs text-green-300">active</span>}
                  </span>
                  <span className="text-xs opacity-70">
                    {p.kind} · {p.default_model || "—"}
                  </span>
                </Button>
              </li>
            );
          })}
        </ul>
      </aside>

      <section className="flex-1 overflow-auto p-3">
        {!draft && (
          <p className="text-xs text-muted-foreground">
            Select a provider to edit or activate it. Click <b>+</b> to add a new one. Credentials are stored in the
            workspace SQLite kv store.
          </p>
        )}
        {draft && (
          <div className="max-w-[560px]">
            <div className="mb-3 flex items-center justify-between">
              <h2 className="text-sm font-semibold">
                {providers.some((p) => p.id === draft.id) ? `Edit: ${draft.name || draft.id}` : "New provider"}
              </h2>
              {providers.some((p) => p.id === draft.id) && (
                <Button
                  type="button"
                  size="sm"
                  className="bg-green-600 hover:bg-green-500 text-white text-xs"
                  onClick={() => void onActivate(draft)}
                  disabled={busy || draft.id === activeId}
                >
                  {draft.id === activeId ? "Active" : "Activate"}
                </Button>
              )}
            </div>

            <Field label="Display name">
              <Input
                className="h-7 text-xs"
                value={draft.name}
                onChange={(e) => setDraft({ ...draft, name: e.target.value })}
                placeholder="My OpenAI"
              />
            </Field>
            <Field label="Kind">
              <Select value={draft.kind} onValueChange={(v) => onKindChange(v as ProviderKind)}>
                <SelectTrigger className="h-7 text-xs">
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  <SelectItem value="openai" className="text-xs">
                    OpenAI-compatible
                  </SelectItem>
                  <SelectItem value="anthropic" className="text-xs">
                    Anthropic
                  </SelectItem>
                  <SelectItem value="ollama" className="text-xs">
                    Ollama (local)
                  </SelectItem>
                  <SelectItem value="github_copilot" className="text-xs">
                    GitHub Copilot
                  </SelectItem>
                  <SelectItem value="custom" className="text-xs">
                    Custom
                  </SelectItem>
                </SelectContent>
              </Select>
            </Field>
            <Field label="Base URL">
              <Input
                className="h-7 text-xs"
                value={draft.base_url}
                onChange={(e) => setDraft({ ...draft, base_url: e.target.value })}
                placeholder="https://api.openai.com/v1"
              />
            </Field>
            <Field label="API key">
              <Input
                className="h-7 text-xs"
                type="password"
                value={draft.api_key}
                onChange={(e) => setDraft({ ...draft, api_key: e.target.value })}
                placeholder="sk-…"
                autoComplete="off"
              />
              {draft.kind === "github_copilot" && (
                <CopilotOauthBlock
                  oauth={oauth}
                  hasKey={!!draft.api_key}
                  onSignIn={signInGithub}
                  onCancel={cancelOauth}
                />
              )}
            </Field>
            <Field label="Default model">
              <ModelSelect
                value={draft.default_model}
                onChange={(v) => setDraft({ ...draft, default_model: v })}
                provider={draft}
              />
            </Field>
            <Field label="Reasoning effort">
              <Select
                value={draft.reasoning_effort || "none"}
                onValueChange={(v) => setDraft({ ...draft, reasoning_effort: v === "none" ? "" : v })}
              >
                <SelectTrigger
                  className="h-7 text-xs"
                  title="Default reasoning_effort for OpenAI gpt-5 / o-series. Per-message chat dropdown can override."
                >
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  <SelectItem value="none" className="text-xs">
                    none (use model default)
                  </SelectItem>
                  <SelectItem value="minimal" className="text-xs">
                    minimal
                  </SelectItem>
                  <SelectItem value="low" className="text-xs">
                    low
                  </SelectItem>
                  <SelectItem value="medium" className="text-xs">
                    medium
                  </SelectItem>
                  <SelectItem value="high" className="text-xs">
                    high
                  </SelectItem>
                  <SelectItem value="xhigh" className="text-xs">
                    xhigh
                  </SelectItem>
                </SelectContent>
              </Select>
            </Field>

            {msg && <p className="mb-3 text-xs text-muted-foreground">{msg}</p>}
            <div className="flex items-center gap-2">
              <Button type="button" size="sm" onClick={() => void onSave()} disabled={busy}>
                Save
              </Button>
              {providers.some((p) => p.id === draft.id) && (
                <Button type="button" size="sm" variant="destructive" onClick={() => void onDelete()} disabled={busy}>
                  Delete
                </Button>
              )}
              <Button
                type="button"
                variant="outline"
                size="sm"
                onClick={() => {
                  setSelectedId(null);
                  setDraft(null);
                  setMsg(null);
                }}
              >
                Cancel
              </Button>
            </div>
          </div>
        )}
      </section>
    </div>
  );
}
