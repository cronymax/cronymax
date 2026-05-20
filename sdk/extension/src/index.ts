// SPDX-License-Identifier: Apache-2.0
//
// Cronymax Extension Platform · IDL v1 · entry point
//
// This file describes the surface area of the `cronymax` namespace as seen by
// extensions (the public side of `@cronymax/extension`). It is also the unit
// the Rust → TS codegen targets and the unit the freeze policy applies to:
// **the shape of `Cronymax` below MUST NOT change in v1 except by additive
// growth** (new optional fields, new namespaces).

import type { AgentsNamespace } from "./agents";
import type { Auth } from "./auth";
import type { Commands } from "./commands";
import type { Events } from "./events";
import type { ExtensionsNamespace } from "./extensions";
import type { ExtensionContext } from "./lifecycle";
import type { Renderers } from "./renderers";
import type { Secrets } from "./secrets";
import type { Window } from "./window";
import type { Workspace } from "./workspace";

/**
 * The shape of the `cronymax` value extensions import:
 *
 *   import * as cronymax from "@cronymax/extension";
 *   export async function activate(ctx: cronymax.ExtensionContext) { ... }
 */
export interface Cronymax {
  /** App-level info. */
  readonly env: EnvNamespace;

  /** L1 — kernel primitives. */
  readonly commands: Commands;
  readonly events: Events;
  readonly workspace: Workspace;
  readonly window: Window;
  readonly secrets: Secrets;
  readonly auth: Auth;
  readonly extensions: ExtensionsNamespace;

  /** L2 — extension-point client APIs. */
  readonly agents: AgentsNamespace;
  readonly renderers: Renderers;
}

export interface EnvNamespace {
  readonly appName: string;
  /** Cronymax host platform: `darwin` | `linux` | `win32`. */
  readonly platform: NodeJS.Platform;
  /** Per-install opaque id. Stable across upgrades. */
  readonly machineId: string;
  /** Resolved user home dir. Available even if `fs` capability is denied. */
  readonly homedir: string;
}

/** Convenience: type signature of the `activate` named export. */
export type ActivateExport = (ctx: ExtensionContext) => void | Promise<void>;
/** Convenience: type signature of the optional `deactivate` named export. */
export type DeactivateExport = () => void | Promise<void>;

// ─── re-exports — the entire v1 surface in one place ───────────────────────

export type {
  AgentEvent,
  AgentEventDone,
  AgentEventPermissionRequest,
  AgentEventText,
  AgentEventThinking,
  AgentEventToolCall,
  AgentEventToolCallUpdate,
  AgentProvider,
  AgentSession,
  AgentsNamespace,
  McpServerSpec,
  ModeInfo,
  ModelInfo,
  PermissionDecision,
  PromptAttachment,
  PromptMessage,
  SessionOptions,
} from "./agents";
export type { Auth, AuthSession, GetSessionOptions } from "./auth";

export type { CommandHandler, Commands } from "./commands";

export type {
  EventHandler,
  Events,
  MessageAssistantDeltaPayload,
  MessageAssistantDonePayload,
  MessageUserSentPayload,
  PermissionRequestedPayload,
  PlatformTopicPayloads,
  SessionEndedPayload,
  SessionStartedPayload,
  ToolCompletedPayload,
  ToolInvokedPayload,
} from "./events";
export { PlatformTopic } from "./events";
export type { Extension, ExtensionsNamespace } from "./extensions";
export type {
  ActivateFn,
  DeactivateFn,
  ExtensionContext,
  ExtensionMode,
} from "./lifecycle";
export type {
  CreateOutputChannelOptions,
  LogLevel,
  LogOutputChannel,
  OutputChannel,
} from "./logging";
export type {
  AgentProviderContribution,
  Capabilities,
  CommandContribution,
  ConfigPageContribution,
  ConfigSchemaContribution,
  ContentRendererContribution,
  Contributes,
  ExtensionPointId,
  FsCapability,
  JsonSchema,
  Manifest,
  NetworkCapability,
  SidebarViewContribution,
  UiSlot,
} from "./manifest";
// type-only (isolatedModules requires `export type`)
export type {
  // primitives
  CancellationError,
  CancellationToken,
  CancellationTokenSource,
  Event,
  Listener,
  Thenable,
  URI,
} from "./primitives";
// values (Disposable is both type & namespace; PlatformTopic is a const)
export { Disposable } from "./primitives";
export type {
  Renderers,
  RenderHandle,
  RenderHandler,
  RenderRequest,
} from "./renderers";
export type { Secrets, SecretsError, SecretsErrorCode } from "./secrets";
export type {
  InputBoxOptions,
  MessageItem,
  MessageOptions,
  QuickPickItem,
  QuickPickOptions,
  WebviewPanel,
  WebviewPanelOptions,
  WebviewSlot,
  Window,
} from "./window";
export type {
  Configuration,
  ConfigurationChangeEvent,
  FileStat,
  Workspace,
  WorkspaceFileSystem,
  WorkspaceFolder,
  WorkspaceFsError,
  WorkspaceFsErrorCode,
} from "./workspace";

// ─── runtime values ────────────────────────────────────────────────────────
// The extension host installs the live runtime onto `globalThis.cronymax`
// (see `bundled/extension-host-bootstrap.js`). `runtime.ts` reads that
// global at import time; re-exporting it here lets user code write
// `import { window, commands } from "@cronymax/extension"` and have those
// names resolve to the host-injected values.

export {
  agents,
  auth,
  commands,
  cronymax,
  env,
  events,
  extensions,
  renderers,
  secrets,
  window,
  workspace,
} from "./runtime";
