import { useCallback, useEffect, useState } from "react";
import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
import { Input } from "@/components/ui/input";
import { Separator } from "@/components/ui/separator";
import { Skeleton } from "@/components/ui/skeleton";
import { ModelGroupCombobox } from "../../components/ModelGroupCombobox";
import { fetchModelGroups, type ModelGroup } from "../../components/modelGroups";
import { WysiwygMarkdown } from "../../components/WysiwygMarkdown";
import { browser } from "../../shells/bridge";
import { ContributionKind, type ContributionKindId, contributionRegistry } from "../../shells/runtime";
import { Field } from "./App";

// ── Agents tab ────────────────────────────────────────────────────────────
interface AgentSummary {
  name: string;
  llm: string;
  /** Which Contribution kind backs this agent — picks the load/save target. */
  contribution_kind: ContributionKindId;
}
interface AgentDetail {
  name: string;
  llm: string;
  /** When set, the agent is backed by an extension AgentProvider (P8) instead
   *  of an LLM — `llm` is then ignored. `model: ""` means the provider default. */
  agent_provider?: { id: string; model: string } | null;
  system_prompt: string;
  memory_namespace: string;
  tools: string[];
  builtin?: boolean;
  prompt_sealed?: boolean;
  contribution_kind: ContributionKindId;
}
const EMPTY_DETAIL: AgentDetail = {
  name: "",
  llm: "gpt-4o-mini",
  system_prompt: "You are a helpful agent.",
  memory_namespace: "",
  tools: [],
  contribution_kind: ContributionKind.AgentsWorkspace,
};

/** Map a ContributionDescriptor into the AgentsTab summary shape. */
function descriptorToSummary(d: { kind: string; id: string; metadata?: unknown }): AgentSummary {
  const meta = (d.metadata && typeof d.metadata === "object" ? d.metadata : {}) as Record<string, unknown>;
  return {
    name: d.id,
    llm: typeof meta.llm === "string" ? meta.llm : "",
    contribution_kind: d.kind as ContributionKindId,
  };
}
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
            <div className="flex cursor-pointer items-center gap-1.5 text-xs font-semibold text-foreground">
              <Checkbox
                checked={groupPartial ? "indeterminate" : groupChecked}
                onCheckedChange={() => toggleGroup(group)}
              />
              {group.label}
            </div>
            <div className="ml-5 mt-0.5 flex flex-wrap gap-x-3 gap-y-0.5">
              {group.tools.map((tool) => (
                <div key={tool} className="flex cursor-pointer items-center gap-1 text-xs text-muted-foreground">
                  <Checkbox
                    checked={effectiveSet.has(tool)}
                    onCheckedChange={(checked) => {
                      if (checked !== "indeterminate") toggle(tool);
                    }}
                  />
                  {tool}
                </div>
              ))}
            </div>
          </div>
        );
      })}
      {unknownTools.length > 0 && (
        <div>
          <span className="text-xs font-semibold text-foreground">Other</span>
          <div className="ml-5 mt-0.5 flex flex-wrap gap-x-3 gap-y-0.5">
            {unknownTools.map((tool) => (
              <div key={tool} className="flex cursor-pointer items-center gap-1 text-xs text-muted-foreground">
                <Checkbox
                  checked={effectiveSet.has(tool)}
                  onCheckedChange={(checked) => {
                    if (checked !== "indeterminate") toggle(tool);
                  }}
                />
                {tool}
              </div>
            ))}
          </div>
        </div>
      )}
    </div>
  );
}

export function AgentsTab() {
  const [agents, setAgents] = useState<AgentSummary[]>([]);
  const [loaded, setLoaded] = useState(false);
  const [selected, setSelected] = useState<string | null>(null);
  const [draft, setDraft] = useState<AgentDetail | null>(null);
  const [creating, setCreating] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  // The same grouped LLM + extension-provider catalog the chat panel uses, so
  // an agent can be bound to any provider's model from one picker.
  const [modelGroups, setModelGroups] = useState<ModelGroup[]>([]);

  useEffect(() => {
    let cancelled = false;
    const load = () => {
      void fetchModelGroups().then((groups) => {
        if (!cancelled) setModelGroups(groups);
      });
    };
    load();
    const off = browser.on("runtime.reconnected", load);
    return () => {
      cancelled = true;
      off();
    };
  }, []);

  const loadList = useCallback(async () => {
    try {
      let res = await contributionRegistry.list();
      const filtered = (res.contributions ?? []).filter(
        (d) => d.kind === ContributionKind.AgentsBuiltin || d.kind === ContributionKind.AgentsWorkspace,
      );
      const existingNames = new Set(filtered.map((d) => d.id));

      // Seed the built-in workspace agents if they are not yet registered.
      // "Chat" is always seeded; the software-dev-cycle agents are seeded
      // alongside so the Flow editor can reference them by name.
      type AgentSeed = {
        name: string;
        llm: string;
        system_prompt: string;
        memory_namespace: string;
        tools: string[];
      };
      const BUILTIN_AGENTS: AgentSeed[] = [
        {
          name: "Chat",
          llm: "",
          system_prompt: "You are a helpful assistant.",
          memory_namespace: "",
          tools: [],
        },
        {
          name: "pm",
          llm: "",
          system_prompt:
            "You are a product manager. Gather requirements and produce " +
            "clear prototypes and PRDs that the engineering team can act on.",
          memory_namespace: "",
          tools: [],
        },
        {
          name: "rd",
          llm: "",
          system_prompt:
            "You are a senior software engineer. Translate PRDs into " +
            "technical specifications, implement the required changes, and " +
            "address QA feedback with focused patch notes.",
          memory_namespace: "",
          tools: [],
        },
        {
          name: "qa",
          llm: "",
          system_prompt:
            "You are a QA engineer. Write test cases from the tech-spec, " +
            "run the test suite, file detailed bug reports, and produce a " +
            "final test report once all issues are resolved.",
          memory_namespace: "",
          tools: [],
        },
        {
          name: "critic",
          llm: "",
          system_prompt:
            "You are a critical reviewer. Evaluate each document for " +
            "clarity, completeness, and correctness. Approve only when " +
            "the document meets the required quality bar.",
          memory_namespace: "",
          tools: [],
        },
        {
          name: "qa-critic",
          llm: "",
          system_prompt:
            "You are a QA-focused reviewer. Evaluate technical " +
            "specifications and test plans for testability, coverage, and " +
            "alignment with the stated requirements.",
          memory_namespace: "",
          tools: [],
        },
      ];

      const missing = BUILTIN_AGENTS.filter((a) => a && !existingNames.has(a.name));
      if (missing.length > 0) {
        await Promise.all(
          missing.map((a) =>
            contributionRegistry.save(ContributionKind.AgentsWorkspace, a.name, {
              kind: "worker",
              llm: a.llm,
              system_prompt: a.system_prompt,
              memory_namespace: a.memory_namespace,
              tools: a.tools,
            }),
          ),
        );
        res = await contributionRegistry.list();
      }
      const finalList = (res.contributions ?? []).filter(
        (d) => d.kind === ContributionKind.AgentsBuiltin || d.kind === ContributionKind.AgentsWorkspace,
      );
      setAgents(finalList.map(descriptorToSummary));
      setLoaded(true);
    } catch (err) {
      setError(`contribution.list: ${(err as Error).message}`);
      setLoaded(true);
    }
  }, []);

  const loadDetail = useCallback(
    async (name: string) => {
      const target = agents.find((a) => a.name === name);
      const kind = target?.contribution_kind ?? ContributionKind.AgentsWorkspace;
      try {
        const res = await contributionRegistry.load(kind, name);
        const src = (res.source && typeof res.source === "object" ? res.source : {}) as Record<string, unknown>;
        const ap = (src.agent_provider && typeof src.agent_provider === "object" ? src.agent_provider : null) as {
          id?: unknown;
          model?: unknown;
        } | null;
        setDraft({
          name: typeof src.name === "string" ? src.name : name,
          llm: typeof src.llm === "string" ? src.llm : "",
          agent_provider:
            ap && typeof ap.id === "string" && ap.id
              ? { id: ap.id, model: typeof ap.model === "string" ? ap.model : "" }
              : null,
          system_prompt: typeof src.system_prompt === "string" ? src.system_prompt : "",
          memory_namespace: typeof src.memory_namespace === "string" ? src.memory_namespace : "",
          tools: Array.isArray(src.tools) ? (src.tools.filter((t) => typeof t === "string") as string[]) : [],
          prompt_sealed: typeof src.prompt_sealed === "boolean" ? src.prompt_sealed : undefined,
          contribution_kind: kind,
        });
        setCreating(false);
        setError(null);
      } catch (err) {
        setError(`contribution.load: ${(err as Error).message}`);
      }
    },
    [agents],
  );

  useEffect(() => {
    void loadList();
    return browser.on("runtime.reconnected", () => {
      void loadList();
    });
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
      const boundToExt = Boolean(draft.agent_provider?.id);
      await contributionRegistry.save(ContributionKind.AgentsWorkspace, draft.name, {
        kind: "worker",
        // Extension-backed agents write `agent_provider:` (mutually exclusive
        // with `llm:`); LLM-backed agents write `llm:`.
        ...(boundToExt
          ? { agent_provider: { id: draft.agent_provider?.id ?? "", model: draft.agent_provider?.model ?? "" } }
          : { llm: draft.llm }),
        system_prompt: draft.system_prompt,
        memory_namespace: draft.memory_namespace,
        tools: draft.tools,
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
      await contributionRegistry.delete(ContributionKind.AgentsWorkspace, selected);
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
      <aside className="flex w-[200px] flex-col border-r border-border bg-card">
        <div className="flex items-center justify-between px-2 py-1.5">
          <span className="text-xs font-semibold">Agents</span>
          <Button type="button" size="icon" className="h-5 w-5 text-xs" onClick={onNew} title="New agent">
            +
          </Button>
        </div>
        <Separator />
        <ul className="flex-1 overflow-auto py-1">
          {!loaded && (
            <>
              <Skeleton className="mx-2 my-1 h-8" />
              <Skeleton className="mx-2 my-1 h-8" />
              <Skeleton className="mx-2 my-1 h-8" />
            </>
          )}
          {loaded && agents.length === 0 && (
            <li className="px-2 py-1 text-xs text-muted-foreground">No agents registered.</li>
          )}
          {agents.map((a) => (
            <li key={a.name}>
              <Button
                type="button"
                variant="ghost"
                onClick={() => onSelect(a.name)}
                className={
                  "flex h-auto w-full flex-col items-start px-2 py-1 text-left text-xs font-normal " +
                  (selected === a.name && !creating
                    ? "bg-primary/15 text-foreground"
                    : "text-muted-foreground hover:bg-accent hover:text-foreground")
                }
              >
                <span className="font-medium">{a.name}</span>
                <span className="text-xs opacity-70">{a.llm}</span>
              </Button>
            </li>
          ))}
        </ul>
      </aside>

      <section className="flex-1 overflow-auto p-3">
        {!draft && (
          <p className="text-xs text-muted-foreground">
            Select an agent to view or edit, or click <b>+</b> to create one. Files live under{" "}
            <code>.cronymax/agents/&lt;name&gt;.agent.yaml</code>.
          </p>
        )}
        {draft && (
          <div className="max-w-[560px]">
            <h2 className="mb-3 text-sm font-semibold">{creating ? "New agent" : `Edit: ${selected}`}</h2>
            {draft.prompt_sealed ? (
              <>
                <p className="mb-4 rounded border border-border bg-card px-3 py-2 text-xs text-muted-foreground">
                  Built-in agent — configuration is sealed and read-only.
                </p>
                <Field label="System prompt">
                  <pre className="whitespace-pre-wrap rounded border border-border bg-background px-3 py-2 text-xs text-foreground">
                    {draft.system_prompt}
                  </pre>
                </Field>
                <div className="flex items-center gap-2">
                  <Button
                    type="button"
                    variant="outline"
                    size="sm"
                    onClick={() => {
                      setDraft(null);
                      setCreating(false);
                      setSelected(null);
                      setError(null);
                    }}
                  >
                    Close
                  </Button>
                </div>
              </>
            ) : (
              <>
                <Field label="Name (file basename)">
                  <Input
                    className="h-7 text-xs"
                    value={draft.name}
                    disabled={!creating}
                    onChange={(e) => setDraft({ ...draft, name: e.target.value })}
                    placeholder="my_worker"
                  />
                  {!creating && (
                    <p className="mt-1 text-xs text-muted-foreground">Rename by deleting and recreating.</p>
                  )}
                </Field>
                <Field label="Model">
                  <ModelGroupCombobox
                    groups={modelGroups}
                    value={
                      draft.agent_provider?.id
                        ? { groupId: `ext:${draft.agent_provider.id}`, model: draft.agent_provider.model }
                        : draft.llm
                          ? { groupId: "", model: draft.llm }
                          : null
                    }
                    triggerLabel={(() => {
                      if (draft.agent_provider?.id) {
                        const label =
                          modelGroups.find((g) => g.agent_id === draft.agent_provider?.id)?.label ??
                          draft.agent_provider.id;
                        return draft.agent_provider.model
                          ? `${label}: ${draft.agent_provider.model}`
                          : `${label} (default)`;
                      }
                      return draft.llm || "provider default";
                    })()}
                    onPick={(g, m) => {
                      if (!g) {
                        setDraft({ ...draft, llm: "", agent_provider: null });
                      } else if (g.kind === "extension" && g.agent_id) {
                        setDraft({ ...draft, agent_provider: { id: g.agent_id, model: m }, llm: "" });
                      } else {
                        setDraft({ ...draft, llm: m, agent_provider: null });
                      }
                    }}
                  />
                </Field>
                <Field label="Memory namespace (optional)">
                  <Input
                    className="h-7 text-xs"
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
                  <Button type="button" size="sm" onClick={() => void onSave()} disabled={busy}>
                    {creating ? "Create" : "Save"}
                  </Button>
                  {!creating && (
                    <Button
                      type="button"
                      size="sm"
                      variant="destructive"
                      onClick={() => void onDelete()}
                      disabled={busy}
                    >
                      Delete
                    </Button>
                  )}
                  <Button
                    type="button"
                    size="sm"
                    variant="outline"
                    onClick={() => {
                      setDraft(null);
                      setCreating(false);
                      setSelected(null);
                      setError(null);
                    }}
                  >
                    Cancel
                  </Button>
                </div>
              </>
            )}
          </div>
        )}
      </section>
    </div>
  );
}
