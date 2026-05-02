/**
 * Config panel — tabbed surface for managing the workspace runtime.
 *
 *   • Flows   – embeds the agent-flow editor as a React component with
 *               its own Provider so the graph store stays isolated.
 *   • Agents  – list + create/edit/delete user-defined agents in
 *               <workspace>/.cronymax/agents/*.agent.yaml. Backed by the
 *               new `agent.registry.save` / `agent.registry.delete`
 *               channels, with `agent.registry.list` / `.load` for reads.
 *   • Runner  – legacy in-browser ReAct runner (Spaces + Ask + Run +
 *               trace), preserved so the existing Explain/Fix/Retry from
 *               the terminal still has a target.
 *
 * The LLM Settings overlay remains accessible from the header gear icon.
 */
import {
  useEffect,
  useCallback,
  useRef,
  useState,
  type KeyboardEvent,
} from "react";
import { bridge } from "@/bridge";
import { useBridgeEvent } from "@/hooks/useBridgeEvent";
import type {
  AgentGraphInstance,
  AgentTraceDetail,
  AgentRunSnapshot,
} from "@/agent_runtime";
import { useStore, type ConfigTab, type PermissionRequest } from "./store";
import { Flows } from "@/components/FlowEditor";

// ── ReAct graph builder (mirrors legacy buildReActGraph) ──────────────────
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

// ── tab strip ─────────────────────────────────────────────────────────────
function TabBar({
  tab,
  onChange,
}: {
  tab: ConfigTab;
  onChange: (t: ConfigTab) => void;
}) {
  const tabs: { id: ConfigTab; label: string }[] = [
    { id: "flows", label: "Flows" },
    { id: "agents", label: "Agents" },
    { id: "workspace", label: "Workspace" },
    { id: "providers", label: "Providers" },
    { id: "runner", label: "Runner" },
  ];
  return (
    <nav className="flex items-center gap-0 border-b border-cronymax-border bg-cronymax-surface-2 px-1">
      {tabs.map((t) => (
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

  const loadList = useCallback(async () => {
    try {
      const res = await bridge.send("agent.registry.list");
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

  const inputCls =
    "w-full rounded border border-cronymax-border bg-cronymax-surface px-2 py-1 text-xs text-cronymax-fg outline-none focus:border-cronymax-accent";
  const taCls = inputCls + " min-h-[160px] resize-y font-mono";

  return (
    <div className="flex h-full">
      {/* List */}
      <aside className="flex w-[220px] flex-col border-r border-cronymax-border bg-cronymax-surface-2">
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
                  "flex w-full flex-col items-start gap-0 px-2 py-1 text-left text-xs " +
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

      {/* Detail / editor */}
      <section className="flex-1 overflow-auto p-3">
        {!draft && (
          <p className="text-xs text-cronymax-fg-muted">
            Select an agent to view or edit its definition, or click <b>+</b> to
            create a new one. Files live under{" "}
            <code>.cronymax/agents/&lt;name&gt;.agent.yaml</code>.
          </p>
        )}
        {draft && (
          <div className="max-w-[640px]">
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
                  Rename by deleting and recreating (basename is the registry
                  key).
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
              <input
                className={inputCls}
                value={draft.llm}
                onChange={(e) => setDraft({ ...draft, llm: e.target.value })}
                placeholder="gpt-4o-mini"
              />
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

// ── Workspace tab — per-Space sandbox profile editor ────────────────────
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

  // Reload whenever the active Space changes, and on initial mount.
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
    "w-full min-h-[110px] resize-y rounded border border-cronymax-border " +
    "bg-cronymax-surface px-2 py-1 font-mono text-xs text-cronymax-fg " +
    "outline-none focus:border-cronymax-accent";

  return (
    <div className="h-full overflow-auto p-4">
      {!profile && (
        <p className="text-xs text-cronymax-fg-muted">Loading profile…</p>
      )}
      {profile && (
        <div className="max-w-[680px]">
          <h2 className="mb-1 text-sm font-semibold">{profile.space_name}</h2>
          <p className="mb-4 break-all text-[11px] text-cronymax-fg-muted">
            <code>{profile.workspace_root}</code>
          </p>

          <p className="mb-4 rounded border border-cronymax-border bg-cronymax-surface-2 p-2 text-[11px] text-cronymax-fg-muted">
            Overrides supplement the default sandbox rules. Persisted to{" "}
            <code>.cronymax/space.profile.yaml</code> in this Space's workspace.
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

// ── Providers tab — manage LLM endpoints + active selection ──────────────
type ProviderKind = "openai" | "anthropic" | "ollama" | "custom";

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
  { base_url: string; default_model: string }
> = {
  openai: {
    base_url: "https://api.openai.com/v1",
    default_model: "gpt-4o-mini",
  },
  anthropic: {
    base_url: "https://api.anthropic.com/v1",
    default_model: "claude-3-5-sonnet-latest",
  },
  ollama: { base_url: "http://localhost:11434/v1", default_model: "llama3.1" },
  custom: { base_url: "", default_model: "" },
};

function newProvider(kind: ProviderKind = "openai"): LlmProvider {
  const preset = KIND_PRESETS[kind];
  // Reasonably collision-free without crypto deps; the kv blob is rewritten
  // wholesale on each save so worst case is a duplicated id we can fix.
  const id =
    "p_" +
    Date.now().toString(36) +
    "_" +
    Math.random().toString(36).slice(2, 7);
  return {
    id,
    name: kind.charAt(0).toUpperCase() + kind.slice(1),
    kind,
    base_url: preset.base_url,
    api_key: "",
    default_model: preset.default_model,
  };
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

  const onAdd = useCallback(() => {
    const p = newProvider();
    setSelectedId(p.id);
    setDraft(p);
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

  // Activating a provider also pushes its credentials into the legacy
  // single-config slot consumed by the existing model_router pickup, so
  // anything reading window.llmClient.* keeps working unchanged.
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
        // Only fill empty fields — never clobber what the user typed.
        base_url: draft.base_url || preset.base_url,
        default_model: draft.default_model || preset.default_model,
      });
    },
    [draft],
  );

  const inputCls =
    "w-full rounded border border-cronymax-border bg-cronymax-surface px-2 py-1 text-xs text-cronymax-fg outline-none focus:border-cronymax-accent";

  return (
    <div className="flex h-full">
      <aside className="flex w-[240px] flex-col border-r border-cronymax-border bg-cronymax-surface-2">
        <div className="flex items-center justify-between border-b border-cronymax-border px-2 py-1.5">
          <span className="text-xs font-semibold">Providers</span>
          <button
            type="button"
            onClick={onAdd}
            className="rounded bg-cronymax-accent px-1.5 py-0.5 text-xs text-white hover:opacity-90"
            title="New provider"
          >
            +
          </button>
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
            Select a provider to view, edit, or activate it. Click <b>+</b> to
            add a new one. Credentials are stored in the workspace's SQLite kv
            store; the active provider's URL/key is mirrored into the legacy{" "}
            <code>llm.config</code> slot so existing consumers keep working.
          </p>
        )}
        {draft && (
          <div className="max-w-[640px]">
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
            </Field>

            <Field label="Default model">
              <input
                className={inputCls}
                value={draft.default_model}
                onChange={(e) =>
                  setDraft({ ...draft, default_model: e.target.value })
                }
                placeholder="gpt-4o-mini"
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

// ── Runner tab — preserved legacy ReAct runner ───────────────────────────
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

function PermissionOverlay({
  perm,
  onResolve,
}: {
  perm: PermissionRequest;
  onResolve: (allow: boolean) => void;
}) {
  return (
    <div className="absolute inset-0 z-50 flex items-center justify-center bg-black/40">
      <div className="w-[360px] rounded-md border border-cronymax-border bg-cronymax-surface-2 p-4 text-sm text-cronymax-fg shadow-lg">
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

function SettingsOverlay({
  baseUrl,
  apiKey,
  model,
  onChange,
  onSave,
  onCancel,
}: {
  baseUrl: string;
  apiKey: string;
  model: string;
  onChange: (field: "baseUrl" | "apiKey" | "model", value: string) => void;
  onSave: () => void;
  onCancel: () => void;
}) {
  return (
    <div className="absolute inset-0 z-40 flex items-center justify-center bg-black/40">
      <div className="w-[400px] rounded-md border border-cronymax-border bg-cronymax-surface-2 p-4 text-sm text-cronymax-fg shadow-lg">
        <h2 className="mb-3 text-base font-semibold">LLM Settings</h2>
        <label className="mb-2 block">
          <span className="mb-1 block text-xs text-cronymax-fg-muted">
            Base URL
          </span>
          <input
            type="text"
            value={baseUrl}
            onChange={(e) => onChange("baseUrl", e.target.value)}
            placeholder="http://localhost:11434"
            className="w-full rounded border border-cronymax-border bg-cronymax-surface px-2 py-1 text-sm outline-none focus:border-cronymax-accent"
          />
        </label>
        <label className="mb-2 block">
          <span className="mb-1 block text-xs text-cronymax-fg-muted">
            API Key
          </span>
          <input
            type="password"
            value={apiKey}
            onChange={(e) => onChange("apiKey", e.target.value)}
            placeholder="sk-…"
            className="w-full rounded border border-cronymax-border bg-cronymax-surface px-2 py-1 text-sm outline-none focus:border-cronymax-accent"
          />
        </label>
        <label className="mb-3 block">
          <span className="mb-1 block text-xs text-cronymax-fg-muted">
            Model
          </span>
          <input
            type="text"
            value={model}
            onChange={(e) => onChange("model", e.target.value)}
            placeholder="gpt-4o-mini"
            className="w-full rounded border border-cronymax-border bg-cronymax-surface px-2 py-1 text-sm outline-none focus:border-cronymax-accent"
          />
        </label>
        <div className="flex justify-end gap-2">
          <button
            type="button"
            onClick={onCancel}
            className="rounded border border-cronymax-border bg-cronymax-surface px-3 py-1 text-xs text-cronymax-fg hover:bg-cronymax-surface-2"
          >
            Cancel
          </button>
          <button
            type="button"
            onClick={onSave}
            className="rounded bg-cronymax-accent px-3 py-1 text-xs font-medium text-white hover:opacity-90"
          >
            Save
          </button>
        </div>
      </div>
    </div>
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
      dispatch({
        type: "appendResult",
        chunk: "\n" + (err as Error).message,
      });
      dispatch({ type: "setStatus", status: "failed" });
    }
  }, [state.task, dispatch]);

  // Inbound: terminal "Explain/Fix/Retry" → switch to runner, fill, run.
  useBridgeEvent("agent.task_from_command", (data) => {
    const action = data.action || "Explain";
    const cmd = data.command || "";
    const out = (data.output || "").slice(0, 1000);
    const ec = data.exit_code ?? -1;
    const text = `${action} the following terminal command and its output.\n\nCommand: ${cmd}\nExit code: ${ec}\nOutput:\n${out}`;
    dispatch({ type: "setTab", tab: "runner" });
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
            title="New Space"
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

// ── App ───────────────────────────────────────────────────────────────────
export function App() {
  const [state, dispatch] = useStore();

  // Load LLM config once on mount so the gear overlay opens with values.
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

  // Permission gate (used by the runner tab; keep wired regardless of tab).
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

  const saveSettings = useCallback(async () => {
    try {
      await bridge.send("llm.config.set", {
        base_url: state.llmBaseUrl,
        api_key: state.llmApiKey,
      });
      window.llmClient.baseUrl = state.llmBaseUrl;
      window.llmClient.apiKey = state.llmApiKey;
      if (state.llmModel) window.llmClient.model = state.llmModel;
      dispatch({ type: "closeSettings" });
    } catch (e) {
      console.warn("llm.config.set failed", e);
    }
  }, [state.llmBaseUrl, state.llmApiKey, state.llmModel, dispatch]);

  return (
    <main className="relative flex h-screen flex-col bg-cronymax-surface text-cronymax-fg">
      <header className="flex items-center gap-3 border-b border-cronymax-border bg-cronymax-surface-2 px-3 py-1.5">
        <h1 className="flex-1 text-sm font-semibold">Config</h1>
        {state.tab === "runner" && (
          <span
            className={
              "rounded px-2 py-0.5 text-xs " +
              (state.status === "running"
                ? "bg-yellow-500/20 text-yellow-300"
                : state.status === "done"
                  ? "bg-green-500/20 text-green-300"
                  : state.status === "failed"
                    ? "bg-red-500/20 text-red-300"
                    : "bg-cronymax-surface text-cronymax-fg-muted")
            }
          >
            {state.status}
          </span>
        )}
        <button
          type="button"
          onClick={() => dispatch({ type: "setTab", tab: "providers" })}
          className="rounded border border-cronymax-border bg-cronymax-surface px-2 py-0.5 text-xs hover:bg-cronymax-surface-2"
          title="LLM Providers"
        >
          ⚙
        </button>
      </header>

      <TabBar
        tab={state.tab}
        onChange={(tab) => dispatch({ type: "setTab", tab })}
      />

      <div className="flex-1 overflow-hidden">
        {state.tab === "flows" && <Flows />}
        {state.tab === "agents" && <AgentsTab />}
        {state.tab === "workspace" && <WorkspaceTab />}
        {state.tab === "providers" && <ProvidersTab />}
        {state.tab === "runner" && <RunnerTab />}
      </div>

      {state.settingsOpen && (
        <SettingsOverlay
          baseUrl={state.llmBaseUrl}
          apiKey={state.llmApiKey}
          model={state.llmModel}
          onChange={(field, value) =>
            dispatch({ type: "updateLlmField", field, value })
          }
          onSave={() => void saveSettings()}
          onCancel={() => dispatch({ type: "closeSettings" })}
        />
      )}
      {state.permission && (
        <PermissionOverlay
          perm={state.permission}
          onResolve={onResolvePermission}
        />
      )}
    </main>
  );
}
