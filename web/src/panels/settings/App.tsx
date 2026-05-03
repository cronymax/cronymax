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
import {
  useCallback,
  useEffect,
  useRef,
  useState,
  type KeyboardEvent,
} from "react";
import { bridge } from "@/bridge";
import { useBridgeEvent } from "@/hooks/useBridgeEvent";
import { useTheme } from "@/hooks/useTheme";
import type { ThemeMode } from "@/types";
import type {
  AgentGraphInstance,
  AgentTraceDetail,
  AgentRunSnapshot,
} from "@/agent_runtime";
import { Flows } from "@/components/FlowEditor";
import { useStore, type PermissionRequest } from "../agent/store";

// ── types ─────────────────────────────────────────────────────────────────

type SettingsTab =
  | "appearance"
  | "providers"
  | "agents"
  | "workspace"
  | "flows"
  | "runner";

// ── ReAct graph builder ───────────────────────────────────────────────────

function buildReActGraph(maxIters: number): AgentGraphInstance {
  const g = new window.AgentGraph();
  g.addLLMNode("llm", {
    system:
      "You are a helpful agent. Use tools when necessary. Reply with clear, concise text.",
  });
  g.addToolNode("tool", {});
  g.addConditionNode("cond", (run: AgentRunSnapshot) => {
    const has =
      (Array.isArray(run.tool_calls) && run.tool_calls.length > 0) ||
      run.finish_reason === "tool_calls";
    return has ? "llm" : null;
  });
  void maxIters;
  return g;
}

// ── shared input styles ───────────────────────────────────────────────────

const inputCls =
  "w-full rounded border border-cronymax-border bg-cronymax-surface px-2 py-1 text-xs text-cronymax-fg outline-none focus:border-cronymax-accent";

// ── shared Field ──────────────────────────────────────────────────────────

function Field({
  label,
  children,
}: {
  label: string;
  children: React.ReactNode;
}) {
  return (
    <div className="mb-3">
      <div className="mb-1 text-[11px] uppercase tracking-wide text-cronymax-fg-muted">
        {label}
      </div>
      {children}
    </div>
  );
}

// ── Appearance tab ────────────────────────────────────────────────────────

function AppearanceTab() {
  const { mode, setMode } = useTheme();
  return (
    <div className="p-4">
      <p className="mb-3 text-xs text-cronymax-fg-muted">
        System follows your macOS appearance and switches automatically.
      </p>
      <div className="flex gap-2">
        {(["system", "light", "dark"] as ThemeMode[]).map((m) => (
          <label
            key={m}
            className={`flex-1 cursor-pointer rounded border px-3 py-2 text-center text-xs capitalize transition-colors ${
              mode === m
                ? "border-cronymax-accent bg-cronymax-accent/10 text-cronymax-fg"
                : "border-cronymax-border bg-cronymax-surface text-cronymax-fg-muted hover:text-cronymax-fg"
            }`}
          >
            <input
              type="radio"
              name="theme-mode"
              value={m}
              checked={mode === m}
              onChange={() => setMode(m)}
              className="sr-only"
            />
            {m}
          </label>
        ))}
      </div>
    </div>
  );
}

// ── Providers tab ─────────────────────────────────────────────────────────

type ProviderKind =
  | "openai"
  | "anthropic"
  | "ollama"
  | "github_copilot"
  | "custom";

interface LlmProvider {
  id: string;
  name: string;
  kind: ProviderKind;
  base_url: string;
  api_key: string;
  default_model: string;
}

const KIND_PRESETS: Record<
  ProviderKind,
  { base_url: string; default_model: string; display: string }
> = {
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
  const id =
    "p_" +
    Date.now().toString(36) +
    "_" +
    Math.random().toString(36).slice(2, 7);
  return {
    id,
    name: preset.display,
    kind,
    base_url: preset.base_url,
    api_key: "",
    default_model: preset.default_model,
  };
}

const ANTHROPIC_MODELS = [
  "claude-opus-4-5",
  "claude-sonnet-4-5",
  "claude-3-5-sonnet-latest",
  "claude-3-5-haiku-latest",
  "claude-3-opus-20240229",
];

async function listProviderModels(provider: LlmProvider): Promise<string[]> {
  const { kind, base_url, api_key } = provider;
  if (!base_url) return [];
  if (kind === "anthropic") return ANTHROPIC_MODELS;
  if (kind === "ollama") {
    const base = base_url.replace(/\/v1\/?$/, "");
    const res = await fetch(`${base}/api/tags`, {
      signal: AbortSignal.timeout(8000),
    });
    if (!res.ok) throw new Error(`/api/tags ${res.status}`);
    const data = await res.json();
    return ((data.models ?? []) as { name: string }[])
      .map((m) => m.name)
      .sort();
  }
  const headers: Record<string, string> = { Accept: "application/json" };
  if (api_key) headers["Authorization"] = `Bearer ${api_key}`;
  const url = base_url.replace(/\/?$/, "") + "/models";
  const res = await fetch(url, { headers, signal: AbortSignal.timeout(8000) });
  if (!res.ok) throw new Error(`/models ${res.status}`);
  const data = await res.json();
  return ((data.data ?? []) as { id: string }[]).map((m) => m.id).sort();
}

function ModelSelect({
  value,
  onChange,
  provider,
}: {
  value: string;
  onChange: (v: string) => void;
  provider: LlmProvider;
}) {
  const [models, setModels] = useState<string[]>([]);
  const [fetching, setFetching] = useState(false);
  const [fetchErr, setFetchErr] = useState<string | null>(null);

  const doFetch = useCallback(async () => {
    setFetching(true);
    setFetchErr(null);
    try {
      const list = await listProviderModels(provider);
      setModels(list);
    } catch (e) {
      setFetchErr((e as Error).message);
    } finally {
      setFetching(false);
    }
  }, [provider.kind, provider.base_url, provider.api_key]);

  useEffect(() => {
    if (provider.base_url) void doFetch();
  }, [provider.kind, provider.base_url]);

  return (
    <div className="space-y-1">
      <div className="flex gap-1">
        <input
          className={`${inputCls} flex-1`}
          value={value}
          onChange={(e) => onChange(e.target.value)}
          list="model-datalist"
          placeholder="gpt-4o-mini"
        />
        <datalist id="model-datalist">
          {models.map((m) => (
            <option key={m} value={m} />
          ))}
        </datalist>
        <button
          type="button"
          onClick={() => void doFetch()}
          disabled={fetching || !provider.base_url}
          title="Fetch available models"
          className="rounded border border-cronymax-border px-2 text-xs hover:bg-cronymax-surface-2 disabled:opacity-40"
        >
          {fetching ? "…" : "⟳"}
        </button>
      </div>
      {fetchErr && (
        <p className="text-[11px] text-red-400">fetch failed: {fetchErr}</p>
      )}
      {models.length > 0 && !fetching && (
        <div className="flex max-h-[120px] flex-wrap gap-1 overflow-y-auto pt-0.5">
          {models.map((m) => (
            <button
              key={m}
              type="button"
              onClick={() => onChange(m)}
              className={
                "rounded border px-1.5 py-0.5 text-[11px] " +
                (m === value
                  ? "border-cronymax-accent bg-cronymax-accent/20 text-cronymax-fg"
                  : "border-cronymax-border text-cronymax-fg-muted hover:text-cronymax-fg")
              }
            >
              {m}
            </button>
          ))}
        </div>
      )}
    </div>
  );
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
  if (!data.device_code)
    throw new Error(data.error_description || "no device_code");
  return data as DeviceCode;
}

async function pollGithubAccessToken(
  device_code: string,
): Promise<
  | { ok: true; access_token: string }
  | { ok: false; retry: boolean; interval?: number; error: string }
> {
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
  if (!resp.ok)
    return { ok: false, retry: false, error: `HTTP ${resp.status}` };
  const data = await resp.json();
  if (data.access_token) return { ok: true, access_token: data.access_token };
  const err = String(data.error || "unknown");
  const retry = err === "authorization_pending" || err === "slow_down";
  return { ok: false, retry, interval: data.interval, error: err };
}

interface OauthState {
  phase:
    | "idle"
    | "starting"
    | "awaiting_user"
    | "polling"
    | "success"
    | "error";
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
  const isActive =
    oauth.phase === "starting" ||
    oauth.phase === "awaiting_user" ||
    oauth.phase === "polling";
  return (
    <div className="mt-2 rounded border border-cronymax-border bg-cronymax-surface-2 p-2 text-xs">
      <div className="flex items-center justify-between gap-2">
        <span className="text-cronymax-fg-muted">
          {hasKey
            ? "Token present. Sign in again to refresh it."
            : "Sign in with your GitHub account to fetch a Copilot token."}
        </span>
        {!isActive ? (
          <button
            type="button"
            onClick={onSignIn}
            className="rounded bg-cronymax-accent px-2 py-0.5 text-[11px] text-white hover:opacity-90"
          >
            Sign in with GitHub
          </button>
        ) : (
          <button
            type="button"
            onClick={onCancel}
            className="rounded border border-cronymax-border px-2 py-0.5 text-[11px] text-cronymax-fg hover:bg-cronymax-surface"
          >
            Cancel
          </button>
        )}
      </div>
      {oauth.phase === "starting" && (
        <p className="mt-1 text-cronymax-fg-muted">Requesting device code…</p>
      )}
      {(oauth.phase === "awaiting_user" || oauth.phase === "polling") &&
        oauth.user_code && (
          <div className="mt-2 space-y-1">
            <p className="text-cronymax-fg-muted">
              Enter this code on{" "}
              <a
                href={oauth.verification_uri}
                target="_blank"
                rel="noopener noreferrer"
                className="underline hover:text-cronymax-accent"
              >
                {oauth.verification_uri}
              </a>
              :
            </p>
            <div className="flex items-center gap-2">
              <code className="select-all rounded bg-cronymax-surface px-2 py-1 font-mono text-sm tracking-widest">
                {oauth.user_code}
              </code>
              <button
                type="button"
                onClick={() =>
                  void navigator.clipboard
                    .writeText(oauth.user_code ?? "")
                    .catch(() => undefined)
                }
                className="rounded border border-cronymax-border px-2 py-0.5 text-[11px] hover:bg-cronymax-surface"
              >
                Copy
              </button>
              <span className="text-[11px] text-cronymax-fg-muted">
                {oauth.phase === "polling" ? "Waiting for authorization…" : ""}
              </span>
            </div>
          </div>
        )}
      {oauth.phase === "success" && (
        <p className="mt-1 text-emerald-500">Signed in. Click Save to store.</p>
      )}
      {oauth.phase === "error" && (
        <p className="mt-1 text-red-500">Sign-in failed: {oauth.error}</p>
      )}
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
      }));
  } catch {
    return [];
  }
}

function ProvidersTab() {
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
      const res = await bridge.send("llm.providers.get");
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

  const persist = useCallback(
    async (next: LlmProvider[], nextActive: string) => {
      await bridge.send("llm.providers.set", {
        raw: JSON.stringify(next),
        active_id: nextActive,
      });
    },
    [],
  );

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
      const next = exists
        ? providers.map((p) => (p.id === draft.id ? draft : p))
        : [...providers, draft];
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
        await bridge.send("llm.config.set", {
          base_url: p.base_url,
          api_key: p.api_key,
        });
        if (window.llmClient) {
          window.llmClient.baseUrl = p.base_url;
          window.llmClient.apiKey = p.api_key;
          if (p.default_model) window.llmClient.model = p.default_model;
        }
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
      await bridge.send("shell.popover_open", { url: dc.verification_uri });
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
                default_model:
                  d.default_model || KIND_PRESETS.github_copilot.default_model,
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
    if (!cancelToken.cancelled)
      setOauth({ phase: "error", error: "code expired — try again" });
  }, [draft]);

  const cancelOauth = useCallback(() => {
    if (oauthCancelRef.current) oauthCancelRef.current.cancelled = true;
    setOauth({ phase: "idle" });
  }, []);

  return (
    <div className="flex h-full">
      <aside className="flex w-[220px] flex-col border-r border-cronymax-border bg-cronymax-surface-2">
        <div className="flex items-center justify-between border-b border-cronymax-border px-2 py-1.5">
          <span className="text-xs font-semibold">Providers</span>
          <div className="flex items-center gap-1">
            <button
              type="button"
              onClick={() => onAdd("github_copilot")}
              className="rounded border border-cronymax-border bg-cronymax-surface px-1.5 py-0.5 text-[11px] text-cronymax-fg hover:bg-cronymax-surface-2"
              title="Quick-add GitHub Copilot"
            >
              + Copilot
            </button>
            <button
              type="button"
              onClick={() => onAdd("openai")}
              className="rounded bg-cronymax-accent px-1.5 py-0.5 text-xs text-white hover:opacity-90"
              title="New provider"
            >
              +
            </button>
          </div>
        </div>
        <ul className="flex-1 overflow-auto py-1">
          {providers.length === 0 && (
            <li className="px-2 py-1 text-[11px] text-cronymax-fg-muted">
              No providers configured.
            </li>
          )}
          {providers.map((p) => {
            const isActive = p.id === activeId;
            const isSelected = p.id === selectedId;
            return (
              <li key={p.id}>
                <button
                  type="button"
                  onClick={() => onSelect(p.id)}
                  className={
                    "flex w-full flex-col items-start gap-0 px-2 py-1 text-left text-xs " +
                    (isSelected
                      ? "bg-cronymax-accent/15 text-cronymax-fg"
                      : "text-cronymax-fg-muted hover:bg-cronymax-surface hover:text-cronymax-fg")
                  }
                >
                  <span className="flex w-full items-center gap-1 font-medium">
                    <span className="flex-1 truncate">{p.name}</span>
                    {isActive && (
                      <span className="rounded bg-green-500/20 px-1 text-[10px] text-green-300">
                        active
                      </span>
                    )}
                  </span>
                  <span className="text-[10px] opacity-70">
                    {p.kind} · {p.default_model || "—"}
                  </span>
                </button>
              </li>
            );
          })}
        </ul>
      </aside>

      <section className="flex-1 overflow-auto p-3">
        {!draft && (
          <p className="text-xs text-cronymax-fg-muted">
            Select a provider to edit or activate it. Click <b>+</b> to add a
            new one. Credentials are stored in the workspace SQLite kv store.
          </p>
        )}
        {draft && (
          <div className="max-w-[560px]">
            <div className="mb-3 flex items-center justify-between">
              <h2 className="text-sm font-semibold">
                {providers.some((p) => p.id === draft.id)
                  ? `Edit: ${draft.name || draft.id}`
                  : "New provider"}
              </h2>
              {providers.some((p) => p.id === draft.id) && (
                <button
                  type="button"
                  onClick={() => void onActivate(draft)}
                  disabled={busy || draft.id === activeId}
                  className="rounded bg-green-500/80 px-3 py-1 text-xs font-medium text-white hover:bg-green-500 disabled:opacity-50"
                >
                  {draft.id === activeId ? "Active" : "Activate"}
                </button>
              )}
            </div>

            <Field label="Display name">
              <input
                className={inputCls}
                value={draft.name}
                onChange={(e) => setDraft({ ...draft, name: e.target.value })}
                placeholder="My OpenAI"
              />
            </Field>
            <Field label="Kind">
              <select
                className={inputCls}
                value={draft.kind}
                onChange={(e) => onKindChange(e.target.value as ProviderKind)}
              >
                <option value="openai">OpenAI-compatible</option>
                <option value="anthropic">Anthropic</option>
                <option value="ollama">Ollama (local)</option>
                <option value="github_copilot">GitHub Copilot</option>
                <option value="custom">Custom</option>
              </select>
            </Field>
            <Field label="Base URL">
              <input
                className={inputCls}
                value={draft.base_url}
                onChange={(e) =>
                  setDraft({ ...draft, base_url: e.target.value })
                }
                placeholder="https://api.openai.com/v1"
              />
            </Field>
            <Field label="API key">
              <input
                className={inputCls}
                type="password"
                value={draft.api_key}
                onChange={(e) =>
                  setDraft({ ...draft, api_key: e.target.value })
                }
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

            {msg && (
              <p className="mb-3 text-xs text-cronymax-fg-muted">{msg}</p>
            )}
            <div className="flex items-center gap-2">
              <button
                type="button"
                onClick={() => void onSave()}
                disabled={busy}
                className="rounded bg-cronymax-accent px-3 py-1 text-xs font-medium text-white hover:opacity-90 disabled:opacity-50"
              >
                Save
              </button>
              {providers.some((p) => p.id === draft.id) && (
                <button
                  type="button"
                  onClick={() => void onDelete()}
                  disabled={busy}
                  className="rounded border border-red-500/50 bg-red-500/10 px-3 py-1 text-xs text-red-300 hover:bg-red-500/20 disabled:opacity-50"
                >
                  Delete
                </button>
              )}
              <button
                type="button"
                onClick={() => {
                  setSelectedId(null);
                  setDraft(null);
                  setMsg(null);
                }}
                className="rounded border border-cronymax-border bg-cronymax-surface px-3 py-1 text-xs hover:bg-cronymax-surface-2"
              >
                Cancel
              </button>
            </div>
          </div>
        )}
      </section>
    </div>
  );
}

// ── Agents tab ────────────────────────────────────────────────────────────

interface AgentSummary {
  name: string;
  kind: string;
  llm: string;
}

interface AgentDetail {
  name: string;
  kind: string;
  llm: string;
  system_prompt: string;
  memory_namespace: string;
  tools: string[];
}

const EMPTY_DETAIL: AgentDetail = {
  name: "",
  kind: "worker",
  llm: "gpt-4o-mini",
  system_prompt: "You are a helpful agent.",
  memory_namespace: "",
  tools: [],
};

function AgentsTab() {
  const [agents, setAgents] = useState<AgentSummary[]>([]);
  const [selected, setSelected] = useState<string | null>(null);
  const [draft, setDraft] = useState<AgentDetail | null>(null);
  const [creating, setCreating] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [activeProvider, setActiveProvider] = useState<LlmProvider | null>(
    null,
  );

  useEffect(() => {
    bridge.send("llm.providers.get").then((res) => {
      try {
        const list = JSON.parse(res.raw || "[]") as LlmProvider[];
        const p = list.find((x) => x.id === res.active_id) ?? list[0] ?? null;
        setActiveProvider(p);
      } catch {
        /* ignore */
      }
    });
  }, []);

  const loadList = useCallback(async () => {
    try {
      let res = await bridge.send("agent.registry.list");
      if ((res.agents ?? []).length === 0) {
        await bridge.send("agent.registry.save", {
          name: "Chat",
          kind: "worker",
          llm: "",
          system_prompt: "You are a helpful assistant.",
          memory_namespace: "",
          tools_csv: "",
        });
        res = await bridge.send("agent.registry.list");
      }
      setAgents(res.agents ?? []);
    } catch (err) {
      setError(`agent.registry.list: ${(err as Error).message}`);
    }
  }, []);

  const loadDetail = useCallback(async (name: string) => {
    try {
      const res = await bridge.send("agent.registry.load", { name });
      setDraft({
        name: res.name,
        kind: res.kind,
        llm: res.llm,
        system_prompt: res.system_prompt,
        memory_namespace: res.memory_namespace ?? "",
        tools: res.tools ?? [],
      });
      setCreating(false);
      setError(null);
    } catch (err) {
      setError(`agent.registry.load: ${(err as Error).message}`);
    }
  }, []);

  useEffect(() => {
    void loadList();
  }, [loadList]);

  const onSelect = useCallback(
    (name: string) => {
      setSelected(name);
      void loadDetail(name);
    },
    [loadDetail],
  );

  const onNew = useCallback(() => {
    setSelected(null);
    setCreating(true);
    setDraft({ ...EMPTY_DETAIL });
    setError(null);
  }, []);

  const onSave = useCallback(async () => {
    if (!draft) return;
    if (!/^[A-Za-z0-9_.-]{1,64}$/.test(draft.name)) {
      setError(
        "Name must be 1-64 chars of letters, digits, _, -, or . (no slashes).",
      );
      return;
    }
    setBusy(true);
    setError(null);
    try {
      await bridge.send("agent.registry.save", {
        name: draft.name,
        kind: draft.kind === "reviewer" ? "reviewer" : "worker",
        llm: draft.llm,
        system_prompt: draft.system_prompt,
        memory_namespace: draft.memory_namespace,
        tools_csv: draft.tools.join(","),
      });
      await loadList();
      setSelected(draft.name);
      setCreating(false);
    } catch (err) {
      setError(`save failed: ${(err as Error).message}`);
    } finally {
      setBusy(false);
    }
  }, [draft, loadList]);

  const onDelete = useCallback(async () => {
    if (!selected) return;
    // eslint-disable-next-line no-alert
    if (!confirm(`Delete agent "${selected}" YAML file?`)) return;
    setBusy(true);
    setError(null);
    try {
      await bridge.send("agent.registry.delete", { name: selected });
      await loadList();
      setSelected(null);
      setDraft(null);
    } catch (err) {
      setError(`delete failed: ${(err as Error).message}`);
    } finally {
      setBusy(false);
    }
  }, [selected, loadList]);

  const taCls = inputCls + " min-h-[160px] resize-y font-mono";

  return (
    <div className="flex h-full">
      <aside className="flex w-[200px] flex-col border-r border-cronymax-border bg-cronymax-surface-2">
        <div className="flex items-center justify-between border-b border-cronymax-border px-2 py-1.5">
          <span className="text-xs font-semibold">Agents</span>
          <button
            type="button"
            onClick={onNew}
            className="rounded bg-cronymax-accent px-1.5 py-0.5 text-xs text-white hover:opacity-90"
            title="New agent"
          >
            +
          </button>
        </div>
        <ul className="flex-1 overflow-auto py-1">
          {agents.length === 0 && (
            <li className="px-2 py-1 text-[11px] text-cronymax-fg-muted">
              No agents registered.
            </li>
          )}
          {agents.map((a) => (
            <li key={a.name}>
              <button
                type="button"
                onClick={() => onSelect(a.name)}
                className={
                  "flex w-full flex-col items-start px-2 py-1 text-left text-xs " +
                  (selected === a.name && !creating
                    ? "bg-cronymax-accent/15 text-cronymax-fg"
                    : "text-cronymax-fg-muted hover:bg-cronymax-surface hover:text-cronymax-fg")
                }
              >
                <span className="font-medium">{a.name}</span>
                <span className="text-[10px] opacity-70">
                  {a.kind} · {a.llm}
                </span>
              </button>
            </li>
          ))}
        </ul>
      </aside>

      <section className="flex-1 overflow-auto p-3">
        {!draft && (
          <p className="text-xs text-cronymax-fg-muted">
            Select an agent to view or edit, or click <b>+</b> to create one.
            Files live under{" "}
            <code>.cronymax/agents/&lt;name&gt;.agent.yaml</code>.
          </p>
        )}
        {draft && (
          <div className="max-w-[560px]">
            <h2 className="mb-3 text-sm font-semibold">
              {creating ? "New agent" : `Edit: ${selected}`}
            </h2>
            <Field label="Name (file basename)">
              <input
                className={inputCls}
                value={draft.name}
                disabled={!creating}
                onChange={(e) => setDraft({ ...draft, name: e.target.value })}
                placeholder="my_worker"
              />
              {!creating && (
                <p className="mt-1 text-[10px] text-cronymax-fg-muted">
                  Rename by deleting and recreating.
                </p>
              )}
            </Field>
            <Field label="Kind">
              <select
                className={inputCls}
                value={draft.kind}
                onChange={(e) => setDraft({ ...draft, kind: e.target.value })}
              >
                <option value="worker">worker</option>
                <option value="reviewer">reviewer</option>
              </select>
            </Field>
            <Field label="LLM model">
              {activeProvider ? (
                <ModelSelect
                  value={draft.llm}
                  onChange={(v) => setDraft({ ...draft, llm: v })}
                  provider={activeProvider}
                />
              ) : (
                <input
                  className={inputCls}
                  value={draft.llm}
                  onChange={(e) => setDraft({ ...draft, llm: e.target.value })}
                  placeholder="(uses provider default)"
                />
              )}
            </Field>
            <Field label="Memory namespace (optional)">
              <input
                className={inputCls}
                value={draft.memory_namespace}
                onChange={(e) =>
                  setDraft({ ...draft, memory_namespace: e.target.value })
                }
                placeholder="(defaults to agent name)"
              />
            </Field>
            <Field label="System prompt">
              <textarea
                className={taCls}
                value={draft.system_prompt}
                onChange={(e) =>
                  setDraft({ ...draft, system_prompt: e.target.value })
                }
              />
            </Field>
            <Field label="Tools (comma-separated; empty = Space defaults)">
              <input
                className={inputCls}
                value={draft.tools.join(",")}
                onChange={(e) =>
                  setDraft({
                    ...draft,
                    tools: e.target.value
                      .split(",")
                      .map((s) => s.trim())
                      .filter(Boolean),
                  })
                }
                placeholder="terminal_exec, file_read"
              />
            </Field>
            {error && <p className="mb-3 text-xs text-red-300">{error}</p>}
            <div className="flex items-center gap-2">
              <button
                type="button"
                onClick={() => void onSave()}
                disabled={busy}
                className="rounded bg-cronymax-accent px-3 py-1 text-xs font-medium text-white hover:opacity-90 disabled:opacity-50"
              >
                {creating ? "Create" : "Save"}
              </button>
              {!creating && (
                <button
                  type="button"
                  onClick={() => void onDelete()}
                  disabled={busy}
                  className="rounded border border-red-500/50 bg-red-500/10 px-3 py-1 text-xs text-red-300 hover:bg-red-500/20 disabled:opacity-50"
                >
                  Delete
                </button>
              )}
              <button
                type="button"
                onClick={() => {
                  setDraft(null);
                  setCreating(false);
                  setSelected(null);
                  setError(null);
                }}
                className="rounded border border-cronymax-border bg-cronymax-surface px-3 py-1 text-xs hover:bg-cronymax-surface-2"
              >
                Cancel
              </button>
            </div>
          </div>
        )}
      </section>
    </div>
  );
}

// ── Workspace tab ─────────────────────────────────────────────────────────

interface WorkspaceProfile {
  space_id: string;
  space_name: string;
  workspace_root: string;
  allow_network: boolean;
  extra_read_paths: string[];
  extra_write_paths: string[];
  extra_deny_paths: string[];
}

function WorkspaceTab() {
  const [profile, setProfile] = useState<WorkspaceProfile | null>(null);
  const [reads, setReads] = useState("");
  const [writes, setWrites] = useState("");
  const [denies, setDenies] = useState("");
  const [allowNet, setAllowNet] = useState(false);
  const [busy, setBusy] = useState(false);
  const [msg, setMsg] = useState<string | null>(null);

  const reload = useCallback(async () => {
    setMsg(null);
    try {
      const res = await bridge.send("space.profile.get");
      setProfile(res);
      setAllowNet(res.allow_network);
      setReads(res.extra_read_paths.join("\n"));
      setWrites(res.extra_write_paths.join("\n"));
      setDenies(res.extra_deny_paths.join("\n"));
    } catch (err) {
      setMsg(`load failed: ${(err as Error).message}`);
    }
  }, []);

  useEffect(() => {
    void reload();
  }, [reload]);
  useBridgeEvent("shell.space_changed", () => void reload());

  const onSave = useCallback(async () => {
    setBusy(true);
    setMsg(null);
    try {
      await bridge.send("space.profile.set", {
        allow_network: allowNet,
        extra_read_paths_nl: reads,
        extra_write_paths_nl: writes,
        extra_deny_paths_nl: denies,
      });
      setMsg("Saved.");
      await reload();
    } catch (err) {
      setMsg(`save failed: ${(err as Error).message}`);
    } finally {
      setBusy(false);
    }
  }, [allowNet, reads, writes, denies, reload]);

  const taCls =
    "w-full min-h-[100px] resize-y rounded border border-cronymax-border " +
    "bg-cronymax-surface px-2 py-1 font-mono text-xs text-cronymax-fg " +
    "outline-none focus:border-cronymax-accent";

  return (
    <div className="h-full overflow-auto p-4">
      {!profile ? (
        <p className="text-xs text-cronymax-fg-muted">Loading profile…</p>
      ) : (
        <div className="max-w-[600px]">
          <h2 className="mb-1 text-sm font-semibold">{profile.space_name}</h2>
          <p className="mb-4 break-all text-[11px] text-cronymax-fg-muted">
            <code>{profile.workspace_root}</code>
          </p>
          <p className="mb-4 rounded border border-cronymax-border bg-cronymax-surface-2 p-2 text-[11px] text-cronymax-fg-muted">
            Overrides supplement the default sandbox rules. Persisted to{" "}
            <code>.cronymax/space.profile.yaml</code>.
          </p>
          <Field label="Network">
            <label className="flex items-center gap-2 text-xs">
              <input
                type="checkbox"
                checked={allowNet}
                onChange={(e) => setAllowNet(e.target.checked)}
              />
              Allow outbound network access
            </label>
          </Field>
          <Field label="Extra readable paths (one per line)">
            <textarea
              className={taCls}
              value={reads}
              onChange={(e) => setReads(e.target.value)}
              placeholder="/Users/me/datasets"
              spellCheck={false}
            />
          </Field>
          <Field label="Extra writable paths (one per line)">
            <textarea
              className={taCls}
              value={writes}
              onChange={(e) => setWrites(e.target.value)}
              placeholder="/Users/me/scratch"
              spellCheck={false}
            />
          </Field>
          <Field label="Extra denied paths (one per line)">
            <textarea
              className={taCls}
              value={denies}
              onChange={(e) => setDenies(e.target.value)}
              placeholder="/Users/me/secrets"
              spellCheck={false}
            />
          </Field>
          {msg && <p className="mb-3 text-xs text-cronymax-fg-muted">{msg}</p>}
          <div className="flex items-center gap-2">
            <button
              type="button"
              onClick={() => void onSave()}
              disabled={busy}
              className="rounded bg-cronymax-accent px-3 py-1 text-xs font-medium text-white hover:opacity-90 disabled:opacity-50"
            >
              Save profile
            </button>
            <button
              type="button"
              onClick={() => void reload()}
              disabled={busy}
              className="rounded border border-cronymax-border bg-cronymax-surface px-3 py-1 text-xs text-cronymax-fg hover:bg-cronymax-surface-2"
            >
              Reload
            </button>
          </div>
        </div>
      )}
    </div>
  );
}

// ── Runner tab ────────────────────────────────────────────────────────────

function SpaceRow({
  space,
  active,
  onActivate,
  onDelete,
}: {
  space: { id: string; name: string };
  active: boolean;
  onActivate: () => void;
  onDelete: () => void;
}) {
  return (
    <li
      onClick={onActivate}
      className={
        "group flex h-7 cursor-pointer items-center gap-1.5 rounded px-2 text-xs " +
        (active
          ? "bg-cronymax-surface-2 text-cronymax-fg"
          : "text-cronymax-fg-muted hover:bg-cronymax-surface-2 hover:text-cronymax-fg")
      }
    >
      <span className="flex-1 truncate">{space.name}</span>
      <button
        type="button"
        onClick={(e) => {
          e.stopPropagation();
          onDelete();
        }}
        className="opacity-0 transition group-hover:opacity-100"
        title="Delete space"
      >
        ×
      </button>
    </li>
  );
}

function RunnerTab() {
  const [state, dispatch] = useStore();
  const taskRef = useRef<HTMLTextAreaElement>(null);

  const loadSpaces = useCallback(async () => {
    try {
      const spaces = await bridge.send("space.list");
      dispatch({ type: "setSpaces", spaces });
    } catch (e) {
      console.warn("space.list failed", e);
    }
  }, [dispatch]);

  useEffect(() => {
    void loadSpaces();
  }, [loadSpaces]);
  useBridgeEvent("space.created", () => void loadSpaces());
  useBridgeEvent("space.deleted", () => void loadSpaces());

  const switchSpace = useCallback(
    async (id: string) => {
      try {
        await bridge.send("space.switch", { space_id: id });
        dispatch({ type: "setActiveSpace", id });
      } catch (e) {
        console.warn("space.switch failed", e);
      }
    },
    [dispatch],
  );

  const deleteSpace = useCallback(
    async (id: string, name: string) => {
      // eslint-disable-next-line no-alert
      if (!confirm(`Delete space "${name}"?`)) return;
      try {
        await bridge.send("space.delete", { space_id: id });
        await loadSpaces();
      } catch (e) {
        console.warn("space.delete failed", e);
      }
    },
    [loadSpaces],
  );

  const newSpace = useCallback(async () => {
    // eslint-disable-next-line no-alert
    const name = prompt("Space name:");
    if (!name) return;
    // eslint-disable-next-line no-alert
    const root = prompt("Root path:", "/");
    if (!root) return;
    try {
      await bridge.send("space.create", { name, root_path: root });
      await loadSpaces();
    } catch (e) {
      console.warn("space.create failed", e);
    }
  }, [loadSpaces]);

  const runTask = useCallback(async () => {
    const text = state.task.trim();
    if (!text) {
      taskRef.current?.focus();
      return;
    }
    dispatch({ type: "setStatus", status: "running" });
    dispatch({ type: "resetResult" });
    const graph = buildReActGraph(10);
    graph.addEventListener("trace", (e) => {
      const d: AgentTraceDetail = e.detail;
      if (d.type === "llm_delta" && d.content) {
        dispatch({ type: "appendResult", chunk: d.content });
      } else if (d.type === "tool_start") {
        dispatch({ type: "appendResult", chunk: `\n[tool: ${d.tool}]\n` });
      } else if (d.type === "tool_done" && d.output) {
        dispatch({ type: "appendResult", chunk: d.output + "\n" });
      } else if (d.type === "error") {
        dispatch({
          type: "appendResult",
          chunk: `\n[error] ${d.message ?? ""}\n`,
        });
        dispatch({ type: "setStatus", status: "failed" });
      } else if (d.type === "human_request" && d.prompt) {
        void window.__getPermission?.(d.prompt, d.request_id ?? "");
      } else if (d.type === "done") {
        dispatch({ type: "setStatus", status: "done" });
      }
    });
    try {
      await graph.run({ task: text, getPermission: window.__getPermission });
    } catch (err) {
      dispatch({ type: "appendResult", chunk: "\n" + (err as Error).message });
      dispatch({ type: "setStatus", status: "failed" });
    }
  }, [state.task, dispatch]);

  useBridgeEvent("agent.task_from_command", (data) => {
    const action = data.action || "Explain";
    const cmd = data.command || "";
    const out = (data.output || "").slice(0, 1000);
    const ec = data.exit_code ?? -1;
    const text = `${action} the following terminal command and its output.\n\nCommand: ${cmd}\nExit code: ${ec}\nOutput:\n${out}`;
    dispatch({ type: "setTask", task: text });
    setTimeout(() => void runTask(), 0);
  });

  const onTaskKeyDown = useCallback(
    (e: KeyboardEvent<HTMLTextAreaElement>) => {
      if ((e.metaKey || e.ctrlKey) && e.key === "Enter") {
        e.preventDefault();
        void runTask();
      }
    },
    [runTask],
  );

  return (
    <div className="flex h-full flex-col">
      <section className="border-b border-cronymax-border px-3 py-2">
        <div className="mb-1 flex items-center justify-between text-xs text-cronymax-fg-muted">
          <span>Spaces</span>
          <button
            type="button"
            onClick={() => void newSpace()}
            className="rounded bg-cronymax-surface px-1.5 text-cronymax-fg hover:bg-cronymax-surface-2"
          >
            +
          </button>
        </div>
        <ul className="flex flex-col gap-px">
          {state.spaces.map((sp) => (
            <SpaceRow
              key={sp.id}
              space={sp}
              active={sp.id === state.activeSpaceId}
              onActivate={() => void switchSpace(sp.id)}
              onDelete={() => void deleteSpace(sp.id, sp.name)}
            />
          ))}
        </ul>
      </section>
      <textarea
        ref={taskRef}
        value={state.task}
        onChange={(e) => dispatch({ type: "setTask", task: e.target.value })}
        onKeyDown={onTaskKeyDown}
        spellCheck={false}
        placeholder="Ask the agent…  (⌘/Ctrl+Enter to run)"
        className="m-3 min-h-[80px] resize-y rounded border border-cronymax-border bg-cronymax-surface-2 p-2 text-sm text-cronymax-fg outline-none focus:border-cronymax-accent"
      />
      <div className="flex justify-end gap-2 px-3">
        <button
          type="button"
          onClick={() => void runTask()}
          disabled={state.status === "running"}
          className="rounded bg-cronymax-accent px-3 py-1 text-sm font-medium text-white transition hover:opacity-90 disabled:cursor-not-allowed disabled:opacity-50"
        >
          Run
        </button>
      </div>
      <pre className="m-3 flex-1 overflow-auto whitespace-pre-wrap break-words rounded border border-cronymax-border bg-cronymax-surface-2 p-2 text-xs text-cronymax-fg">
        {state.result}
      </pre>
    </div>
  );
}

// ── Permission overlay ────────────────────────────────────────────────────

function PermissionOverlay({
  perm,
  onResolve,
}: {
  perm: PermissionRequest;
  onResolve: (allow: boolean) => void;
}) {
  return (
    <div className="absolute inset-0 z-50 flex items-center justify-center bg-black/40">
      <div className="w-[340px] rounded-md border border-cronymax-border bg-cronymax-surface-2 p-4 text-sm text-cronymax-fg shadow-lg">
        <p className="mb-3 whitespace-pre-wrap">{perm.prompt}</p>
        <div className="flex justify-end gap-2">
          <button
            type="button"
            onClick={() => onResolve(true)}
            className="rounded bg-cronymax-accent px-3 py-1 text-xs font-medium text-white hover:opacity-90"
          >
            Allow
          </button>
          <button
            type="button"
            onClick={() => onResolve(false)}
            className="rounded border border-cronymax-border bg-cronymax-surface px-3 py-1 text-xs text-cronymax-fg hover:bg-cronymax-surface-2"
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
  { id: "agents", label: "Agents" },
  { id: "workspace", label: "Workspace" },
  { id: "flows", label: "Flows" },
  { id: "runner", label: "Runner" },
];

function TabBar({
  tab,
  onChange,
}: {
  tab: SettingsTab;
  onChange: (t: SettingsTab) => void;
}) {
  return (
    <nav className="flex items-center gap-0 border-b border-cronymax-border bg-cronymax-surface-2 px-1">
      {TAB_LABELS.map((t) => (
        <button
          key={t.id}
          type="button"
          onClick={() => onChange(t.id)}
          className={
            "border-b-2 px-3 py-1.5 text-xs transition " +
            (tab === t.id
              ? "border-cronymax-accent text-cronymax-fg"
              : "border-transparent text-cronymax-fg-muted hover:text-cronymax-fg")
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

  // Load LLM config on mount so the runner/providers have initial values.
  useEffect(() => {
    void (async () => {
      try {
        await window.llmClient.loadConfig();
        dispatch({
          type: "setLlmConfig",
          baseUrl: window.llmClient.baseUrl,
          apiKey: window.llmClient.apiKey,
          model: window.llmClient.model,
        });
      } catch {
        /* ignore */
      }
    })();
  }, [dispatch]);

  // Permission gate wired to the runner tab.
  useEffect(() => {
    window.__getPermission = (prompt: string, requestId: string) =>
      new Promise<boolean>((resolve) => {
        dispatch({
          type: "requestPermission",
          req: { prompt, requestId, resolve },
        });
      });
    return () => {
      window.__getPermission = undefined;
    };
  }, [dispatch]);

  const onResolvePermission = useCallback(
    (allow: boolean) => {
      const perm = state.permission;
      if (!perm) return;
      if (perm.requestId) {
        bridge
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
    bridge.send("shell.popover_close").catch(() => {});
  }, []);

  return (
    <main className="relative flex h-screen w-screen flex-col bg-cronymax-surface text-cronymax-fg">
      <header className="flex items-center justify-between border-b border-cronymax-border bg-cronymax-surface-2 px-4 py-2">
        <h1 className="text-sm font-semibold tracking-wide">Settings</h1>
        <button
          type="button"
          onClick={onClose}
          className="rounded px-2 py-0.5 text-xs text-cronymax-fg-muted hover:bg-cronymax-surface hover:text-cronymax-fg"
          title="Close"
        >
          ✕
        </button>
      </header>

      <TabBar tab={tab} onChange={setTab} />

      <div className="flex-1 overflow-hidden">
        {tab === "appearance" && <AppearanceTab />}
        {tab === "providers" && <ProvidersTab />}
        {tab === "agents" && <AgentsTab />}
        {tab === "workspace" && <WorkspaceTab />}
        {tab === "flows" && <Flows />}
        {tab === "runner" && <RunnerTab />}
      </div>

      {state.permission && (
        <PermissionOverlay
          perm={state.permission}
          onResolve={onResolvePermission}
        />
      )}
    </main>
  );
}
