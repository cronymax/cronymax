// SPDX-License-Identifier: Apache-2.0
//
// Cronymax Extension Platform · IDL v1 · contributions
//
// The shared wire shape for "user-selectable things" in the picker:
// extension agent providers, workspace YAML agents, the Crony built-in
// chat agent, and (later) LLM providers and other contribution kinds.
//
// Mirror of `crates/cronymax/src/extensions/contributions/mod.rs`. The
// JSON shape MUST match field-for-field — codegen runs against this file.

/**
 * Where a contribution came from. Determines who can edit / delete it
 * and how the picker groups it visually.
 */
export type ContributionOwner = { type: "platform" } | { type: "workspace" } | { type: "extension"; extId: string };

/**
 * One top-level row in the picker. `metadata` is kind-specific; consumers
 * downcast by `kind` (see the `ContributionKind` string-union below).
 */
export interface ContributionDescriptor {
  kind: string;
  id: string;
  owner: ContributionOwner;
  label: string;
  description?: string;
  icon?: string;
  metadata?: unknown;
}

/**
 * One selectable child of a descriptor — e.g. a model under an extension
 * agent provider, or a variant of a workspace agent. Returned by
 * `AgentProvider.enumerate()` and by the runtime's synthesized lists for
 * platform / workspace contributions.
 */
export interface ContributionItem {
  id: string;
  label: string;
  description?: string;
  icon?: string;
  metadata?: unknown;
}

/**
 * Known contribution kinds. Strings match the `kind::*` constants in the
 * Rust runtime and the `contributes.*` keys in `manifest.ts`.
 */
export const ContributionKind = {
  Command: "cronymax.command",
  ConfigSchema: "cronymax.config.schema",
  ConfigPage: "cronymax.config.page",
  AgentProvider: "cronymax.agents.provider",
  ContentRenderer: "cronymax.content.renderer",
  SidebarView: "cronymax.ui.sidebar.view",
  AgentBuiltin: "cronymax.agents.builtin",
  AgentWorkspace: "cronymax.agents.workspace",
} as const;
export type ContributionKindId = (typeof ContributionKind)[keyof typeof ContributionKind];
