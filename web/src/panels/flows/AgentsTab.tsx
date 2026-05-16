import { useCallback, useEffect, useState } from "react";
import { type LlmProvider, ModelSelect } from "../../components/ModelSelect";
import { WysiwygMarkdown } from "../../components/WysiwygMarkdown";
import { browser } from "../../shells/bridge";
import { agentRegistry } from "../../shells/runtime";
import { Field, inputCls } from "./App";

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
  builtin?: boolean;
  prompt_sealed?: boolean;
}
const EMPTY_DETAIL: AgentDetail = {
  name: "",
  llm: "gpt-4o-mini",
  system_prompt: "You are a helpful agent.",
  memory_namespace: "",
  tools: [],
};
/** Canonical tool groups for the agent tools checkbox UI. */
const TOOL_GROUPS: { label: string; tools: string[] }[] = [
  { label: "Shell", tools: ["run_shell", "run_terminal"] },
  {
    label: "Filesystem",
    tools: ["read_file", "write_file", "str_replace", "list_dir"],
  },
  {
    label: "Search",
    tools: ["search_workspace", "grep_workspace", "glob_files"],
  },
  {
    label: "Git",
    tools: ["git_status", "git_diff", "git_log", "git_add", "git_reset", "git_commit", "git_push"],
  },
  {
    label: "Workflow",
    tools: ["submit_document", "notify", "request_approval", "mention"],
  },
  {
    label: "Testing",
    tools: ["discover_tests", "run_suite", "get_last_report"],
  },
];
/** Flat set of all known tools (for "Other" bucket detection). */
const ALL_KNOWN_TOOLS = new Set(TOOL_GROUPS.flatMap((g) => g.tools));
/**
 * Grouped checkbox list for selecting which tools an agent may use.
 *
 * `value=[]` means "all tools" (Space defaults). When ALL known groups are
 * fully checked, the value is saved as `[]`; otherwise the explicit list is
 * saved.
 */
function ToolCheckboxes({ value, onChange }: { value: string[]; onChange: (v: string[]) => void }) {
  // Derive "unknown" tools from the current value that aren't in any group.
  const unknownTools = value.filter((t) => !ALL_KNOWN_TOOLS.has(t));

  // When value === [], treat all known tools as checked.
  const effectiveSet = new Set(value.length === 0 ? TOOL_GROUPS.flatMap((g) => g.tools) : value);

  function toggle(tool: string) {
    const next = new Set(effectiveSet);
    if (next.has(tool)) {
      next.delete(tool);
    } else {
      next.add(tool);
    }
    // If all known tools are checked, save as []
    const allKnown = TOOL_GROUPS.flatMap((g) => g.tools);
    const allChecked = allKnown.every((t) => next.has(t));
    onChange(allChecked ? [] : [...next]);
  }

  function toggleGroup(group: { label: string; tools: string[] }) {
    const allChecked = group.tools.every((t) => effectiveSet.has(t));
    const next = new Set(effectiveSet);
    if (allChecked) {
      for (const t of group.tools) next.delete(t);
    } else {
      for (const t of group.tools) next.add(t);
    }
    const allKnown = TOOL_GROUPS.flatMap((g) => g.tools);
    const allChecked2 = allKnown.every((t) => next.has(t));
    onChange(allChecked2 ? [] : [...next]);
  }

  return (
    <div className="space-y-2">
      {TOOL_GROUPS.map((group) => {
        const groupChecked = group.tools.every((t) => effectiveSet.has(t));
        const groupPartial = !groupChecked && group.tools.some((t) => effectiveSet.has(t));
        return (
          <div key={group.label}>
            <label className="flex cursor-pointer items-center gap-1.5 text-[11px] font-semibold text-cronymax-title">
              <input
                type="checkbox"
                checked={groupChecked}
                ref={(el) => {
                  if (el) el.indeterminate = groupPartial;
                }}
                onChange={() => toggleGroup(group)}
                className="accent-cronymax-primary"
              />
              {group.label}
            </label>
            <div className="ml-4 mt-0.5 flex flex-wrap gap-x-3 gap-y-0.5">
              {group.tools.map((tool) => (
                <label key={tool} className="flex cursor-pointer items-center gap-1 text-[11px] text-cronymax-caption">
                  <input
                    type="checkbox"
                    checked={effectiveSet.has(tool)}
                    onChange={() => toggle(tool)}
                    className="accent-cronymax-primary"
                  />
                  {tool}
                </label>
              ))}
            </div>
          </div>
        );
      })}
      {unknownTools.length > 0 && (
        <div>
          <span className="text-[11px] font-semibold text-cronymax-title">Other</span>
          <div className="ml-4 mt-0.5 flex flex-wrap gap-x-3 gap-y-0.5">
            {unknownTools.map((tool) => (
              <label key={tool} className="flex cursor-pointer items-center gap-1 text-[11px] text-cronymax-caption">
                <input
                  type="checkbox"
                  checked={effectiveSet.has(tool)}
                  onChange={() => toggle(tool)}
                  className="accent-cronymax-primary"
                />
                {tool}
              </label>
            ))}
          </div>
        </div>
      )}
    </div>
  );
}
export function AgentsTab() {
  const [agents, setAgents] = useState<AgentSummary[]>([]);
  const [selected, setSelected] = useState<string | null>(null);
  const [draft, setDraft] = useState<AgentDetail | null>(null);
  const [creating, setCreating] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [activeProvider, setActiveProvider] = useState<LlmProvider | null>(null);

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
      const existingNames = new Set((res.agents ?? []).map((a: AgentSummary) => a.name));

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

      const missing = BUILTIN_AGENTS.filter((a) => a && !existingNames.has(a.name));
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
      setError("Name must be 1-64 chars of letters, digits, _, -, or . (no slashes).");
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
            <li className="px-2 py-1 text-[11px] text-cronymax-caption">No agents registered.</li>
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
            Select an agent to view or edit, or click <b>+</b> to create one. Files live under{" "}
            <code>.cronymax/agents/&lt;name&gt;.agent.yaml</code>.
          </p>
        )}
        {draft && (
          <div className="max-w-[560px]">
            <h2 className="mb-3 text-sm font-semibold">{creating ? "New agent" : `Edit: ${selected}`}</h2>
            {draft.prompt_sealed ? (
              <>
                <p className="mb-4 rounded border border-cronymax-border bg-cronymax-float px-3 py-2 text-xs text-cronymax-caption">
                  Built-in agent — configuration is sealed and read-only.
                </p>
                <Field label="System prompt">
                  <pre className="whitespace-pre-wrap rounded border border-cronymax-border bg-cronymax-base px-3 py-2 text-xs text-cronymax-body">
                    {draft.system_prompt}
                  </pre>
                </Field>
                <div className="flex items-center gap-2">
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
                    Close
                  </button>
                </div>
              </>
            ) : (
              <>
                <Field label="Name (file basename)">
                  <input
                    className={inputCls}
                    value={draft.name}
                    disabled={!creating}
                    onChange={(e) => setDraft({ ...draft, name: e.target.value })}
                    placeholder="my_worker"
                  />
                  {!creating && (
                    <p className="mt-1 text-[10px] text-cronymax-caption">Rename by deleting and recreating.</p>
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
                    onChange={(e) => setDraft({ ...draft, memory_namespace: e.target.value })}
                    placeholder="(defaults to agent name)"
                  />
                </Field>
                <Field label="System prompt">
                  <WysiwygMarkdown
                    value={draft.system_prompt}
                    onChange={(v) => setDraft({ ...draft, system_prompt: v })}
                    readOnly={false}
                  />
                </Field>
                <Field label="Tools (empty = Space defaults, all checked)">
                  <ToolCheckboxes value={draft.tools} onChange={(v) => setDraft({ ...draft, tools: v })} />
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
              </>
            )}
          </div>
        )}
      </section>
    </div>
  );
}
