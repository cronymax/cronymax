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
import { browser } from "@/shells/bridge";
import { agentRegistry, docType, agentRun } from "@/shells/runtime";
import { useBridgeEvent } from "@/hooks/useBridgeEvent";
import { useTheme } from "@/hooks/useTheme";
import type { ThemeMode } from "@/types";

import { Flows } from "@/components/FlowEditor";
import { Icon } from "@/components/Icon";
import { useStore, type PermissionRequest } from "./store";

import { Editor, rootCtx, defaultValueCtx } from "@milkdown/core";
import { listener, listenerCtx } from "@milkdown/plugin-listener";
import { commonmark } from "@milkdown/preset-commonmark";
import { Milkdown, MilkdownProvider, useEditor } from "@milkdown/react";

// ── types ─────────────────────────────────────────────────────────────────

type SettingsTab =
  | "appearance"
  | "providers"
  | "agents"
  | "doc-types"
  | "profiles"
  | "flows"
  | "runner";

// ── ReAct graph builder ───────────────────────────────────────────────────

// ── shared input styles ───────────────────────────────────────────────────

const inputCls =
  "w-full rounded border border-cronymax-border bg-cronymax-base px-2 py-1 text-xs text-cronymax-title outline-none focus:border-cronymax-primary";

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
      <div className="mb-1 text-[11px] uppercase tracking-wide text-cronymax-caption">
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
      <p className="mb-3 text-xs text-cronymax-caption">
        System follows your macOS appearance and switches automatically.
      </p>
      <div className="flex gap-2">
        {(["system", "light", "dark"] as ThemeMode[]).map((m) => (
          <label
            key={m}
            className={`flex-1 cursor-pointer rounded border px-3 py-2 text-center text-xs capitalize transition-colors ${
              mode === m
                ? "border-cronymax-primary bg-cronymax-primary/10 text-cronymax-title"
                : "border-cronymax-border bg-cronymax-base text-cronymax-caption hover:text-cronymax-title"
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
  /** Default reasoning_effort for runs against this provider. Empty = none. */
  reasoning_effort?: string;
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
          className="rounded border border-cronymax-border px-2 text-xs hover:bg-cronymax-float disabled:opacity-40"
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
                  ? "border-cronymax-primary bg-cronymax-primary/20 text-cronymax-title"
                  : "border-cronymax-border text-cronymax-caption hover:text-cronymax-title")
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
    <div className="mt-2 rounded border border-cronymax-border bg-cronymax-float p-2 text-xs">
      <div className="flex items-center justify-between gap-2">
        <span className="text-cronymax-caption">
          {hasKey
            ? "Token present. Sign in again to refresh it."
            : "Sign in with your GitHub account to fetch a Copilot token."}
        </span>
        {!isActive ? (
          <button
            type="button"
            onClick={onSignIn}
            className="rounded bg-cronymax-primary px-2 py-0.5 text-[11px] text-white hover:opacity-90"
          >
            Sign in with GitHub
          </button>
        ) : (
          <button
            type="button"
            onClick={onCancel}
            className="rounded border border-cronymax-border px-2 py-0.5 text-[11px] text-cronymax-title hover:bg-cronymax-base"
          >
            Cancel
          </button>
        )}
      </div>
      {oauth.phase === "starting" && (
        <p className="mt-1 text-cronymax-caption">Requesting device code…</p>
      )}
      {(oauth.phase === "awaiting_user" || oauth.phase === "polling") &&
        oauth.user_code && (
          <div className="mt-2 space-y-1">
            <p className="text-cronymax-caption">
              Enter this code on{" "}
              <a
                href={oauth.verification_uri}
                target="_blank"
                rel="noopener noreferrer"
                className="underline hover:text-cronymax-primary"
              >
                {oauth.verification_uri}
              </a>
              :
            </p>
            <div className="flex items-center gap-2">
              <code className="select-all rounded bg-cronymax-base px-2 py-1 font-mono text-sm tracking-widest">
                {oauth.user_code}
              </code>
              <button
                type="button"
                onClick={() =>
                  void navigator.clipboard
                    .writeText(oauth.user_code ?? "")
                    .catch(() => undefined)
                }
                className="rounded border border-cronymax-border px-2 py-0.5 text-[11px] hover:bg-cronymax-base"
              >
                Copy
              </button>
              <span className="text-[11px] text-cronymax-caption">
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
        reasoning_effort: typeof x.reasoning_effort === "string"
          ? x.reasoning_effort
          : "",
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
      const res = await browser.send("llm.providers.get");
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
      await browser.send("llm.providers.set", {
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
        await browser.send("llm.config.set", {
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
      await browser.send("shell.popover_open", { url: dc.verification_uri });
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
      <aside className="flex w-[220px] flex-col border-r border-cronymax-border bg-cronymax-float">
        <div className="flex items-center justify-between border-b border-cronymax-border px-2 py-1.5">
          <span className="text-xs font-semibold">Providers</span>
          <div className="flex items-center gap-1">
            <button
              type="button"
              onClick={() => onAdd("github_copilot")}
              className="rounded border border-cronymax-border bg-cronymax-base px-1.5 py-0.5 text-[11px] text-cronymax-title hover:bg-cronymax-float"
              title="Quick-add GitHub Copilot"
            >
              + Copilot
            </button>
            <button
              type="button"
              onClick={() => onAdd("openai")}
              className="rounded bg-cronymax-primary px-1.5 py-0.5 text-xs text-white hover:opacity-90"
              title="New provider"
            >
              +
            </button>
          </div>
        </div>
        <ul className="flex-1 overflow-auto py-1">
          {providers.length === 0 && (
            <li className="px-2 py-1 text-[11px] text-cronymax-caption">
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
                      ? "bg-cronymax-primary/15 text-cronymax-title"
                      : "text-cronymax-caption hover:bg-cronymax-base hover:text-cronymax-title")
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
          <p className="text-xs text-cronymax-caption">
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
            <Field label="Reasoning effort">
              <select
                className={inputCls}
                value={draft.reasoning_effort ?? ""}
                onChange={(e) =>
                  setDraft({ ...draft, reasoning_effort: e.target.value })
                }
                title="Default reasoning_effort for OpenAI gpt-5 / o-series. Per-message chat dropdown can override."
              >
                <option value="">none (use model default)</option>
                <option value="minimal">minimal</option>
                <option value="low">low</option>
                <option value="medium">medium</option>
                <option value="high">high</option>
                <option value="xhigh">xhigh</option>
              </select>
            </Field>

            {msg && <p className="mb-3 text-xs text-cronymax-caption">{msg}</p>}
            <div className="flex items-center gap-2">
              <button
                type="button"
                onClick={() => void onSave()}
                disabled={busy}
                className="rounded bg-cronymax-primary px-3 py-1 text-xs font-medium text-white hover:opacity-90 disabled:opacity-50"
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
                className="rounded border border-cronymax-border bg-cronymax-base px-3 py-1 text-xs hover:bg-cronymax-float"
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
  llm: string;
}

interface AgentDetail {
  name: string;
  llm: string;
  system_prompt: string;
  memory_namespace: string;
  tools: string[];
}

const EMPTY_DETAIL: AgentDetail = {
  name: "",
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
    browser.send("llm.providers.get").then((res) => {
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
      let res = await agentRegistry.list();
      const existingNames = new Set(
        (res.agents ?? []).map((a: AgentSummary) => a.name),
      );

      // Seed the built-in agents if they are not yet registered.
      // "Chat" is always seeded; the software-dev-cycle agents are seeded
      // alongside so the Flow editor can reference them by name.
      type AgentSaveReq = {
        name: string;
        llm: string;
        system_prompt: string;
        memory_namespace: string;
        tools_csv: string;
      };
      const BUILTIN_AGENTS: AgentSaveReq[] = [
        {
          name: "Chat",
          llm: "",
          system_prompt: "You are a helpful assistant.",
          memory_namespace: "",
          tools_csv: "",
        },
        {
          name: "pm",
          llm: "",
          system_prompt:
            "You are a product manager. Gather requirements and produce " +
            "clear prototypes and PRDs that the engineering team can act on.",
          memory_namespace: "",
          tools_csv: "",
        },
        {
          name: "rd",
          llm: "",
          system_prompt:
            "You are a senior software engineer. Translate PRDs into " +
            "technical specifications, implement the required changes, and " +
            "address QA feedback with focused patch notes.",
          memory_namespace: "",
          tools_csv: "",
        },
        {
          name: "qa",
          llm: "",
          system_prompt:
            "You are a QA engineer. Write test cases from the tech-spec, " +
            "run the test suite, file detailed bug reports, and produce a " +
            "final test report once all issues are resolved.",
          memory_namespace: "",
          tools_csv: "",
        },
        {
          name: "critic",
          llm: "",
          system_prompt:
            "You are a critical reviewer. Evaluate each document for " +
            "clarity, completeness, and correctness. Approve only when " +
            "the document meets the required quality bar.",
          memory_namespace: "",
          tools_csv: "",
        },
        {
          name: "qa-critic",
          llm: "",
          system_prompt:
            "You are a QA-focused reviewer. Evaluate technical " +
            "specifications and test plans for testability, coverage, and " +
            "alignment with the stated requirements.",
          memory_namespace: "",
          tools_csv: "",
        },
      ];

      const missing = BUILTIN_AGENTS.filter(
        (a) => a && !existingNames.has(a.name),
      );
      if (missing.length > 0) {
        await Promise.all(missing.map((a) => agentRegistry.save(a)));
        res = await agentRegistry.list();
      }
      setAgents(res.agents ?? []);
    } catch (err) {
      setError(`agent.registry.list: ${(err as Error).message}`);
    }
  }, []);

  const loadDetail = useCallback(async (name: string) => {
    try {
      const res = await agentRegistry.load(name);
      setDraft({
        name: res.name,
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
      await agentRegistry.save({
        name: draft.name,
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
      await agentRegistry.delete(selected);
      await loadList();
      setSelected(null);
      setDraft(null);
    } catch (err) {
      setError(`delete failed: ${(err as Error).message}`);
    } finally {
      setBusy(false);
    }
  }, [selected, loadList]);

  return (
    <div className="flex h-full">
      <aside className="flex w-[200px] flex-col border-r border-cronymax-border bg-cronymax-float">
        <div className="flex items-center justify-between border-b border-cronymax-border px-2 py-1.5">
          <span className="text-xs font-semibold">Agents</span>
          <button
            type="button"
            onClick={onNew}
            className="rounded bg-cronymax-primary px-1.5 py-0.5 text-xs text-white hover:opacity-90"
            title="New agent"
          >
            +
          </button>
        </div>
        <ul className="flex-1 overflow-auto py-1">
          {agents.length === 0 && (
            <li className="px-2 py-1 text-[11px] text-cronymax-caption">
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
                    ? "bg-cronymax-primary/15 text-cronymax-title"
                    : "text-cronymax-caption hover:bg-cronymax-base hover:text-cronymax-title")
                }
              >
                <span className="font-medium">{a.name}</span>
                <span className="text-[10px] opacity-70">{a.llm}</span>
              </button>
            </li>
          ))}
        </ul>
      </aside>

      <section className="flex-1 overflow-auto p-3">
        {!draft && (
          <p className="text-xs text-cronymax-caption">
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
                <p className="mt-1 text-[10px] text-cronymax-caption">
                  Rename by deleting and recreating.
                </p>
              )}
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
              <WysiwygMarkdownField
                value={draft.system_prompt}
                onChange={(v) => setDraft({ ...draft, system_prompt: v })}
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
                className="rounded bg-cronymax-primary px-3 py-1 text-xs font-medium text-white hover:opacity-90 disabled:opacity-50"
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
                className="rounded border border-cronymax-border bg-cronymax-base px-3 py-1 text-xs hover:bg-cronymax-float"
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

// ── Profiles tab ─────────────────────────────────────────────────────────

interface ProfileRecord {
  id: string;
  name: string;
  allow_network: boolean;
  extra_read_paths: string[];
  extra_write_paths: string[];
  extra_deny_paths: string[];
}

/** Inline edit form for a single profile. */
function ProfileForm({
  initial,
  onSave,
  onDelete,
  onCancel,
  isDefault,
}: {
  initial: ProfileRecord;
  onSave: (r: ProfileRecord) => Promise<void>;
  onDelete?: () => Promise<void>;
  onCancel: () => void;
  isDefault: boolean;
}) {
  const [name, setName] = useState(initial.name);
  const [allowNet, setAllowNet] = useState(initial.allow_network);
  const [reads, setReads] = useState(initial.extra_read_paths.join("\n"));
  const [writes, setWrites] = useState(initial.extra_write_paths.join("\n"));
  const [denies, setDenies] = useState(initial.extra_deny_paths.join("\n"));
  const [busy, setBusy] = useState(false);
  const [err, setErr] = useState<string | null>(null);

  const splitPaths = (s: string) =>
    s
      .split("\n")
      .map((l) => l.trim())
      .filter(Boolean);

  const handleSave = async () => {
    if (!name.trim()) {
      setErr("Name is required.");
      return;
    }
    setBusy(true);
    setErr(null);
    try {
      await onSave({
        ...initial,
        name: name.trim(),
        allow_network: allowNet,
        extra_read_paths: splitPaths(reads),
        extra_write_paths: splitPaths(writes),
        extra_deny_paths: splitPaths(denies),
      });
    } catch (e) {
      setErr((e as Error).message);
    } finally {
      setBusy(false);
    }
  };

  const handleDelete = async () => {
    if (!onDelete) return;
    setBusy(true);
    setErr(null);
    try {
      await onDelete();
    } catch (e) {
      setErr((e as Error).message);
    } finally {
      setBusy(false);
    }
  };

  const taCls =
    "w-full min-h-[80px] resize-y rounded border border-cronymax-border " +
    "bg-cronymax-base px-2 py-1 font-mono text-xs text-cronymax-title " +
    "outline-none focus:border-cronymax-primary";

  return (
    <div className="mt-2 rounded border border-cronymax-border bg-cronymax-float p-3 text-xs">
      <Field label="Profile name">
        <input
          className={inputCls}
          value={name}
          onChange={(e) => setName(e.target.value)}
          placeholder="e.g. Restricted"
          disabled={isDefault}
        />
        {isDefault && (
          <p className="mt-1 text-[11px] text-cronymax-caption">
            The default profile name cannot be changed.
          </p>
        )}
      </Field>
      <Field label="Network">
        <label className="flex items-center gap-2">
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
      {err && <p className="mb-2 text-xs text-red-500">{err}</p>}
      <div className="flex items-center gap-2">
        <button
          type="button"
          onClick={() => void handleSave()}
          disabled={busy}
          className="rounded bg-cronymax-primary px-3 py-1 text-xs font-medium text-white hover:opacity-90 disabled:opacity-50"
        >
          Save
        </button>
        <button
          type="button"
          onClick={onCancel}
          disabled={busy}
          className="rounded border border-cronymax-border bg-cronymax-base px-3 py-1 text-xs text-cronymax-title hover:bg-cronymax-float"
        >
          Cancel
        </button>
        {onDelete && !isDefault && (
          <button
            type="button"
            onClick={() => void handleDelete()}
            disabled={busy}
            className="ml-auto rounded border border-red-400 px-3 py-1 text-xs text-red-500 hover:bg-red-50"
          >
            Delete
          </button>
        )}
        {isDefault && (
          <span
            className="ml-auto text-[11px] text-cronymax-caption"
            title="The default profile cannot be deleted"
          >
            🔒 Cannot delete default
          </span>
        )}
      </div>
    </div>
  );
}

/** New-profile creation form. */
function NewProfileForm({ onCreated }: { onCreated: () => void }) {
  const [open, setOpen] = useState(false);

  if (!open) {
    return (
      <button
        type="button"
        onClick={() => setOpen(true)}
        className="mt-3 rounded border border-dashed border-cronymax-border px-3 py-1.5 text-xs text-cronymax-caption hover:text-cronymax-title"
      >
        + New profile
      </button>
    );
  }

  const blank: ProfileRecord = {
    id: "",
    name: "",
    allow_network: true,
    extra_read_paths: [],
    extra_write_paths: [],
    extra_deny_paths: [],
  };

  return (
    <ProfileForm
      initial={blank}
      isDefault={false}
      onSave={async (r) => {
        await browser.send("profiles.create", {
          name: r.name,
          allow_network: r.allow_network,
          extra_read_paths: r.extra_read_paths,
          extra_write_paths: r.extra_write_paths,
          extra_deny_paths: r.extra_deny_paths,
        });
        setOpen(false);
        onCreated();
      }}
      onCancel={() => setOpen(false)}
    />
  );
}

function ProfilesTab() {
  const [profiles, setProfiles] = useState<ProfileRecord[]>([]);
  const [expandedId, setExpandedId] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [msg, setMsg] = useState<string | null>(null);

  const reload = useCallback(async () => {
    setMsg(null);
    try {
      const res = await browser.send("profiles.list");
      setProfiles(res);
    } catch (e) {
      setMsg(`load failed: ${(e as Error).message}`);
    }
  }, []);

  useEffect(() => {
    void reload();
  }, [reload]);

  const handleSave = async (r: ProfileRecord) => {
    setBusy(true);
    try {
      await browser.send("profiles.update", {
        id: r.id,
        name: r.name,
        allow_network: r.allow_network,
        extra_read_paths: r.extra_read_paths,
        extra_write_paths: r.extra_write_paths,
        extra_deny_paths: r.extra_deny_paths,
      });
      setExpandedId(null);
      await reload();
    } finally {
      setBusy(false);
    }
  };

  const handleDelete = async (id: string) => {
    setBusy(true);
    try {
      await browser.send("profiles.delete", { id });
      setExpandedId(null);
      await reload();
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="h-full overflow-auto p-4">
      <p className="mb-4 text-[11px] text-cronymax-caption">
        Named sandbox profiles are stored in <code>~/.cronymax/profiles/</code>.
        Assign a profile to a workspace when opening a folder.
      </p>
      {msg && <p className="mb-3 text-xs text-red-500">{msg}</p>}
      <div className="max-w-[600px] space-y-2">
        {profiles.map((p) => (
          <div
            key={p.id}
            className="rounded border border-cronymax-border bg-cronymax-base"
          >
            <button
              type="button"
              onClick={() =>
                setExpandedId((prev) => (prev === p.id ? null : p.id))
              }
              className="flex w-full items-center justify-between px-3 py-2 text-left text-xs"
            >
              <span className="font-medium text-cronymax-title">{p.name}</span>
              <span className="text-cronymax-caption">
                {p.allow_network ? "network ✓" : "network ✗"} ·{" "}
                {p.id === "default" ? "🔒 default" : p.id}
              </span>
            </button>
            {expandedId === p.id && (
              <ProfileForm
                initial={p}
                isDefault={p.id === "default"}
                onSave={handleSave}
                onDelete={() => handleDelete(p.id)}
                onCancel={() => setExpandedId(null)}
              />
            )}
          </div>
        ))}
      </div>
      <NewProfileForm onCreated={() => void reload()} />
      {busy && (
        <p className="mt-3 text-[11px] text-cronymax-caption">Saving…</p>
      )}
    </div>
  );
}

// ── Runner tab ────────────────────────────────────────────────────────────

// ── Doc Types tab ─────────────────────────────────────────────────────────

/**
 * Inner Milkdown editor — must be rendered inside a MilkdownProvider.
 * Initialised once with `initialValue`; calls `onEmit` on every change.
 */
function WysiwygInner({
  initialValue,
  onEmit,
}: {
  initialValue: string;
  onEmit: (v: string) => void;
}) {
  const onEmitRef = useRef(onEmit);
  useEffect(() => {
    onEmitRef.current = onEmit;
  });

  useEditor((root) =>
    Editor.make()
      .config((ctx) => {
        ctx.set(rootCtx, root);
        ctx.set(defaultValueCtx, initialValue);
        ctx.get(listenerCtx).markdownUpdated((_, markdown) => {
          onEmitRef.current(markdown);
        });
      })
      .use(commonmark)
      .use(listener),
  );

  return <Milkdown />;
}

/**
 * WYSIWYG Markdown editor backed by Milkdown/ProseMirror.
 * Renders Markdown as rich text that the user can edit directly.
 *
 * When `value` changes from the outside (e.g. a different doc type is
 * selected), the editor is remounted to pick up the new initial content.
 */
function WysiwygMarkdownField({
  value,
  onChange,
  disabled = false,
}: {
  value: string;
  onChange?: (v: string) => void;
  disabled?: boolean;
  placeholder?: string;
}) {
  // Track the last markdown emitted by the editor so we can distinguish
  // an external value change (user selected a different doc type) from
  // an internal one (user typed in the editor). Only external changes
  // trigger a remount.
  const lastEmitted = useRef<string>(value);
  const [editorKey, setEditorKey] = useState(0);

  useEffect(() => {
    if (value !== lastEmitted.current) {
      lastEmitted.current = value;
      setEditorKey((k) => k + 1);
    }
  }, [value]);

  const handleEmit = useCallback(
    (md: string) => {
      lastEmitted.current = md;
      onChange?.(md);
    },
    [onChange],
  );

  return (
    <div
      className={
        "cronymax-wysiwyg rounded border border-cronymax-border " +
        "bg-cronymax-base overflow-auto " +
        (disabled ? "pointer-events-none opacity-60" : "")
      }
    >
      <MilkdownProvider key={editorKey}>
        <WysiwygInner initialValue={value} onEmit={handleEmit} />
      </MilkdownProvider>
    </div>
  );
}

interface DocTypeSummary {
  name: string;
  display_name: string;
  user_defined: boolean;
}

interface DocTypeDraft {
  name: string;
  display_name: string;
  description: string;
}

const EMPTY_DOC_TYPE: DocTypeDraft = {
  name: "",
  display_name: "",
  description: "",
};

function DocTypesTab() {
  const [docTypes, setDocTypes] = useState<DocTypeSummary[]>([]);
  const [selected, setSelected] = useState<string | null>(null);
  const [draft, setDraft] = useState<DocTypeDraft | null>(null);
  const [creating, setCreating] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const loadList = useCallback(async () => {
    try {
      const res = await docType.list();
      setDocTypes((res.doc_types ?? []) as DocTypeSummary[]);
    } catch (err) {
      setError(`doc_type.list: ${(err as Error).message}`);
    }
  }, []);

  useEffect(() => {
    void loadList();
  }, [loadList]);

  const onSelect = useCallback(async (dt: DocTypeSummary) => {
    setSelected(dt.name);
    setCreating(false);
    setError(null);
    // Show name/display immediately while description loads
    setDraft({ name: dt.name, display_name: dt.display_name, description: "" });
    try {
      const res = (await docType.load(dt.name)) as {
        name: string;
        display_name: string;
        description: string;
      };
      setDraft({
        name: res.name,
        display_name: res.display_name,
        description: res.description,
      });
    } catch (err) {
      setError(`doc_type.load: ${(err as Error).message}`);
    }
  }, []);

  const onNew = useCallback(() => {
    setSelected(null);
    setCreating(true);
    setDraft({ ...EMPTY_DOC_TYPE });
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
      await docType.save(
        draft.name,
        draft.display_name || draft.name,
        draft.description,
      );
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
    if (!confirm(`Delete doc-type "${selected}" YAML file?`)) return;
    setBusy(true);
    setError(null);
    try {
      await docType.delete(selected);
      await loadList();
      setSelected(null);
      setDraft(null);
    } catch (err) {
      setError(`delete failed: ${(err as Error).message}`);
    } finally {
      setBusy(false);
    }
  }, [selected, loadList]);

  const selectedIsUserDefined =
    selected != null &&
    (docTypes.find((d) => d.name === selected)?.user_defined ?? false);

  const editable = creating || selectedIsUserDefined;

  return (
    <div className="flex h-full">
      <aside className="flex w-[200px] flex-col border-r border-cronymax-border bg-cronymax-float">
        <div className="flex items-center justify-between border-b border-cronymax-border px-2 py-1.5">
          <span className="text-xs font-semibold">Doc Types</span>
          <button
            type="button"
            onClick={onNew}
            className="rounded bg-cronymax-primary px-1.5 py-0.5 text-xs text-white hover:opacity-90"
            title="New doc type"
          >
            +
          </button>
        </div>
        <ul className="flex-1 overflow-auto py-1">
          {docTypes.length === 0 && (
            <li className="px-2 py-1 text-[11px] text-cronymax-caption">
              No doc types found.
            </li>
          )}
          {docTypes.map((dt) => (
            <li key={dt.name}>
              <button
                type="button"
                onClick={() => void onSelect(dt)}
                className={
                  "flex w-full flex-col items-start px-2 py-1 text-left text-xs " +
                  (selected === dt.name && !creating
                    ? "bg-cronymax-primary/15 text-cronymax-title"
                    : "text-cronymax-caption hover:bg-cronymax-base hover:text-cronymax-title")
                }
              >
                <span className="font-medium">{dt.name}</span>
                <span className="text-[10px] opacity-70">
                  {dt.user_defined ? "user" : "built-in"}
                </span>
              </button>
            </li>
          ))}
        </ul>
      </aside>

      <section className="flex-1 overflow-auto p-3">
        {!draft && (
          <p className="text-xs text-cronymax-caption">
            Select a doc type to view its Markdown description, or click{" "}
            <b>+</b> to create a new one. User-defined doc types are stored in{" "}
            <code>.cronymax/doc-types/&lt;name&gt;.yaml</code> and appear
            alongside built-ins in the Flow PRODUCES picker.
          </p>
        )}
        {draft && (
          <div className="max-w-[640px]">
            <h2 className="mb-3 text-sm font-semibold">
              {creating
                ? "New doc type"
                : `${editable ? "Edit" : "View"}: ${selected}`}
            </h2>
            {!creating && !selectedIsUserDefined && (
              <p className="mb-3 rounded border border-cronymax-border bg-cronymax-float p-2 text-[11px] text-cronymax-caption">
                Built-in doc types are read-only. Create a user doc type to
                define your own document structure.
              </p>
            )}
            <Field label="Name (file basename)">
              <input
                className={inputCls}
                value={draft.name}
                disabled={!creating}
                onChange={(e) => setDraft({ ...draft, name: e.target.value })}
                placeholder="my-doc-type"
              />
              {!creating && (
                <p className="mt-1 text-[10px] text-cronymax-caption">
                  Rename by deleting and recreating.
                </p>
              )}
            </Field>
            <Field label="Display name">
              <input
                className={inputCls}
                value={draft.display_name}
                disabled={!editable}
                onChange={(e) =>
                  setDraft({ ...draft, display_name: e.target.value })
                }
                placeholder="My Doc Type"
              />
            </Field>
            <Field label="Description">
              <WysiwygMarkdownField
                value={draft.description}
                onChange={
                  editable
                    ? (v) => setDraft({ ...draft, description: v })
                    : undefined
                }
                disabled={!editable}
                placeholder="Describe what this document type represents (Markdown supported)"
              />
            </Field>
            {error && <p className="mb-3 text-xs text-red-300">{error}</p>}
            {editable && (
              <div className="mt-3 flex items-center gap-2">
                <button
                  type="button"
                  onClick={() => void onSave()}
                  disabled={busy}
                  className="rounded bg-cronymax-primary px-3 py-1 text-xs font-medium text-white hover:opacity-90 disabled:opacity-50"
                >
                  {creating ? "Create" : "Save"}
                </button>
                {!creating && selectedIsUserDefined && (
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
                  className="rounded border border-cronymax-border bg-cronymax-base px-3 py-1 text-xs hover:bg-cronymax-float"
                >
                  Cancel
                </button>
              </div>
            )}
          </div>
        )}
      </section>
    </div>
  );
}

// ── Runner tab (was here) ──────────────────────────────────────────────────

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
          ? "bg-cronymax-float text-cronymax-title"
          : "text-cronymax-caption hover:bg-cronymax-float hover:text-cronymax-title")
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
        aria-label="Delete space"
      >
        <Icon name="close" size={12} aria-hidden="true" />
      </button>
    </li>
  );
}

function RunnerTab() {
  const [state, dispatch] = useStore();
  const taskRef = useRef<HTMLTextAreaElement>(null);

  const loadSpaces = useCallback(async () => {
    try {
      const spaces = await browser.send("space.list");
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
        await browser.send("space.switch", { space_id: id });
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
        await browser.send("space.delete", { space_id: id });
        await loadSpaces();
      } catch (e) {
        console.warn("space.delete failed", e);
      }
    },
    [loadSpaces],
  );

  const newSpace = useCallback(async () => {
    // eslint-disable-next-line no-alert
    const root = prompt("Root path:", "/");
    if (!root) return;
    try {
      await browser.send("space.create", {
        root_path: root,
        profile_id: "default",
      });
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

    let runId = "";
    try {
      runId = await agentRun(text);
      if (!runId) throw new Error("runtime did not return run_id");
      await browser.send("events.subscribe", { run_id: runId }).catch(() => {});
    } catch (err) {
      dispatch({ type: "appendResult", chunk: "\n" + (err as Error).message });
      dispatch({ type: "setStatus", status: "failed" });
      return;
    }

    const off = browser.on("event", (raw: unknown) => {
      const ev = raw as Record<string, unknown> | null;
      if (!ev) return;
      if (ev.tag === "event") {
        const inner = (ev.event as Record<string, unknown> | undefined) ?? {};
        const pl = (inner.payload as Record<string, unknown> | undefined) ?? {};
        const pRunId = (inner as Record<string, unknown>).run_id as
          | string
          | undefined;
        if (pRunId && pRunId !== runId) return;
        const kind = pl.kind as string | undefined;
        if (kind === "token" && pl.content) {
          dispatch({ type: "appendResult", chunk: pl.content as string });
        } else if (kind === "run_status") {
          const status = pl.status as string | undefined;
          if (status === "succeeded") {
            dispatch({ type: "setStatus", status: "done" });
            off();
          } else if (status === "failed" || status === "cancelled") {
            dispatch({ type: "appendResult", chunk: `\n[${status}]` });
            dispatch({ type: "setStatus", status: "failed" });
            off();
          }
        } else if (kind === "log") {
          dispatch({
            type: "appendResult",
            chunk: `\n[log] ${pl.message ?? ""}`,
          });
        }
      }
    });
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
        <div className="mb-1 flex items-center justify-between text-xs text-cronymax-caption">
          <span>Spaces</span>
          <button
            type="button"
            onClick={() => void newSpace()}
            className="rounded bg-cronymax-base px-1.5 text-cronymax-title hover:bg-cronymax-float"
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
        className="m-3 min-h-[80px] resize-y rounded border border-cronymax-border bg-cronymax-float p-2 text-sm text-cronymax-title outline-none focus:border-cronymax-primary"
      />
      <div className="flex justify-end gap-2 px-3">
        <button
          type="button"
          onClick={() => void runTask()}
          disabled={state.status === "running"}
          className="rounded bg-cronymax-primary px-3 py-1 text-sm font-medium text-white transition hover:opacity-90 disabled:cursor-not-allowed disabled:opacity-50"
        >
          Run
        </button>
      </div>
      <pre className="m-3 flex-1 overflow-auto whitespace-pre-wrap break-words rounded border border-cronymax-border bg-cronymax-float p-2 text-xs text-cronymax-title">
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
  { id: "agents", label: "Agents" },
  { id: "doc-types", label: "Doc Types" },
  { id: "profiles", label: "Profiles" },
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
        const active =
          providers.find((p) => p.id === provRes.active_id) || providers[0];
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
    browser.send("shell.popover_close").catch(() => {});
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
        {tab === "agents" && <AgentsTab />}
        {tab === "doc-types" && <DocTypesTab />}
        {tab === "profiles" && <ProfilesTab />}
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
