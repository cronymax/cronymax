// Shared model-group catalog for the provider/model picker.
//
// One canonical source for "what models can a run/agent use", grouped by
// provider: every configured LLM provider (its fetched models) plus every
// active extension AgentProvider (its enumerated models). Both the chat panel's
// top model picker and the Agents tab editor build their picker from this, so
// the two surfaces stay identical (see `ModelGroupCombobox`).

import { shells } from "../shells/bridge";
import { listProviderModels, type ProviderKind } from "../shells/llm";
import { ContributionKind, contributionRegistry } from "../shells/runtime";

/** One provider's worth of selectable models. */
export interface ModelGroup {
  /** Heading shown in the picker (provider name). */
  label: string;
  /** Group id — an LLM provider id, or `ext:<providerId>` for extensions. */
  id: string;
  /** Provider kind, or `"extension"` for an extension AgentProvider group. */
  kind: string;
  base_url: string;
  api_key: string;
  models: string[];
  /** Extension provider id (set only for `kind === "extension"` groups). */
  agent_id?: string;
  /** Contribution kind backing the group (extension groups only). */
  contribution_kind?: string;
}

export interface LlmGroupsResult {
  groups: ModelGroup[];
  /** Active LLM provider (the default when no model is explicitly chosen). */
  activeProviderId: string;
  activeProviderKind: string;
}

interface StoredProvider {
  id: string;
  name: string;
  kind: ProviderKind;
  base_url: string;
  api_key: string;
  default_model: string;
}

/**
 * LLM provider groups (one per configured provider with reachable models).
 * Slow (one HTTP model-list fetch per provider) — call sparingly. Also returns
 * the active provider id/kind for default-model resolution.
 */
export async function fetchLlmGroups(): Promise<LlmGroupsResult> {
  const groups: ModelGroup[] = [];
  let activeProviderId = "";
  let activeProviderKind = "";
  try {
    const { raw, active_id } = await shells.browser.llm.providers.get();
    if (raw) {
      const providers = JSON.parse(raw) as StoredProvider[];
      const active = providers.find((p) => p.id === active_id);
      if (active) {
        activeProviderId = active.id;
        activeProviderKind = active.kind;
      }
      for (const p of providers) {
        if (!p.base_url) continue;
        let models: string[] = [];
        try {
          models = await listProviderModels(p);
        } catch {
          /* keep empty; fall through to default_model */
        }
        if (models.length === 0 && p.default_model) models = [p.default_model];
        if (models.length > 0) {
          groups.push({
            label: p.name || p.kind,
            id: p.id,
            kind: p.kind,
            base_url: p.base_url,
            api_key: p.api_key,
            models,
          });
        }
      }
    }
  } catch {
    /* no LLM providers configured */
  }
  return { groups, activeProviderId, activeProviderKind };
}

/**
 * Extension AgentProvider groups (one per active provider with enumerated
 * models). Fast (local IPC) — safe to refresh when the activation catalog
 * changes.
 */
export async function fetchExtensionGroups(): Promise<ModelGroup[]> {
  const groups: ModelGroup[] = [];
  try {
    const res = await contributionRegistry.list();
    const provs = (res.contributions ?? []).filter((d) => d.kind === ContributionKind.AgentsProvider);
    for (const p of provs) {
      try {
        const { items } = await contributionRegistry.enumerate(ContributionKind.AgentsProvider, p.id);
        if (!items.length) continue;
        groups.push({
          label: p.label || p.id,
          id: `ext:${p.id}`,
          kind: "extension",
          base_url: "",
          api_key: "",
          models: items.map((it) => it.id),
          agent_id: p.id,
          contribution_kind: ContributionKind.AgentsProvider,
        });
      } catch {
        /* provider failed to enumerate — skip it */
      }
    }
  } catch {
    /* no extension runtime */
  }
  return groups;
}

/**
 * Full grouped catalog: LLM provider groups then extension groups. One-shot
 * convenience for surfaces (e.g. the Agents editor) that don't need chat's
 * separate slow/fast refresh cadences.
 */
export async function fetchModelGroups(): Promise<ModelGroup[]> {
  const [llm, ext] = await Promise.all([fetchLlmGroups(), fetchExtensionGroups()]);
  return [...llm.groups, ...ext];
}
